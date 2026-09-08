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
use yttt_transport::{AuthToken, ClientIdentity, SharedConnector, TransportConnector};
use zeroize::{Zeroize, Zeroizing};

pub enum RemoteConnectEvent {
    Status(String),
    HostKey(yttt_ssh::HostKeyChallenge),
    Takeover {
        owner: String,
        force: bool,
        answer: flume::Sender<bool>,
    },
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
            SshAuthPreference::Password if self.password.is_some() => Authentication::Password {
                secret: self.password.clone().unwrap(),
                save_as: self.save_password_as.clone(),
            },
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

struct RemoteConnector {
    service: TransportService,
    credentials: Arc<ConnectionCredentials>,
    socket: String,
    server: String,
    gate: tokio::sync::Mutex<()>,
}
impl TransportConnector for RemoteConnector {
    fn connect(
        &self,
    ) -> yttt_transport::BoxFuture<
        '_,
        Result<yttt_transport::TransportStream, yttt_transport::TransportError>,
    > {
        Box::pin(async move {
            let connector = self
                .service
                .streamlocal_connector(self.credentials.connection.id.clone(), self.socket.clone());
            if let Ok(stream) = connector.connect().await {
                return Ok(stream);
            }
            let _guard = self.gate.lock().await;
            if let Ok(stream) = connector.connect().await {
                return Ok(stream);
            }
            self.service
                .connect(self.credentials.request(true))
                .await
                .map_err(|error| yttt_transport::TransportError::Other(error.to_string()))?;
            let service = self.service.clone();
            let id = self.credentials.connection.id.clone();
            let server = self.server.clone();
            tokio::task::spawn_blocking(move || ensure_server(&service, id, &server))
                .await
                .map_err(|error| yttt_transport::TransportError::Other(error.to_string()))?
                .map_err(yttt_transport::TransportError::Other)?;
            connector.connect().await
        })
    }
}

pub fn connect(
    launch: RemoteLaunch,
    events: flume::Sender<RemoteConnectEvent>,
) -> Result<RemoteEnvironment, String> {
    match &launch.target {
        RemoteTarget::SshServer { .. } => connect_ssh(launch, events),
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
            let _ = events.send(RemoteConnectEvent::Status(
                "Verifying the existing Host certificate and environment; no Server is deployed"
                    .into(),
            ));
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
                channel: ConnectionChannel::Control,
                terminal_session_id: None,
            };
            initialize_environment(
                runtime,
                connector,
                identity,
                AuthToken::from_bytes(connection_info.work_secret),
                events,
            )
        }
    }
}

fn connect_ssh(
    mut launch: RemoteLaunch,
    events: flume::Sender<RemoteConnectEvent>,
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
    let config = launch.local_profile.config_paths();
    let service = TransportService::start_with_credential_namespace(
        config.ssh_host_keys_file(),
        launch.local_profile.credential_namespace(),
    )
    .map_err(|error| error.to_string())?;
    let ssh_events = service.events();
    let ui_events = events.clone();
    let saved_connection = credentials.connection.clone();
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
                            break;
                        }
                    }
                    TransportEvent::StateChanged(status) => {
                        let _ = ui_events.send(RemoteConnectEvent::Status(format!(
                            "SSH {:?}{}",
                            status.state,
                            status
                                .error
                                .map(|error| format!(": {error}"))
                                .unwrap_or_default()
                        )));
                    }
                    TransportEvent::CredentialSaved { credential, .. } => {
                        if let Ok(mut connections) =
                            crate::config::ssh::load_ssh_connections(&config)
                        {
                            if let Some(connection) = connections
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
                                let _ =
                                    crate::config::ssh::save_ssh_connections(&config, &connections);
                            }
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
    runtime
        .block_on(service.connect(credentials.request(false)))
        .map_err(|error| error.to_string())?;
    let _ = events.send(RemoteConnectEvent::Status(
        "Checking remote platform and Server installation".to_string(),
    ));
    let server = deploy_server(&service, &credentials.connection)?;
    let _ = events.send(RemoteConnectEvent::Status(
        "Starting or attaching independent remote Host".to_string(),
    ));
    let descriptor = ensure_server(&service, credentials.connection.id.clone(), &server)?;
    if descriptor.resource_protocol != yttt_protocol::RESOURCE_PROTOCOL_VERSION
        || descriptor.lifecycle_protocol != yttt_protocol::LIFECYCLE_PROTOCOL_VERSION
    {
        return Err(
            "Remote Host protocol is incompatible; its running resources were not stopped"
                .to_string(),
        );
    }
    let connector = SharedConnector::new(RemoteConnector {
        service,
        credentials: credentials.clone(),
        socket: descriptor.endpoint.clone(),
        server,
        gate: tokio::sync::Mutex::new(()),
    });
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
        channel: ConnectionChannel::Control,
        terminal_session_id: None,
    };
    initialize_environment(runtime, connector, identity, token, events)
}

fn initialize_environment(
    runtime: tokio::runtime::Runtime,
    connector: SharedConnector,
    identity: ClientIdentity,
    token: AuthToken,
    events: flume::Sender<RemoteConnectEvent>,
) -> Result<RemoteEnvironment, String> {
    let client = Arc::new(
        runtime
            .block_on(ClientCore::connect(
                connector.clone(),
                identity.clone(),
                token.clone(),
            ))
            .map_err(|error| error.to_string())?,
    );
    let environment = match runtime
        .block_on(client.request(Request::Workspace(WorkspaceRequest::Environment)))
        .map_err(|error| error.to_string())?
    {
        Response::Workspace(WorkspaceResponse::Environment(environment)) => environment,
        _ => return Err("unexpected Host environment response".into()),
    };
    let index = match runtime
        .block_on(client.request(Request::Workspace(WorkspaceRequest::List)))
        .map_err(|error| error.to_string())?
    {
        Response::Workspace(WorkspaceResponse::Workspaces(index)) => index,
        _ => return Err("unexpected Host workspace index".into()),
    };
    let mut control = client
        .control_status()
        .ok_or_else(|| "Host control state is unavailable".to_string())?;
    let take_control = confirm_transfer(
        &events,
        format!(
            "{} · {} · {} workspaces · {}",
            environment.environment_id,
            identity.profile_id,
            index.len(),
            control
                .owner
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_else(|| "unowned".into())
        ),
        false,
    )
    .is_ok();
    let request_control = |request| -> Result<yttt_protocol::session::ControlStatus, String> {
        match runtime
            .block_on(client.request(Request::ProfileControl(request)))
            .map_err(|error| error.to_string())?
        {
            Response::ProfileControl(status) => Ok(status),
            _ => Err("unexpected profile control response".to_string()),
        }
    };
    if take_control {
        control = request_control(yttt_protocol::session::ProfileControlRequest::RequestControl)?;
    }
    while take_control {
        if control.owner.as_ref() == Some(client.client_id()) {
            break;
        }
        let transfer = control.transfer.as_ref().ok_or_else(|| {
            "profile transfer was cancelled; the previous Client retains its work".to_string()
        })?;
        if events.is_disconnected() {
            let _ = request_control(yttt_protocol::session::ProfileControlRequest::Cancel {
                transfer_id: transfer.id.clone(),
            });
            return Err("remote connection window closed".to_string());
        }
        if transfer.phase == yttt_protocol::session::TransferPhase::ForceConfirmationRequired {
            if confirm_transfer(
                &events,
                transfer
                    .previous_owner
                    .as_ref()
                    .map(ToString::to_string)
                    .unwrap_or_default(),
                true,
            )
            .is_err()
            {
                let _ = request_control(yttt_protocol::session::ProfileControlRequest::Cancel {
                    transfer_id: transfer.id.clone(),
                });
                break;
            }
            control = request_control(
                yttt_protocol::session::ProfileControlRequest::ConfirmForce {
                    transfer_id: transfer.id.clone(),
                },
            )?;
        } else {
            std::thread::sleep(Duration::from_millis(100));
            control = request_control(yttt_protocol::session::ProfileControlRequest::Status)?;
        }
    }
    let config_root = environment
        .config_root
        .to_path()
        .map_err(|error| error.to_string())?;
    let storage = Arc::new(HostStorage::new(
        client.clone(),
        runtime.handle().clone(),
        config_root.clone(),
        environment.clone(),
        true,
    ));
    crate::config::storage::bind_environment(storage.clone()).map_err(|error| error.to_string())?;
    let config_paths = AppConfigPaths::from_config_dir(config_root);
    let home = environment
        .home
        .to_path()
        .map_err(|error| error.to_string())?;
    let label = format!("{} · {}", environment.environment_id, identity.profile_id);
    let host = DesktopHostRuntime::from_remote(
        runtime,
        client,
        storage,
        connector,
        identity,
        token,
        environment,
        label.clone(),
        &config_paths,
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
) -> Result<ServerDescriptor, String> {
    let root = service.sftp_project(
        id,
        RemotePathBuf::new("/").map_err(|error| error.to_string())?,
    );
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
) -> Result<String, String> {
    let root = service.sftp_project(
        connection.id.clone(),
        RemotePathBuf::new("/").map_err(|error| error.to_string())?,
    );
    let platform = root
        .run_command("uname", vec!["-s".to_string()])
        .map_err(|error| error.to_string())?;
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
    let present = root
        .run_command("test", vec!["-x".to_string(), server.clone()])
        .map_err(|error| error.to_string())?;
    if present.success() {
        return Ok(server);
    }
    let bytes = server_bytes(&target)?;
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
    match root
        .save_file(relative, bytes, None, false, MAX_SERVER_BYTES)
        .map_err(|error| error.to_string())?
    {
        yttt_ssh::RemoteSaveOutcome::Saved(_) => {}
        _ => return Err("Server upload conflicted with another writer".to_string()),
    }
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
fn server_bytes(target: &str) -> Result<Vec<u8>, String> {
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
        let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
        if file.metadata().map_err(|error| error.to_string())?.len() > MAX_SERVER_BYTES {
            return Err("Server binary exceeds upload limit".to_string());
        }
        let mut bytes = Vec::new();
        file.take(MAX_SERVER_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| error.to_string())?;
        return Ok(bytes);
    }
    let base = format!(
        "https://github.com/yuWorm/yttt/releases/download/v{}",
        env!("CARGO_PKG_VERSION")
    );
    let http = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|error| error.to_string())?;
    let sums = http
        .get(format!("{base}/SHA256SUMS"))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("cannot obtain Server checksums: {error}"))?
        .text()
        .map_err(|error| error.to_string())?;
    let expected = sums
        .lines()
        .find_map(|line| {
            let mut parts = line.split_whitespace();
            let hash = parts.next()?;
            (parts.next()? == filename).then_some(hash)
        })
        .ok_or_else(|| format!("release does not contain a checksum for {filename}"))?;
    let response = http
        .get(format!("{base}/{filename}"))
        .send()
        .and_then(|response| response.error_for_status())
        .map_err(|error| format!("cannot download remote Server: {error}"))?;
    let mut bytes = Vec::new();
    response
        .take(MAX_SERVER_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_SERVER_BYTES || format!("{:x}", Sha256::digest(&bytes)) != expected
    {
        return Err("remote Server download failed checksum or size validation".to_string());
    }
    Ok(bytes)
}

fn confirm_transfer(
    events: &flume::Sender<RemoteConnectEvent>,
    owner: String,
    force: bool,
) -> Result<(), String> {
    let (answer, receiver) = flume::bounded(1);
    events
        .send(RemoteConnectEvent::Takeover {
            owner,
            force,
            answer,
        })
        .map_err(|_| "remote connection window closed".to_string())?;
    if receiver
        .recv()
        .map_err(|_| "profile transfer cancelled".to_string())?
    {
        Ok(())
    } else {
        Err("profile transfer cancelled".to_string())
    }
}
