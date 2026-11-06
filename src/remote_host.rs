use crate::{
    config::{
        paths::AppConfigPaths,
        ssh::{SshAuthPreference, SshConnectionConfig},
    },
    host_runtime::DesktopHostRuntime,
    host_storage::HostStorage,
    remote_launch::{RemoteLaunch, RemoteTarget},
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use yttt_client_core::ClientCore;
use yttt_core::model::{
    ids::{ClientInstanceId, ProfileId},
    project::{RemotePathBuf, RemoteRelativePathBuf},
};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, ProtocolRange, Request, Response,
    workspace::{WorkspaceRequest, WorkspaceResponse},
};
use yttt_ssh::{
    Authentication, ConnectRequest, SshEndpoint, StoredCredential, TransportEvent, TransportService,
};
use yttt_transport::{AuthToken, ClientIdentity, Credential, SharedConnector, TransportConnector};
use zeroize::{Zeroize, Zeroizing};

pub enum RemoteConnectEvent {
    Status(RemoteConnectStatus),
    HostKey(yttt_ssh::HostKeyChallenge),
}

pub enum RemoteConnectStatus {
    VerifyingHost,
    CheckingServer,
    StartingHost,
    Ssh {
        state: yttt_ssh::ConnectionState,
        error: Option<String>,
    },
}

const CONNECT_CANCELLED: &str = "remote connection cancelled";

#[derive(Clone)]
pub struct RemoteConnectCancellation {
    sender: tokio::sync::watch::Sender<bool>,
}

impl Default for RemoteConnectCancellation {
    fn default() -> Self {
        Self::new()
    }
}

impl RemoteConnectCancellation {
    pub fn new() -> Self {
        let (sender, _) = tokio::sync::watch::channel(false);
        Self { sender }
    }

    pub fn cancel(&self) {
        self.sender.send_replace(true);
    }

    fn is_cancelled(&self) -> bool {
        *self.sender.borrow()
    }

    fn check(&self, events: &flume::Sender<RemoteConnectEvent>) -> Result<(), String> {
        if self.is_cancelled() || events.is_disconnected() {
            self.cancel();
            return Err(CONNECT_CANCELLED.to_string());
        }
        Ok(())
    }

    async fn cancelled(&self, events: &flume::Sender<RemoteConnectEvent>) {
        let mut cancelled = self.sender.subscribe();
        loop {
            if self.is_cancelled() || events.is_disconnected() {
                self.cancel();
                return;
            }
            tokio::select! {
                result = cancelled.changed() => {
                    if result.is_err() || *cancelled.borrow() {
                        return;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(25)) => {}
            }
        }
    }
}

pub struct RemoteEnvironment {
    pub runtime: Arc<DesktopHostRuntime>,
    pub config_paths: AppConfigPaths,
    pub label: String,
    pub home: PathBuf,
}
#[derive(Deserialize)]
struct ServerDescriptor {
    profile_id: String,
    endpoint: String,
    auth_token_hex: String,
    resource_protocol: u16,
    lifecycle_protocol: u16,
}
impl Drop for ServerDescriptor {
    fn drop(&mut self) {
        self.auth_token_hex.zeroize();
    }
}

struct ConnectionCredentials {
    connection: SshConnectionConfig,
    password: Option<Zeroizing<String>>,
    passphrase: Option<Zeroizing<String>>,
    save_password_as: Option<yttt_core::model::ids::CredentialId>,
}
impl ConnectionCredentials {
    fn request(&self, reconnect: bool) -> ConnectRequest {
        let connection = &self.connection;
        let credential = connection
            .credential
            .as_ref()
            .map(|credential| StoredCredential {
                id: credential.id.clone(),
                effective_user: credential.binding.effective_user.clone(),
                resolved_host: credential.binding.resolved_host.clone(),
                port: credential.binding.port,
                host_key_sha256: credential.binding.host_key_sha256.clone(),
                private_key_identity: credential.binding.private_key_identity.clone(),
            });
        let authentication = match connection.auth {
            SshAuthPreference::Agent => Authentication::Agent,
            SshAuthPreference::Password | SshAuthPreference::Auto if self.password.is_some() => {
                Authentication::Password {
                    secret: self.password.clone().unwrap(),
                    save_as: self.save_password_as.clone(),
                }
            }
            SshAuthPreference::Password if credential.is_some() => {
                Authentication::StoredPassword(credential.unwrap())
            }
            SshAuthPreference::PublicKey if connection.identity_file.is_some() => {
                Authentication::PrivateKey {
                    path: connection.identity_file.clone().unwrap(),
                    passphrase: self.passphrase.clone(),
                }
            }
            _ => Authentication::Auto {
                identity_file: connection.identity_file.clone(),
                passphrase: self.passphrase.clone(),
                credential,
            },
        };
        ConnectRequest {
            connection_id: connection.id.clone(),
            endpoint: SshEndpoint {
                host: connection.host.clone(),
                port: connection.port,
                user: connection.user.clone(),
            },
            authentication,
            reconnect,
        }
    }
}

fn ssh_transport_error(error: yttt_ssh::TransportError) -> yttt_transport::TransportError {
    let message = error.to_string();
    if error.is_permanent() {
        yttt_transport::TransportError::Permanent(message)
    } else {
        yttt_transport::TransportError::Other(message)
    }
}

fn send_status(
    events: &flume::Sender<RemoteConnectEvent>,
    cancellation: &RemoteConnectCancellation,
    status: RemoteConnectStatus,
) -> Result<(), String> {
    cancellation.check(events)?;
    events
        .send(RemoteConnectEvent::Status(status))
        .map_err(|_| {
            cancellation.cancel();
            CONNECT_CANCELLED.to_string()
        })
}

/// Why a tunnel to the remote Host could not be opened.
enum TunnelFailure {
    /// The SSH session works but nothing accepts on the Host socket.
    Refused(String),
    /// The SSH session is gone or stalled.
    Session(String),
}

/// The steps a tunnel to the remote Host can take to recover.
trait TunnelLink: Send + Sync + 'static {
    fn open(
        &self,
    ) -> yttt_transport::BoxFuture<'_, Result<yttt_transport::TransportStream, TunnelFailure>>;
    /// Replaces the SSH session.
    fn rebuild(&self) -> yttt_transport::BoxFuture<'_, Result<(), yttt_transport::TransportError>>;
    /// Starts the remote Server if it is not running.
    fn ensure_server(
        &self,
    ) -> yttt_transport::BoxFuture<'_, Result<(), yttt_transport::TransportError>>;
}

/// Reconnects a tunnel with as few steps as its failure needs: a stalled
/// session is rebuilt at once instead of being waited on again, and the
/// remote Server is started only when the session works but nothing listens.
struct TunnelConnector<L> {
    link: L,
    gate: tokio::sync::Mutex<()>,
    /// Counts rebuilt sessions, so a caller that waited on the gate reuses one.
    rebuilds: std::sync::atomic::AtomicU64,
}

impl<L: TunnelLink> TunnelConnector<L> {
    fn new(link: L) -> Self {
        Self {
            link,
            gate: tokio::sync::Mutex::new(()),
            rebuilds: std::sync::atomic::AtomicU64::new(0),
        }
    }

    async fn connect_tunnel(
        &self,
    ) -> Result<yttt_transport::TransportStream, yttt_transport::TransportError> {
        use std::sync::atomic::Ordering;
        let rebuilds = self.rebuilds.load(Ordering::Acquire);
        let failure = match self.link.open().await {
            Ok(stream) => return Ok(stream),
            Err(failure) => failure,
        };
        let _guard = self.gate.lock().await;
        if let TunnelFailure::Refused(_) = failure {
            return self.start_server_and_open().await;
        }
        if self.rebuilds.load(Ordering::Acquire) != rebuilds {
            match self.link.open().await {
                Ok(stream) => return Ok(stream),
                Err(TunnelFailure::Refused(_)) => return self.start_server_and_open().await,
                Err(TunnelFailure::Session(_)) => {}
            }
        }
        self.link.rebuild().await?;
        self.rebuilds.fetch_add(1, Ordering::AcqRel);
        match self.link.open().await {
            Ok(stream) => Ok(stream),
            Err(TunnelFailure::Refused(_)) => self.start_server_and_open().await,
            Err(TunnelFailure::Session(message)) => {
                Err(yttt_transport::TransportError::Other(message))
            }
        }
    }

    async fn start_server_and_open(
        &self,
    ) -> Result<yttt_transport::TransportStream, yttt_transport::TransportError> {
        self.link.ensure_server().await?;
        self.link.open().await.map_err(|failure| match failure {
            TunnelFailure::Refused(message) | TunnelFailure::Session(message) => {
                yttt_transport::TransportError::Other(message)
            }
        })
    }
}

impl<L: TunnelLink> TransportConnector for TunnelConnector<L> {
    fn connect(
        &self,
    ) -> yttt_transport::BoxFuture<
        '_,
        Result<yttt_transport::TransportStream, yttt_transport::TransportError>,
    > {
        Box::pin(self.connect_tunnel())
    }

    fn link_profile(&self) -> yttt_transport::LinkProfile {
        yttt_transport::LinkProfile::REMOTE
    }
}

/// A tunnel over this app's SSH connection to the remote Server's socket.
struct SshTunnel {
    service: TransportService,
    credentials: Arc<ConnectionCredentials>,
    socket: String,
    server: String,
}

impl TunnelLink for SshTunnel {
    fn open(
        &self,
    ) -> yttt_transport::BoxFuture<'_, Result<yttt_transport::TransportStream, TunnelFailure>> {
        Box::pin(async move {
            self.service
                .open_streamlocal(self.credentials.connection.id.clone(), self.socket.clone())
                .await
                .map_err(|error| match error {
                    yttt_ssh::TransportError::ChannelRefused(_) => {
                        TunnelFailure::Refused(error.to_string())
                    }
                    error => TunnelFailure::Session(error.to_string()),
                })
        })
    }

    fn rebuild(&self) -> yttt_transport::BoxFuture<'_, Result<(), yttt_transport::TransportError>> {
        Box::pin(async move {
            self.service
                .connect(self.credentials.request(true))
                .await
                .map(|_| ())
                .map_err(ssh_transport_error)
        })
    }

    fn ensure_server(
        &self,
    ) -> yttt_transport::BoxFuture<'_, Result<(), yttt_transport::TransportError>> {
        Box::pin(async move {
            let service = self.service.clone();
            let id = self.credentials.connection.id.clone();
            let server = self.server.clone();
            tokio::task::spawn_blocking(move || ensure_server(&service, id, &server, None))
                .await
                .map_err(|error| yttt_transport::TransportError::Other(error.to_string()))?
                .map(|_| ())
                .map_err(yttt_transport::TransportError::Other)
        })
    }
}

pub fn connect(
    launch: RemoteLaunch,
    events: flume::Sender<RemoteConnectEvent>,
    cancellation: RemoteConnectCancellation,
) -> Result<RemoteEnvironment, String> {
    cancellation.check(&events)?;
    let label = launch.target.label();
    match &launch.target {
        RemoteTarget::SshServer { .. } => connect_ssh(launch, events, cancellation, label),
        RemoteTarget::PairedHost { host, .. } => {
            let host = host.clone();
            connect_paired(launch, host, events, cancellation, label)
        }
        RemoteTarget::ExistingHost {
            address,
            connection_info,
        } => {
            use yttt_protocol::remote_access::TLS_SERVER_NAME;
            if connection_info.server_name != TLS_SERVER_NAME
                || connection_info.certificate_sha256
                    != format!("{:x}", Sha256::digest(&connection_info.certificate_der))
                || connection_info.environment_id.is_empty()
            {
                return Err("Imported Host identity or certificate fingerprint is invalid".into());
            }
            send_status(&events, &cancellation, RemoteConnectStatus::VerifyingHost)?;
            let connector = SharedConnector::new(
                yttt_transport_tls::TlsConnector::new(
                    address.clone(),
                    connection_info.certificate_der.clone(),
                )
                .map_err(|error| error.to_string())?,
            );
            let runtime = tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .enable_all()
                .thread_name("yttt-existing-host-client")
                .build()
                .map_err(|error| error.to_string())?;
            let identity = ClientIdentity {
                expected_environment: Some(connection_info.environment_id.clone()),
                credential_generation: connection_info.credential_generation,
                session_nonce: yttt_transport::new_session_nonce(),
                supported: ProtocolRange::exact(yttt_protocol::RESOURCE_PROTOCOL_VERSION),
                build: BuildIdentity {
                    product_version: env!("CARGO_PKG_VERSION").into(),
                    build_fingerprint: option_env!("YTTT_BUILD_FINGERPRINT")
                        .unwrap_or(env!("CARGO_PKG_VERSION"))
                        .into(),
                    resource_compatibility: "yttt-resource-v1".into(),
                },
                profile_id: connection_info.profile_id.clone(),
                client_instance_id: ClientInstanceId::new(format!(
                    "existing-host-client-{}",
                    uuid::Uuid::new_v4()
                )),
                host_epoch_hint: None,
                can_force_stop: false,
                channel: ConnectionChannel::Session,
            };
            cancellation.check(&events)?;
            initialize_environment(
                runtime,
                connector,
                identity,
                AuthToken::from_bytes(connection_info.work_secret).into(),
                events,
                cancellation,
                label,
                &launch.local_profile.paths().logs,
            )
        }
    }
}

/// How often the remote Client checks whether a P2P session runs direct or
/// through the relay; hole punching usually upgrades a session after it starts.
const PATH_CHECK_INTERVAL: Duration = Duration::from_secs(2);

fn connect_paired(
    launch: RemoteLaunch,
    host: yttt_client_core::p2p::PairedHost,
    events: flume::Sender<RemoteConnectEvent>,
    cancellation: RemoteConnectCancellation,
    label: String,
) -> Result<RemoteEnvironment, String> {
    use yttt_transport_iroh::{
        DirectPaths, EndpointConfig, IrohEndpoint, OpenGate, RelayTarget,
    };
    // Read here rather than carried in the launch, so no long-lived key passes
    // between processes.
    let secret = crate::paired_hosts::load_device_key(
        &crate::paired_hosts::device_key_store(&launch.local_profile),
        host.host,
    )?;
    send_status(&events, &cancellation, RemoteConnectStatus::VerifyingHost)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("yttt-paired-host-client")
        .build()
        .map_err(|error| error.to_string())?;
    let endpoint = runtime
        .block_on(IrohEndpoint::bind(EndpointConfig {
            secret: *secret,
            relay: host
                .relay_url
                .clone()
                .map(|url| RelayTarget { url, token: None })
                .into(),
            alpns: Vec::new(),
            gate: Arc::new(OpenGate),
            direct: DirectPaths::Any,
        }))
        .map_err(|error| format!("cannot open the P2P endpoint: {error}"))?;
    let connector = Arc::new(host.connector(endpoint));
    watch_path(
        &runtime,
        connector.clone(),
        launch.local_profile.paths().logs.clone(),
        label.clone(),
    );
    let identity = ClientIdentity {
        expected_environment: Some(host.environment_id.clone()),
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(yttt_protocol::RESOURCE_PROTOCOL_VERSION),
        build: BuildIdentity {
            product_version: env!("CARGO_PKG_VERSION").into(),
            build_fingerprint: option_env!("YTTT_BUILD_FINGERPRINT")
                .unwrap_or(env!("CARGO_PKG_VERSION"))
                .into(),
            resource_compatibility: "yttt-resource-v1".into(),
        },
        profile_id: host.profile_id.clone(),
        client_instance_id: ClientInstanceId::new(format!(
            "paired-host-client-{}",
            uuid::Uuid::new_v4()
        )),
        host_epoch_hint: None,
        can_force_stop: false,
        channel: ConnectionChannel::Session,
    };
    cancellation.check(&events)?;
    initialize_environment(
        runtime,
        SharedConnector::new(connector),
        identity,
        host.credential(*secret),
        events,
        cancellation,
        label,
        &launch.local_profile.paths().logs,
    )
}

/// Logs each change between a direct and a relayed path for the session.
fn watch_path(
    runtime: &tokio::runtime::Runtime,
    connector: Arc<yttt_transport_iroh::IrohConnector>,
    logs: PathBuf,
    label: String,
) {
    runtime.spawn(async move {
        let mut reported = yttt_transport_iroh::PathKind::Unknown;
        loop {
            tokio::time::sleep(PATH_CHECK_INTERVAL).await;
            let path = connector.path_kind().await;
            if path != reported {
                reported = path;
                let line = match path {
                    yttt_transport_iroh::PathKind::Direct => "P2P path: direct",
                    yttt_transport_iroh::PathKind::Relay => "P2P path: relay",
                    yttt_transport_iroh::PathKind::Unknown => continue,
                };
                crate::connection_log::note(&logs, &label, line).await;
            }
        }
    });
}

fn connect_ssh(
    mut launch: RemoteLaunch,
    events: flume::Sender<RemoteConnectEvent>,
    cancellation: RemoteConnectCancellation,
    label: String,
) -> Result<RemoteEnvironment, String> {
    let RemoteTarget::SshServer {
        connection,
        password,
        passphrase,
        save_password_as,
    } = &mut launch.target
    else {
        unreachable!("SSH target selected")
    };
    let credentials = Arc::new(ConnectionCredentials {
        connection: connection.clone(),
        password: password.take().map(Zeroizing::new),
        passphrase: passphrase.take().map(Zeroizing::new),
        save_password_as: save_password_as.clone(),
    });
    cancellation.check(&events)?;
    let config = launch.local_profile.config_paths();
    let service = TransportService::start_with_credential_namespace(
        config.ssh_host_keys_file(),
        launch.local_profile.credential_namespace(),
    )
    .map_err(|error| error.to_string())?;
    let ssh_events = service.events();
    let ui_events = events.clone();
    let saved_connection = credentials.connection.clone();
    let event_cancellation = cancellation.clone();
    std::thread::Builder::new()
        .name("yttt-remote-ssh-events".to_string())
        .spawn(move || {
            while let Ok(event) = ssh_events.recv_blocking() {
                match event {
                    TransportEvent::HostKeyChallenge(challenge) => {
                        if ui_events
                            .send(RemoteConnectEvent::HostKey(challenge))
                            .is_err()
                        {
                            event_cancellation.cancel();
                            break;
                        }
                    }
                    TransportEvent::StateChanged(status) => {
                        if ui_events
                            .send(RemoteConnectEvent::Status(RemoteConnectStatus::Ssh {
                                state: status.state,
                                error: status.error,
                            }))
                            .is_err()
                        {
                            event_cancellation.cancel();
                            break;
                        }
                    }
                    TransportEvent::CredentialSaved { credential, .. } => {
                        if event_cancellation.is_cancelled() || ui_events.is_disconnected() {
                            event_cancellation.cancel();
                            break;
                        }
                        if let Ok(mut connections) =
                            crate::config::ssh::load_ssh_connections(&config)
                            && let Some(connection) = connections
                                .connections
                                .iter_mut()
                                .find(|connection| connection.id == saved_connection.id)
                        {
                            connection.credential = Some(crate::config::ssh::CredentialRef {
                                id: credential.id,
                                kind: crate::config::ssh::CredentialKind::LoginPassword,
                                binding: crate::config::ssh::CredentialBinding {
                                    connection_id: connection.id.clone(),
                                    effective_user: credential.effective_user,
                                    resolved_host: credential.resolved_host,
                                    port: credential.port,
                                    host_key_sha256: credential.host_key_sha256,
                                    private_key_identity: credential.private_key_identity,
                                },
                            });
                            let _ = crate::config::ssh::save_ssh_connections(&config, &connections);
                        }
                    }
                }
            }
        })
        .map_err(|error| error.to_string())?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .thread_name("yttt-remote-client")
        .build()
        .map_err(|error| error.to_string())?;
    cancellation.check(&events)?;
    runtime
        .block_on(async {
            let mut cancellation_state = cancellation.sender.subscribe();
            let connect =
                service.connect_cancellable(credentials.request(false), &mut cancellation_state);
            tokio::pin!(connect);
            tokio::select! {
                result = &mut connect => result,
                _ = cancellation.cancelled(&events) => connect.await,
            }
        })
        .map_err(|error| error.to_string())?;
    send_status(&events, &cancellation, RemoteConnectStatus::CheckingServer)?;
    let deployment_service = service.clone();
    let deployment_connection = credentials.connection.clone();
    let deployment_cancellation = cancellation.clone();
    let deployment_events = events.clone();
    let server = match run_cancellable_blocking(&runtime, &cancellation, &events, move || {
        deploy_server(
            &deployment_service,
            &deployment_connection,
            &deployment_cancellation,
            &deployment_events,
        )
    }) {
        Ok(server) => server,
        Err(error) => {
            if cancellation.is_cancelled() {
                let _ = runtime.block_on(service.disconnect(credentials.connection.id.clone()));
            }
            return Err(error);
        }
    };
    send_status(&events, &cancellation, RemoteConnectStatus::StartingHost)?;
    let ensure_service = service.clone();
    let ensure_id = credentials.connection.id.clone();
    let ensure_cancellation = cancellation.clone();
    let ensure_server_path = server.clone();
    let ensure_events = events.clone();
    let descriptor = match run_cancellable_blocking(&runtime, &cancellation, &events, move || {
        ensure_server(
            &ensure_service,
            ensure_id,
            &ensure_server_path,
            Some((&ensure_cancellation, &ensure_events)),
        )
    }) {
        Ok(descriptor) => descriptor,
        Err(error) => {
            if cancellation.is_cancelled() {
                let _ = runtime.block_on(service.disconnect(credentials.connection.id.clone()));
            }
            return Err(error);
        }
    };
    if descriptor.resource_protocol != yttt_protocol::RESOURCE_PROTOCOL_VERSION
        || descriptor.lifecycle_protocol != yttt_protocol::LIFECYCLE_PROTOCOL_VERSION
    {
        return Err(
            "Remote Host protocol is incompatible; its running resources were not stopped"
                .to_string(),
        );
    }
    cancellation.check(&events)?;
    let connector = SharedConnector::new(TunnelConnector::new(SshTunnel {
        service,
        credentials: credentials.clone(),
        socket: descriptor.endpoint.clone(),
        server,
    }));
    let token = decode_token(&descriptor.auth_token_hex)?;
    let identity = ClientIdentity {
        expected_environment: None,
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(yttt_protocol::RESOURCE_PROTOCOL_VERSION),
        build: BuildIdentity {
            product_version: env!("CARGO_PKG_VERSION").to_string(),
            build_fingerprint: option_env!("YTTT_BUILD_FINGERPRINT")
                .unwrap_or(env!("CARGO_PKG_VERSION"))
                .to_string(),
            resource_compatibility: "yttt-resource-v1".to_string(),
        },
        profile_id: ProfileId::new(descriptor.profile_id.clone()),
        client_instance_id: ClientInstanceId::new(format!(
            "remote-client-{}",
            uuid::Uuid::new_v4()
        )),
        host_epoch_hint: None,
        can_force_stop: false,
        channel: ConnectionChannel::Session,
    };
    initialize_environment(
        runtime,
        connector,
        identity,
        token.into(),
        events,
        cancellation,
        label,
        &launch.local_profile.paths().logs,
    )
}

fn run_cancellable_blocking<T: Send + 'static>(
    runtime: &tokio::runtime::Runtime,
    cancellation: &RemoteConnectCancellation,
    events: &flume::Sender<RemoteConnectEvent>,
    operation: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    cancellation.check(events)?;
    // A cancelled blocking HTTP read may outlive this connection attempt. Do not put it
    // on this Runtime's blocking pool: dropping the Runtime would wait for it anyway.
    let (sender, result) = flume::bounded(1);
    std::thread::Builder::new()
        .name("yttt-remote-deploy".into())
        .spawn(move || {
            let _ = sender.send(operation());
        })
        .map_err(|error| error.to_string())?;
    runtime.block_on(async {
        tokio::select! {
            result = result.recv_async() => result.map_err(|error| error.to_string())?,
            _ = cancellation.cancelled(events) => Err(CONNECT_CANCELLED.to_string()),
        }
    })
}

#[allow(clippy::too_many_arguments)]
fn initialize_environment(
    runtime: tokio::runtime::Runtime,
    connector: SharedConnector,
    identity: ClientIdentity,
    token: Credential,
    events: flume::Sender<RemoteConnectEvent>,
    cancellation: RemoteConnectCancellation,
    label: String,
    logs: &Path,
) -> Result<RemoteEnvironment, String> {
    cancellation.check(&events)?;
    let client = Arc::new(runtime.block_on(async {
        let connect = ClientCore::connect(connector.clone(), identity.clone(), token.clone());
        tokio::pin!(connect);
        tokio::select! {
            result = &mut connect => result.map_err(|error| error.to_string()),
            _ = cancellation.cancelled(&events) => Err(CONNECT_CANCELLED.to_string()),
        }
    })?);
    crate::connection_log::follow(client.clone(), runtime.handle(), logs, label.clone());
    let environment = match runtime.block_on(async {
        cancellation.check(&events)?;
        tokio::select! {
            result = client.request(Request::Workspace(WorkspaceRequest::Environment)) => {
                result.map_err(|error| error.to_string())
            }
            _ = cancellation.cancelled(&events) => Err(CONNECT_CANCELLED.to_string()),
        }
    })? {
        Response::Workspace(WorkspaceResponse::Environment(environment)) => environment,
        _ => return Err("unexpected Host environment response".into()),
    };
    cancellation.check(&events)?;
    let config_root = environment
        .config_root
        .to_client_path()
        .map_err(|error| error.to_string())?;
    let storage = Arc::new(HostStorage::new(
        client.clone(),
        runtime.handle().clone(),
        config_root.clone(),
        environment.clone(),
        true,
    ));
    cancellation.check(&events)?;
    crate::config::storage::bind_environment(storage.clone()).map_err(|error| error.to_string())?;
    let config_paths =
        AppConfigPaths::from_host_config_dir(config_root, environment.project_config.clone())
            .map_err(|error| error.to_string())?;
    let home = environment
        .home
        .to_client_path()
        .map_err(|error| error.to_string())?;
    cancellation.check(&events)?;
    let host = DesktopHostRuntime::from_remote(
        runtime,
        client,
        storage,
        connector,
        identity,
        token,
        environment,
        label.clone(),
    )
    .map_err(|error| error.to_string())?;
    Ok(RemoteEnvironment {
        runtime: host,
        config_paths,
        label,
        home,
    })
}

fn ensure_server(
    service: &TransportService,
    id: yttt_core::model::ids::ConnectionId,
    server: &str,
    cancellation: Option<(
        &RemoteConnectCancellation,
        &flume::Sender<RemoteConnectEvent>,
    )>,
) -> Result<ServerDescriptor, String> {
    let check_cancelled = || match cancellation {
        Some((cancellation, events)) => cancellation.check(events),
        None => Ok(()),
    };
    check_cancelled()?;
    let root = service.sftp_project(
        id,
        RemotePathBuf::new("/").map_err(|error| error.to_string())?,
    );
    check_cancelled()?;
    let mut output = root
        .run_command(
            server,
            vec![
                "ensure".to_string(),
                "--profile".to_string(),
                "default".to_string(),
            ],
        )
        .map_err(|error| error.to_string())?;
    let result = if output.success() {
        serde_json::from_slice(&output.stdout)
            .map_err(|_| "Server returned an invalid descriptor".to_string())
    } else {
        Err(format!(
            "Remote Server could not start: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    };
    output.stdout.zeroize();
    result
}
fn decode_token(hex: &str) -> Result<AuthToken, String> {
    if hex.len() != 64 {
        return Err("invalid Server token".to_string());
    }
    let mut bytes = [0u8; 32];
    for (index, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16)
            .map_err(|_| "invalid Server token".to_string())?;
    }
    let token = AuthToken::from_bytes(bytes);
    bytes.zeroize();
    Ok(token)
}
fn deploy_server(
    service: &TransportService,
    connection: &SshConnectionConfig,
    cancellation: &RemoteConnectCancellation,
    events: &flume::Sender<RemoteConnectEvent>,
) -> Result<String, String> {
    cancellation.check(events)?;
    let root = service.sftp_project(
        connection.id.clone(),
        RemotePathBuf::new("/").map_err(|error| error.to_string())?,
    );
    cancellation.check(events)?;
    let platform = root
        .run_command("uname", vec!["-s".to_string()])
        .map_err(|error| error.to_string())?;
    cancellation.check(events)?;
    let architecture = root
        .run_command("uname", vec!["-m".to_string()])
        .map_err(|error| error.to_string())?;
    if !platform.success() || !architecture.success() {
        return Err("Remote Server requires a POSIX SSH account with uname".to_string());
    }
    let platform = match String::from_utf8_lossy(&platform.stdout).trim() {
        "Linux" => "linux",
        "Darwin" => "macos",
        other => return Err(format!("unsupported remote Server OS: {other}")),
    };
    let architecture = match String::from_utf8_lossy(&architecture.stdout).trim() {
        "arm64" | "aarch64" => "aarch64",
        "x86_64" | "amd64" => "x86_64",
        other => return Err(format!("unsupported remote Server architecture: {other}")),
    };
    let target = format!("{platform}-{architecture}");
    cancellation.check(events)?;
    let home = root
        .run_command("sh", vec!["-c".into(), "cd \"$HOME\" && pwd -P".into()])
        .map_err(|error| error.to_string())?;
    if !home.success() {
        return Err("SSH account did not provide an accessible HOME".into());
    }
    let home = RemotePathBuf::new(
        String::from_utf8(home.stdout)
            .map_err(|error| error.to_string())?
            .trim()
            .to_string(),
    )
    .map_err(|error| error.to_string())?;
    let directory = format!(
        "{}/.local/share/yttt/server/{}/{target}",
        home.as_str().trim_end_matches('/'),
        env!("CARGO_PKG_VERSION")
    );
    let server = format!("{directory}/yttt-server");
    cancellation.check(events)?;
    let present = root
        .run_command("test", vec!["-x".to_string(), server.clone()])
        .map_err(|error| error.to_string())?;
    if present.success() {
        return Ok(server);
    }
    let bytes = server_bytes(&target, cancellation, events)?;
    cancellation.check(events)?;
    let created = root
        .run_command(
            "sh",
            vec![
                "-c".to_string(),
                "umask 077; mkdir -p \"$1\"".to_string(),
                "yttt-deploy".to_string(),
                directory.clone(),
            ],
        )
        .map_err(|error| error.to_string())?;
    if !created.success() {
        return Err(format!(
            "cannot create Server install directory: {}",
            String::from_utf8_lossy(&created.stderr)
        ));
    }
    let upload = format!("{directory}/.yttt-server-{}.tmp", uuid::Uuid::new_v4());
    let relative = RemoteRelativePathBuf::new(upload.trim_start_matches('/'))
        .map_err(|error| error.to_string())?;
    cancellation.check(events)?;
    match root
        .save_file(relative, bytes, None, false, MAX_SERVER_BYTES)
        .map_err(|error| error.to_string())?
    {
        yttt_ssh::RemoteSaveOutcome::Saved(_) => {}
        _ => return Err("Server upload conflicted with another writer".to_string()),
    }
    cancellation.check(events)?;
    let published = root
        .run_command(
            "sh",
            vec![
                "-c".to_string(),
                "chmod 700 \"$1\" && mv -f \"$1\" \"$2\"".to_string(),
                "yttt-deploy".to_string(),
                upload,
                server.clone(),
            ],
        )
        .map_err(|error| error.to_string())?;
    if !published.success() {
        return Err(format!(
            "cannot publish Server binary: {}",
            String::from_utf8_lossy(&published.stderr)
        ));
    }
    Ok(server)
}

const MAX_SERVER_BYTES: u64 = 128 * 1024 * 1024;
const MAX_SERVER_READ_BYTES: usize = 128 * 1024 * 1024 + 1;

fn server_bytes(
    target: &str,
    cancellation: &RemoteConnectCancellation,
    events: &flume::Sender<RemoteConnectEvent>,
) -> Result<Vec<u8>, String> {
    cancellation.check(events)?;
    let filename = format!("yttt-server-{}-{target}", env!("CARGO_PKG_VERSION"));
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let adjacent = executable
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(&filename);
    let native_platform = if cfg!(target_os = "macos") {
        "macos"
    } else {
        std::env::consts::OS
    };
    let native = format!("{native_platform}-{}", std::env::consts::ARCH);
    let bundled = executable
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("yttt-server");
    let local = std::env::var_os("YTTT_SERVER_BINARY")
        .map(PathBuf::from)
        .or_else(|| adjacent.is_file().then_some(adjacent))
        .or_else(|| (native == target && bundled.is_file()).then_some(bundled));
    if let Some(path) = local {
        cancellation.check(events)?;
        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_SERVER_BYTES {
            return Err("Server binary exceeds upload limit".to_string());
        }
        return read_server_bytes(file, cancellation, events);
    }
    let base = format!(
        "https://github.com/yuWorm/yttt/releases/download/v{}",
        env!("CARGO_PKG_VERSION")
    );
    cancellation.check(events)?;
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| error.to_string())?;
    cancellation.check(events)?;
    let sums = http
        .get(format!("{base}/SHA256SUMS"))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("cannot obtain Server checksums: {error}"))?
        .text()
        .map_err(|error| error.to_string())?;
    cancellation.check(events)?;
    let expected = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            (parts.next()? == filename).then_some(hash)
        })
        .ok_or_else(|| format!("release does not contain a checksum for {filename}"))?;
    cancellation.check(events)?;
    let response = http
        .get(format!("{base}/{filename}"))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("cannot download remote Server: {error}"))?;
    let bytes = read_server_bytes(response, cancellation, events)?;
    if format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err("remote Server download failed checksum or size validation".to_string());
    }
    Ok(bytes)
}

fn read_server_bytes(
    mut reader: impl Read,
    cancellation: &RemoteConnectCancellation,
    events: &flume::Sender<RemoteConnectEvent>,
) -> Result<Vec<u8>, String> {
    let mut bytes = Vec::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        cancellation.check(events)?;
        let remaining = MAX_SERVER_READ_BYTES.saturating_sub(bytes.len());
        if remaining == 0 {
            return Err("Server binary exceeds upload limit".to_string());
        }
        let chunk_len = remaining.min(buffer.len());
        let read = reader
            .read(&mut buffer[..chunk_len])
            .map_err(|error| error.to_string())?;
        if read == 0 {
            break;
        }
        bytes.extend_from_slice(&buffer[..read]);
    }
    cancellation.check(events)?;
    if bytes.len() as u64 > MAX_SERVER_BYTES {
        return Err("Server binary exceeds upload limit".to_string());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// What one `open` of a scripted tunnel returns.
    #[derive(Clone, Copy)]
    enum Open {
        Ok,
        Refused,
        Stalled,
    }

    #[derive(Default)]
    struct ScriptedLink {
        opens: std::sync::Mutex<std::collections::VecDeque<Open>>,
        calls: std::sync::Mutex<Vec<&'static str>>,
    }

    impl ScriptedLink {
        fn new(opens: impl IntoIterator<Item = Open>) -> Self {
            Self {
                opens: std::sync::Mutex::new(opens.into_iter().collect()),
                calls: Default::default(),
            }
        }
    }

    impl TunnelLink for Arc<ScriptedLink> {
        fn open(
            &self,
        ) -> yttt_transport::BoxFuture<'_, Result<yttt_transport::TransportStream, TunnelFailure>>
        {
            Box::pin(async move {
                self.calls.lock().unwrap().push("open");
                match self
                    .opens
                    .lock()
                    .unwrap()
                    .pop_front()
                    .expect("unscripted open")
                {
                    Open::Ok => {
                        Ok(Box::new(tokio::io::duplex(64).0) as yttt_transport::TransportStream)
                    }
                    Open::Refused => Err(TunnelFailure::Refused("no listener".into())),
                    Open::Stalled => Err(TunnelFailure::Session("timed out".into())),
                }
            })
        }

        fn rebuild(
            &self,
        ) -> yttt_transport::BoxFuture<'_, Result<(), yttt_transport::TransportError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("rebuild");
                Ok(())
            })
        }

        fn ensure_server(
            &self,
        ) -> yttt_transport::BoxFuture<'_, Result<(), yttt_transport::TransportError>> {
            Box::pin(async move {
                self.calls.lock().unwrap().push("ensure");
                Ok(())
            })
        }
    }

    fn tunnel_calls(opens: impl IntoIterator<Item = Open>) -> (bool, Vec<&'static str>) {
        let link = Arc::new(ScriptedLink::new(opens));
        let connector = TunnelConnector::new(link.clone());
        let connected = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(connector.connect_tunnel())
            .is_ok();
        let calls = link.calls.lock().unwrap().clone();
        (connected, calls)
    }

    #[test]
    fn a_stalled_session_is_rebuilt_without_waiting_on_it_again() {
        assert_eq!(
            tunnel_calls([Open::Stalled, Open::Ok]),
            (true, vec!["open", "rebuild", "open"])
        );
    }

    #[test]
    fn a_reachable_remote_host_is_not_ensured() {
        assert_eq!(tunnel_calls([Open::Ok]), (true, vec!["open"]));
        assert!(
            !tunnel_calls([Open::Stalled, Open::Ok])
                .1
                .contains(&"ensure")
        );
    }

    #[test]
    fn a_stopped_remote_host_is_started_once() {
        assert_eq!(
            tunnel_calls([Open::Refused, Open::Ok]),
            (true, vec!["open", "ensure", "open"])
        );
        assert_eq!(
            tunnel_calls([Open::Stalled, Open::Refused, Open::Ok]),
            (true, vec!["open", "rebuild", "open", "ensure", "open"])
        );
        assert_eq!(
            tunnel_calls([Open::Refused, Open::Refused]),
            (false, vec!["open", "ensure", "open"])
        );
    }

    #[test]
    fn a_tunnel_reconnects_on_a_remote_link_budget() {
        let connector = TunnelConnector::new(Arc::new(ScriptedLink::default()));
        assert_eq!(
            connector.link_profile(),
            yttt_transport::LinkProfile::REMOTE
        );
    }

    #[test]
    fn cancelled_phase_does_not_start_a_remote_side_effect() {
        let cancellation = RemoteConnectCancellation::new();
        cancellation.cancel();
        let (events, _receiver) = flume::unbounded();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let ran = Arc::new(AtomicBool::new(false));
        let operation_ran = ran.clone();

        let result = run_cancellable_blocking(&runtime, &cancellation, &events, move || {
            operation_ran.store(true, Ordering::Release);
            Ok(())
        });

        assert_eq!(result, Err(CONNECT_CANCELLED.to_string()));
        assert!(!ran.load(Ordering::Acquire));
    }

    #[test]
    fn cancellation_releases_the_runtime_before_a_blocked_download_finishes() {
        let cancellation = RemoteConnectCancellation::new();
        let worker_cancellation = cancellation.clone();
        let (started, start) = flume::bounded(1);
        let (release, blocked) = flume::bounded(1);
        let (finished, result) = flume::bounded(1);
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap();
            let (events, _receiver) = flume::unbounded();
            let outcome =
                run_cancellable_blocking(&runtime, &worker_cancellation, &events, move || {
                    started.send(()).unwrap();
                    blocked.recv().unwrap();
                    Ok(())
                });
            drop(runtime);
            finished.send(outcome).unwrap();
        });
        start.recv_timeout(Duration::from_secs(2)).unwrap();
        cancellation.cancel();
        let outcome = result.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        worker.join().unwrap();
        assert_eq!(outcome.unwrap(), Err(CONNECT_CANCELLED.to_string()));
    }
}
