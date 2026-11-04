use super::{ConnectionContext, device_settings::DeviceSettingsStore};
use parking_lot::Mutex;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::HashMap,
    fs, io,
    net::{IpAddr, SocketAddr},
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
use yttt_protocol::{FailureCode, ProtocolFailure, remote_access::*};
use yttt_transport::{AuthToken, IngressKind};
use yttt_transport_tls::TlsListener;
use zeroize::{Zeroize as _, Zeroizing};

#[derive(Clone)]
pub(crate) struct RemoteAccessHandle(mpsc::Sender<Command>);
pub(crate) struct Command {
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
        request: RemoteAccessRequest,
    ) -> Result<RemoteAccessResponse, ProtocolFailure> {
        let (reply, receive) = oneshot::channel();
        self.0
            .send(Command { request, reply })
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
/// How long the manager waits before rebinding a listener that failed to
/// open or stopped on its own, doubling up to the maximum.
const FIRST_REBIND_DELAY: Duration = Duration::from_secs(1);
const MAX_REBIND_DELAY: Duration = Duration::from_secs(30);

struct Rebind {
    at: tokio::time::Instant,
    delay: Duration,
}

struct Manager {
    store: Arc<Mutex<DeviceSettingsStore>>,
    context: ConnectionContext,
    credentials: Option<Credentials>,
    network: Option<NetworkGeneration>,
    effective: RemoteAccessState,
    error: Option<String>,
    next_connection: Arc<AtomicU64>,
    /// A pending attempt to restore the enabled listener without user action.
    rebind: Option<Rebind>,
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
        rebind: None,
    };
    if desired.enabled
        && let Err(error) = manager.enable(desired.listen_address, true).await
    {
        manager.listener_failed(error.message);
    }
    let mut health = tokio::time::interval(Duration::from_secs(1));
    loop {
        tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            command = commands.recv() => {
                let Some(command) = command else { break };
                let result = manager.handle(command.request).await;
                let _ = command.reply.send(result);
            }
            _ = health.tick() => {
                if manager.network.as_ref().is_some_and(|network| network.task.is_finished()) {
                    manager.stop_network().await;
                    manager.listener_failed("network listener stopped unexpectedly; local Host and tasks remain running".to_string());
                }
                if manager.rebind.as_ref().is_some_and(|rebind| rebind.at <= tokio::time::Instant::now()) {
                    manager.rebind().await;
                }
            }
        }
    }
    manager.stop_network().await;
}

impl Manager {
    /// Marks the listener failed and schedules the next attempt to reopen it.
    fn listener_failed(&mut self, error: String) {
        let delay = self.rebind.as_ref().map_or(FIRST_REBIND_DELAY, |rebind| {
            (rebind.delay * 2).min(MAX_REBIND_DELAY)
        });
        self.rebind = Some(Rebind {
            at: tokio::time::Instant::now() + delay,
            delay,
        });
        self.effective = RemoteAccessState::Failed;
        self.error = Some(format!("{error}; retrying in {} s", delay.as_secs()));
    }
    async fn rebind(&mut self) {
        let settings = self.store.lock().settings().clone();
        if !settings.enabled || self.network.is_some() {
            self.rebind = None;
            return;
        }
        match self.enable(settings.listen_address, true).await {
            Ok(()) => self.rebind = None,
            Err(error) => self.listener_failed(error.message),
        }
    }
    fn status(&self) -> RemoteAccessStatus {
        RemoteAccessStatus {
            settings: self.store.lock().settings().clone(),
            effective: self.effective,
            bound_address: self.network.as_ref().map(|network| network.address),
            error: self.error.clone(),
            clients: self
                .network
                .as_ref()
                .map(|network| network.admission.summaries())
                .unwrap_or_default(),
        }
    }
    async fn handle(
        &mut self,
        request: RemoteAccessRequest,
    ) -> Result<RemoteAccessResponse, ProtocolFailure> {
        if !matches!(
            request,
            RemoteAccessRequest::Status | RemoteAccessRequest::ExportConnectionInfo
        ) {
            // The user's choice supersedes an automatic retry.
            self.rebind = None;
        }
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
            }
            RemoteAccessRequest::DisconnectAll => {
                if let Some(network) = &self.network {
                    network.admission.cancel(None);
                    network.admission.drain(None).await;
                }
            }
        }
        Ok(RemoteAccessResponse::Status(self.status()))
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

/// How long an accepted connection has for TLS and authentication together.
const NETWORK_HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);
/// The longest pause after accept fails, e.g. while file descriptors run out.
const MAX_ACCEPT_BACKOFF: Duration = Duration::from_secs(1);

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
    let mut accept_backoff = Duration::ZERO;
    let mut penalties = HandshakePenalties::default();
    loop {
        tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            completed = tasks.join_next(), if !tasks.is_empty() => {
                if let Some(Ok(Some((peer, authenticated)))) = completed {
                    let now = tokio::time::Instant::now();
                    if authenticated { penalties.forgive(peer) } else { penalties.record_failure(peer, now) }
                }
            }
            accepted = async { tokio::time::sleep_until(next_accept).await; listener.accept_tcp().await } => {
                let (stream, peer) = match accepted {
                    Ok(accepted) => { accept_backoff = Duration::ZERO; accepted }
                    Err(_) => {
                        // Errors such as running out of descriptors pass; back off and keep listening.
                        accept_backoff = (accept_backoff * 2).clamp(Duration::from_millis(10), MAX_ACCEPT_BACKOFF);
                        next_accept = tokio::time::Instant::now() + accept_backoff;
                        continue;
                    }
                };
                let peer = peer.ip();
                let Some(start) = penalties.admit(peer, tokio::time::Instant::now()) else {
                    drop(stream);
                    continue;
                };
                let (Ok(pending), Ok(slot)) = (handshakes.clone().try_acquire_owned(), streams.clone().try_acquire_owned()) else {
                    drop(stream);
                    next_accept = tokio::time::Instant::now() + Duration::from_millis(50);
                    continue;
                };
                let mut connection = context.clone();
                connection.identity.connection_sequence = sequence.fetch_add(1, Ordering::Relaxed);
                // `serve_connection` releases the permit once the Client authenticates.
                let permit = Arc::new(Mutex::new(Some(pending)));
                connection.handshake_permit = Some(permit.clone());
                let deadline = start + NETWORK_HANDSHAKE_TIMEOUT;
                connection.handshake_deadline = Some(deadline);
                let acceptor = listener.acceptor();
                let token = token.clone();
                let mut stop = connection.stop.clone();
                tasks.spawn(async move {
                    let _slot = slot;
                    let stream = tokio::select! {
                        _ = stop.changed() => return None,
                        stream = async {
                            tokio::time::sleep_until(start).await;
                            tokio::time::timeout_at(deadline, acceptor.accept(stream)).await
                        } => stream,
                    };
                    if let Ok(Ok(stream)) = stream {
                        let _ = super::serve_connection(Box::new(stream), token, connection).await;
                    }
                    let authenticated = permit.lock().is_none();
                    Some((peer, authenticated))
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

/// The pause after the first unauthenticated failure from an address; it
/// doubles with each further failure up to the maximum, which stays below a
/// Client's TLS handshake timeout so a delayed Client still connects.
const FIRST_HANDSHAKE_PENALTY: Duration = Duration::from_millis(100);
const MAX_HANDSHAKE_PENALTY: Duration = Duration::from_secs(5);
/// How long an address's failures are remembered after its pause ends.
const HANDSHAKE_PENALTY_MEMORY: Duration = Duration::from_secs(60);
const MAX_PENALIZED_ADDRESSES: usize = 4096;

/// Delays connections from an address whose connections recently failed
/// before authenticating, so one misbehaving peer cannot slow down others.
/// Clients sharing the address, e.g. behind NAT, are delayed, not refused.
#[derive(Default)]
struct HandshakePenalties {
    addresses: HashMap<IpAddr, HandshakePenalty>,
}

struct HandshakePenalty {
    failures: u32,
    until: tokio::time::Instant,
    /// A connection is already waiting for this pause to end.
    waiting: bool,
}

impl HandshakePenalties {
    /// When a new connection from `address` may start its handshake, or
    /// `None` to refuse it because another one is already waiting.
    fn admit(
        &mut self,
        address: IpAddr,
        now: tokio::time::Instant,
    ) -> Option<tokio::time::Instant> {
        let Some(penalty) = self.addresses.get_mut(&address) else {
            return Some(now);
        };
        if now >= penalty.until {
            return Some(now);
        }
        if penalty.waiting {
            return None;
        }
        penalty.waiting = true;
        Some(penalty.until)
    }

    fn record_failure(&mut self, address: IpAddr, now: tokio::time::Instant) {
        if self.addresses.len() >= MAX_PENALIZED_ADDRESSES {
            self.addresses
                .retain(|_, penalty| now < penalty.until + HANDSHAKE_PENALTY_MEMORY);
        }
        let known = self.addresses.contains_key(&address);
        if !known && self.addresses.len() >= MAX_PENALIZED_ADDRESSES {
            return;
        }
        let penalty = self.addresses.entry(address).or_insert(HandshakePenalty {
            failures: 0,
            until: now,
            waiting: false,
        });
        if now >= penalty.until + HANDSHAKE_PENALTY_MEMORY {
            penalty.failures = 0;
        }
        penalty.failures = penalty.failures.saturating_add(1);
        let delay = FIRST_HANDSHAKE_PENALTY
            .saturating_mul(1 << (penalty.failures - 1).min(16))
            .min(MAX_HANDSHAKE_PENALTY);
        penalty.until = now + delay;
        penalty.waiting = false;
    }

    fn forgive(&mut self, address: IpAddr) {
        self.addresses.remove(&address);
    }
}

fn failure(message: impl Into<String>) -> ProtocolFailure {
    ProtocolFailure::new(FailureCode::InvalidRequest, message, false)
}
fn io_failure(error: io::Error) -> ProtocolFailure {
    ProtocolFailure::new(FailureCode::Internal, error.to_string(), false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(last: u8) -> IpAddr {
        IpAddr::from([192, 0, 2, last])
    }

    #[test]
    fn a_failing_address_is_delayed_without_affecting_others() {
        let mut penalties = HandshakePenalties::default();
        let now = tokio::time::Instant::now();
        penalties.record_failure(address(1), now);
        assert_eq!(
            penalties.admit(address(1), now),
            Some(now + FIRST_HANDSHAKE_PENALTY)
        );
        assert_eq!(penalties.admit(address(2), now), Some(now));
        let later = now + FIRST_HANDSHAKE_PENALTY;
        assert_eq!(penalties.admit(address(1), later), Some(later));
    }

    #[test]
    fn only_one_connection_waits_per_address() {
        let mut penalties = HandshakePenalties::default();
        let now = tokio::time::Instant::now();
        penalties.record_failure(address(1), now);
        assert!(penalties.admit(address(1), now).is_some());
        assert_eq!(penalties.admit(address(1), now), None);
        // The waiting connection failed as well: the next one may wait again.
        penalties.record_failure(address(1), now);
        assert_eq!(
            penalties.admit(address(1), now),
            Some(now + FIRST_HANDSHAKE_PENALTY * 2)
        );
    }

    #[test]
    fn repeated_failures_double_the_pause_up_to_the_maximum() {
        let mut penalties = HandshakePenalties::default();
        let mut now = tokio::time::Instant::now();
        let mut expected = FIRST_HANDSHAKE_PENALTY;
        for _ in 0..12 {
            penalties.record_failure(address(1), now);
            assert_eq!(penalties.admit(address(1), now), Some(now + expected));
            now += expected;
            expected = (expected * 2).min(MAX_HANDSHAKE_PENALTY);
        }
        assert_eq!(expected, MAX_HANDSHAKE_PENALTY);
    }

    #[test]
    fn authentication_or_a_quiet_minute_forgets_failures() {
        let mut penalties = HandshakePenalties::default();
        let now = tokio::time::Instant::now();
        penalties.record_failure(address(1), now);
        penalties.record_failure(address(1), now);
        penalties.forgive(address(1));
        assert_eq!(penalties.admit(address(1), now), Some(now));

        penalties.record_failure(address(2), now);
        penalties.record_failure(address(2), now);
        let later = now + FIRST_HANDSHAKE_PENALTY * 2 + HANDSHAKE_PENALTY_MEMORY;
        penalties.record_failure(address(2), later);
        assert_eq!(
            penalties.admit(address(2), later),
            Some(later + FIRST_HANDSHAKE_PENALTY)
        );
    }

    #[test]
    fn the_table_of_penalized_addresses_is_bounded() {
        let mut penalties = HandshakePenalties::default();
        let now = tokio::time::Instant::now();
        for index in 0..MAX_PENALIZED_ADDRESSES as u32 + 10 {
            penalties.record_failure(IpAddr::from(index.to_be_bytes()), now);
        }
        assert_eq!(penalties.addresses.len(), MAX_PENALIZED_ADDRESSES);
    }
}
