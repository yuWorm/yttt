use super::{ConnectionContext, device_settings::DeviceSettingsStore};
use parking_lot::Mutex;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashMap,
    fs, io,
    net::SocketAddr,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Semaphore, mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
};
use yttt_core::model::ids::ClientInstanceId;
use yttt_protocol::{
    FailureCode, ProtocolFailure, remote_access::*, session::ProfileControlRequest,
};
use yttt_transport::{AuthToken, IngressKind};
use yttt_transport_tls::TlsListener;
use zeroize::{Zeroize as _, Zeroizing};

#[derive(Clone)]
pub(crate) struct RemoteAccessHandle(mpsc::Sender<Command>);
pub(crate) struct Command {
    client: ClientInstanceId,
    request: RemoteAccessRequest,
    reply: oneshot::Sender<Result<RemoteAccessResponse, ProtocolFailure>>,
}
impl RemoteAccessHandle {
    pub fn channel() -> (Self, mpsc::Receiver<Command>) {
        let (send, receive) = mpsc::channel(16);
        (Self(send), receive)
    }
    pub async fn request(
        &self,
        client: ClientInstanceId,
        request: RemoteAccessRequest,
    ) -> Result<RemoteAccessResponse, ProtocolFailure> {
        let (reply, receive) = oneshot::channel();
        self.0
            .send(Command {
                client,
                request,
                reply,
            })
            .await
            .map_err(|_| failure("remote access manager stopped"))?;
        receive
            .await
            .map_err(|_| failure("remote access manager stopped"))?
    }
}

struct ClientStreams {
    connected_millis: u64,
    streams: usize,
    cancel: watch::Sender<bool>,
}

pub(crate) struct NetworkAdmission {
    accepting: AtomicBool,
    clients: Mutex<HashMap<ClientInstanceId, ClientStreams>>,
}
impl NetworkAdmission {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            accepting: AtomicBool::new(true),
            clients: Mutex::new(HashMap::new()),
        })
    }
    pub fn admit(self: &Arc<Self>, client: &ClientInstanceId) -> Result<NetworkSession, ()> {
        let mut clients = self.clients.lock();
        if !self.accepting.load(Ordering::Acquire) {
            return Err(());
        }
        if !clients.contains_key(client) && clients.len() >= MAX_REMOTE_CLIENTS {
            return Err(());
        }
        let streams = clients
            .entry(client.clone())
            .or_insert_with(|| ClientStreams {
                connected_millis: super::now_millis(),
                streams: 0,
                cancel: watch::channel(false).0,
            });
        if *streams.cancel.borrow() {
            return Err(());
        }
        streams.streams += 1;
        Ok(NetworkSession {
            admission: self.clone(),
            client: client.clone(),
            stop: streams.cancel.subscribe(),
        })
    }
    fn summaries(&self) -> Vec<RemoteClientSummary> {
        let mut clients = self
            .clients
            .lock()
            .iter()
            .map(|(client, streams)| RemoteClientSummary {
                client_id: client.clone(),
                connected_millis: streams.connected_millis,
                streams: streams.streams,
            })
            .collect::<Vec<_>>();
        clients.sort_by(|a, b| a.client_id.as_str().cmp(b.client_id.as_str()));
        clients
    }
    fn cancel(&self, client: Option<&ClientInstanceId>) {
        for (id, streams) in self.clients.lock().iter() {
            if client.is_none_or(|client| client == id) {
                streams.cancel.send_replace(true);
            }
        }
    }
    fn close(&self) {
        self.accepting.store(false, Ordering::Release);
        self.cancel(None);
    }
    async fn drain(&self, client: Option<&ClientInstanceId>) {
        loop {
            let done = {
                let clients = self.clients.lock();
                client.map_or(clients.is_empty(), |client| !clients.contains_key(client))
            };
            if done {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
pub(crate) struct NetworkSession {
    admission: Arc<NetworkAdmission>,
    client: ClientInstanceId,
    pub stop: watch::Receiver<bool>,
}
impl Drop for NetworkSession {
    fn drop(&mut self) {
        let mut clients = self.admission.clients.lock();
        if let Some(streams) = clients.get_mut(&self.client) {
            streams.streams -= 1;
            if streams.streams == 0 {
                clients.remove(&self.client);
            }
        }
    }
}

#[derive(Serialize, Deserialize)]
struct Credentials {
    certificate_der: Vec<u8>,
    private_key_der: Vec<u8>,
    work_secret: [u8; 32],
    generation: u64,
}
impl Drop for Credentials {
    fn drop(&mut self) {
        self.private_key_der.zeroize();
        self.work_secret.zeroize();
    }
}
impl Credentials {
    fn load(root: &Path) -> io::Result<Option<Self>> {
        let path = root.join("credentials.json");
        match fs::symlink_metadata(&path) {
            Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
            Ok(_) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "network credentials must be a private regular file",
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error),
        }
        super::validate_user_only_file(&path).map_err(io::Error::other)?;
        let bytes = Zeroizing::new(super::workspace::read_bounded_file(&path, 64 * 1024)?);
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(io::Error::other)
    }
    fn generate() -> io::Result<Self> {
        let rcgen::CertifiedKey { cert, key_pair } =
            rcgen::generate_simple_self_signed(vec![TLS_SERVER_NAME.to_string()])
                .map_err(io::Error::other)?;
        let mut work_secret = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut work_secret);
        Ok(Self {
            certificate_der: cert.der().to_vec(),
            private_key_der: key_pair.serialize_der(),
            work_secret,
            generation: 1,
        })
    }
    fn persist(&self, root: &Path) -> io::Result<()> {
        let bytes = Zeroizing::new(serde_json::to_vec(self).map_err(io::Error::other)?);
        super::workspace::atomic_write(
            &root.join("credentials.json"),
            &bytes,
            uuid::Uuid::new_v4().as_u128() as u64,
        )
    }
    fn connection_info(&self, context: &ConnectionContext) -> RemoteConnectionInfo {
        RemoteConnectionInfo {
            environment_id: context.identity.environment_id.clone(),
            profile_id: context.profile_id.clone(),
            server_name: TLS_SERVER_NAME.to_string(),
            certificate_der: self.certificate_der.clone(),
            certificate_sha256: format!("{:x}", Sha256::digest(&self.certificate_der)),
            credential_generation: self.generation,
            work_secret: self.work_secret,
        }
    }
}

struct NetworkGeneration {
    address: SocketAddr,
    cancel: watch::Sender<bool>,
    admission: Arc<NetworkAdmission>,
    task: JoinHandle<()>,
}
struct Manager {
    store: Arc<Mutex<DeviceSettingsStore>>,
    context: ConnectionContext,
    credentials: Option<Credentials>,
    network: Option<NetworkGeneration>,
    effective: RemoteAccessState,
    error: Option<String>,
    next_connection: Arc<AtomicU64>,
}

pub(crate) async fn run(
    context: ConnectionContext,
    mut commands: mpsc::Receiver<Command>,
    next_connection: Arc<AtomicU64>,
) {
    let store = context.device_settings.clone();
    let desired = store.lock().settings().clone();
    let mut stop = context.stop.clone();
    let mut manager = Manager {
        store,
        context,
        credentials: None,
        network: None,
        effective: RemoteAccessState::Disabled,
        error: None,
        next_connection,
    };
    if desired.enabled
        && let Err(error) = manager.enable(desired.listen_address, true).await
    {
        manager.effective = RemoteAccessState::Failed;
        manager.error = Some(error.message);
    }
    let mut health = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            command = commands.recv() => {
                let Some(command) = command else { break };
                let result = manager.handle(command.client, command.request).await;
                let _ = command.reply.send(result);
            }
            _ = health.tick() => {
                if manager.network.as_ref().is_some_and(|network| network.task.is_finished()) {
                    manager.stop_network().await;
                    manager.effective = RemoteAccessState::Failed;
                    manager.error = Some("network listener stopped unexpectedly; local Host and tasks remain running".to_string());
                }
            }
        }
    }
    manager.stop_network().await;
}

impl Manager {
    fn status(&self) -> RemoteAccessStatus {
        RemoteAccessStatus {
            settings: self.store.lock().settings().clone(),
            effective: self.effective,
            bound_address: self.network.as_ref().map(|network| network.address),
            sharing_ready: self.context.workspaces.sharing_ready(),
            error: self.error.clone(),
            clients: self
                .network
                .as_ref()
                .map(|network| network.admission.summaries())
                .unwrap_or_default(),
            control: self
                .context
                .workspaces
                .control
                .status(self.context.identity.host_epoch),
        }
    }
    async fn handle(
        &mut self,
        client: ClientInstanceId,
        request: RemoteAccessRequest,
    ) -> Result<RemoteAccessResponse, ProtocolFailure> {
        match request {
            RemoteAccessRequest::Status => {}
            RemoteAccessRequest::SetEnabled {
                enabled,
                listen_address,
                confirm_non_loopback,
            } => {
                if enabled && !listen_address.ip().is_loopback() && !confirm_non_loopback {
                    return Err(ProtocolFailure::new(
                        FailureCode::PermissionDenied,
                        "non-loopback listening requires local confirmation",
                        false,
                    ));
                }
                if enabled {
                    if self
                        .network
                        .as_ref()
                        .is_some_and(|network| network.address == listen_address)
                    {
                        return Ok(RemoteAccessResponse::Status(self.status()));
                    }
                    self.stop_network().await;
                    if let Err(error) = self.enable(listen_address, false).await {
                        self.effective = RemoteAccessState::Failed;
                        self.error = Some(error.message.clone());
                        return Err(error);
                    }
                } else {
                    self.stop_network().await;
                    let mut settings = self.store.lock().settings().clone();
                    settings.enabled = false;
                    settings.listen_address = listen_address;
                    self.error = self.store.lock().save(settings).err().map(|error| format!("Network is closed for this run, but the disabled preference could not be saved: {error}"));
                    self.return_local_control(&client).await?;
                }
            }
            RemoteAccessRequest::ExportConnectionInfo => {
                if self.network.is_none() {
                    return Err(failure(
                        "enable remote access before exporting connection information",
                    ));
                }
                let credentials = self
                    .credentials
                    .as_ref()
                    .ok_or_else(|| failure("network credentials are unavailable"))?;
                return Ok(RemoteAccessResponse::ConnectionInfo(
                    credentials.connection_info(&self.context),
                ));
            }
            RemoteAccessRequest::ResetCredentials => {
                self.stop_network().await;
                self.return_local_control(&client).await?;
                let root = self.store.lock().root().to_path_buf();
                let mut credentials = Credentials::load(&root)
                    .map_err(io_failure)?
                    .ok_or_else(|| failure("remote credentials have not been created"))?;
                credentials.generation = credentials
                    .generation
                    .checked_add(1)
                    .ok_or_else(|| failure("credential generation is exhausted"))?;
                rand::rngs::OsRng.fill_bytes(&mut credentials.work_secret);
                if let Err(error) = credentials.persist(&root) {
                    self.effective = RemoteAccessState::Failed;
                    self.error = Some(format!(
                        "Network is closed, but credential rotation could not be saved; old credentials may return after restart: {error}"
                    ));
                    return Err(io_failure(error));
                }
                self.credentials = Some(credentials);
                let settings = self.store.lock().settings().clone();
                if settings.enabled {
                    self.enable(settings.listen_address, true).await?;
                }
            }
            RemoteAccessRequest::DisconnectClient { client_id } => {
                if let Some(network) = &self.network {
                    network.admission.cancel(Some(&client_id));
                    network.admission.drain(Some(&client_id)).await;
                }
                self.return_local_control(&client).await?;
            }
            RemoteAccessRequest::DisconnectAll => {
                if let Some(network) = &self.network {
                    network.admission.cancel(None);
                    network.admission.drain(None).await;
                }
                self.return_local_control(&client).await?;
            }
        }
        Ok(RemoteAccessResponse::Status(self.status()))
    }
    async fn return_local_control(&self, client: &ClientInstanceId) -> Result<(), ProtocolFailure> {
        if self
            .context
            .workspaces
            .control
            .status(self.context.identity.host_epoch)
            .owner
            .is_none()
        {
            super::handle_profile_control(
                &self.context,
                client,
                ProfileControlRequest::RequestControl,
            )
            .await?;
        }
        Ok(())
    }
    async fn stop_network(&mut self) {
        if let Some(network) = self.network.take() {
            self.effective = RemoteAccessState::Stopping;
            network.admission.close();
            network.cancel.send_replace(true);
            let _ = network.task.await;
        }
        self.effective = RemoteAccessState::Disabled;
    }
    async fn enable(
        &mut self,
        address: SocketAddr,
        existing_only: bool,
    ) -> Result<(), ProtocolFailure> {
        if !self.context.workspaces.sharing_ready() {
            return Err(failure(
                "all workspaces must finish their initial publication before enabling remote access",
            ));
        }
        if address.port() == 0 {
            return Err(failure("choose an explicit nonzero listening port"));
        }
        self.effective = RemoteAccessState::Starting;
        self.error = None;
        let root = self.store.lock().root().to_path_buf();
        if self.credentials.is_none() {
            self.credentials = Credentials::load(&root).map_err(io_failure)?;
            if self.credentials.is_none() {
                if existing_only {
                    return Err(failure(
                        "persisted remote credentials are missing; enable remote access locally to create new credentials",
                    ));
                }
                let credentials = Credentials::generate().map_err(io_failure)?;
                credentials.persist(&root).map_err(io_failure)?;
                self.credentials = Some(credentials);
            }
        }
        let credentials = self.credentials.as_ref().expect("loaded credentials");
        let listener = TlsListener::bind(
            address,
            credentials.certificate_der.clone(),
            credentials.private_key_der.clone(),
        )
        .await
        .map_err(|error| {
            ProtocolFailure::new(
                FailureCode::AddressConflict,
                format!("cannot open remote listener: {error}"),
                false,
            )
        })?;
        let address = listener.local_addr().map_err(io_failure)?;
        let mut settings = self.store.lock().settings().clone();
        settings.enabled = true;
        settings.listen_address = address;
        self.store.lock().save(settings).map_err(io_failure)?;
        let token = Arc::new(AuthToken::from_bytes(credentials.work_secret));
        let (cancel, stop) = watch::channel(false);
        let admission = NetworkAdmission::new();
        let mut context = self.context.clone();
        context.identity.ingress = IngressKind::TlsWork;
        context.identity.credential_generation = credentials.generation;
        context.stop = stop;
        context.network_admission = Some(admission.clone());
        let sequence = self.next_connection.clone();
        let task = tokio::spawn(network_loop(listener, token, context, sequence));
        self.network = Some(NetworkGeneration {
            address,
            cancel,
            admission,
            task,
        });
        self.effective = RemoteAccessState::Listening;
        Ok(())
    }
}

async fn network_loop(
    listener: TlsListener,
    token: Arc<AuthToken>,
    context: ConnectionContext,
    sequence: Arc<AtomicU64>,
) {
    let handshakes = Arc::new(Semaphore::new(MAX_PENDING_HANDSHAKES));
    let streams = Arc::new(Semaphore::new(MAX_REMOTE_STREAMS));
    let mut tasks = JoinSet::new();
    let mut stop = context.stop.clone();
    let mut next_accept = tokio::time::Instant::now();
    loop {
        tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                if !matches!(completed, Some(Ok(Ok(())))) { next_accept = tokio::time::Instant::now() + Duration::from_millis(100); }
            }
            accepted = async { tokio::time::sleep_until(next_accept).await; listener.accept_tcp().await } => {
                let Ok(stream) = accepted else { break };
                let (Ok(pending), Ok(slot)) = (handshakes.clone().try_acquire_owned(), streams.clone().try_acquire_owned()) else {
                    drop(stream);
                    next_accept = tokio::time::Instant::now() + Duration::from_millis(50);
                    continue;
                };
                let mut connection = context.clone();
                connection.identity.connection_sequence = sequence.fetch_add(1, Ordering::Relaxed);
                connection.handshake_permit = Some(Arc::new(Mutex::new(Some(pending))));
                let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
                connection.handshake_deadline = Some(deadline);
                let acceptor = listener.acceptor();
                let token = token.clone();
                let mut stop = connection.stop.clone();
                tasks.spawn(async move {
                    let _slot = slot;
                    let stream = tokio::select! {
                        _ = stop.changed() => return Ok(()),
                        stream = tokio::time::timeout_at(deadline, acceptor.accept(stream)) => stream.map_err(|_| ())?.map_err(|_| ())?,
                    };
                    super::serve_connection(Box::new(stream), token, connection).await
                });
            }
        }
    }
    drop(listener);
    context
        .network_admission
        .as_ref()
        .expect("network admission")
        .close();
    while tasks.join_next().await.is_some() {}
}
fn failure(message: impl Into<String>) -> ProtocolFailure {
    ProtocolFailure::new(FailureCode::InvalidRequest, message, false)
}
fn io_failure(error: io::Error) -> ProtocolFailure {
    ProtocolFailure::new(FailureCode::Internal, error.to_string(), false)
}
