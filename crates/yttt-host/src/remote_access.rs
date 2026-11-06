use super::{
    ConnectionContext,
    device_settings::DeviceSettingsStore,
    p2p::{
        self, Decision, DeviceRegistry, HostGate, P2pGeneration, P2pShared, PairingSlot,
        StoredRelayAccount,
    },
};
use parking_lot::Mutex;
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    net::{IpAddr, SocketAddr},
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};
use tokio::{
    sync::{Semaphore, mpsc, oneshot, watch},
    task::{JoinHandle, JoinSet},
};
use yttt_control_client::{ControlSession, RelayTicket};
use yttt_core::model::ids::ClientInstanceId;
use yttt_protocol::{
    EndpointKey, FailureCode, ProtocolFailure,
    p2p::{
        MAX_DIRECT_ADDRS, MAX_RELAY_URL_BYTES, PAIR_ALPN, PairRejectReason, PairedDevice,
        PairingCode, WORK_ALPN,
    },
    remote_access::*,
};
use yttt_transport::{AuthToken, Credential, IngressKind};
use yttt_transport_iroh::{DirectPaths, EndpointConfig, IrohEndpoint, RelaySetup, RelayTarget};
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
    ingress: IngressKind,
    client_id: ClientInstanceId,
    connected_millis: u64,
    streams: usize,
    cancel: watch::Sender<bool>,
}

/// Who a remote session belongs to. TLS sessions share one secret, so only their
/// Client id tells them apart; P2P sessions are keyed by the authenticated device.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Principal {
    Session(ClientInstanceId),
    Device(EndpointKey),
}

#[derive(Default)]
struct AdmissionState {
    open: HashSet<IngressKind>,
    clients: HashMap<Principal, ClientStreams>,
}

/// Remote sessions of every network ingress, which share the client limit.
pub(crate) struct NetworkAdmission {
    state: Mutex<AdmissionState>,
}
impl NetworkAdmission {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(AdmissionState::default()),
        })
    }
    fn open(&self, ingress: IngressKind) {
        self.state.lock().open.insert(ingress);
    }
    /// Stops admitting through `ingress` and cancels its sessions only.
    fn close(&self, ingress: IngressKind) {
        let mut state = self.state.lock();
        state.open.remove(&ingress);
        for streams in state.clients.values() {
            if streams.ingress == ingress {
                streams.cancel.send_replace(true);
            }
        }
    }
    /// `still_allowed` runs under the admission lock, so a revocation that removes
    /// the device and then cancels its sessions cannot miss this one.
    pub fn admit(
        self: &Arc<Self>,
        ingress: IngressKind,
        principal: Principal,
        client_id: &ClientInstanceId,
        still_allowed: impl FnOnce() -> bool,
    ) -> Result<NetworkSession, ()> {
        let mut state = self.state.lock();
        if !state.open.contains(&ingress) || !still_allowed() {
            return Err(());
        }
        if !state.clients.contains_key(&principal) && state.clients.len() >= MAX_REMOTE_CLIENTS {
            return Err(());
        }
        let streams = state
            .clients
            .entry(principal.clone())
            .or_insert_with(|| ClientStreams {
                ingress,
                client_id: client_id.clone(),
                connected_millis: super::now_millis(),
                streams: 0,
                cancel: watch::channel(false).0,
            });
        if streams.ingress != ingress || *streams.cancel.borrow() {
            return Err(());
        }
        streams.client_id = client_id.clone();
        streams.streams += 1;
        Ok(NetworkSession {
            admission: self.clone(),
            principal,
            stop: streams.cancel.subscribe(),
        })
    }
    fn summaries(&self) -> Vec<RemoteClientSummary> {
        let mut clients = self
            .state
            .lock()
            .clients
            .iter()
            .map(|(principal, streams)| RemoteClientSummary {
                client_id: streams.client_id.clone(),
                connected_millis: streams.connected_millis,
                streams: streams.streams,
                device: match principal {
                    Principal::Device(device) => Some(*device),
                    Principal::Session(_) => None,
                },
            })
            .collect::<Vec<_>>();
        clients.sort_by(|a, b| a.client_id.as_str().cmp(b.client_id.as_str()));
        clients
    }
    fn cancel(&self, selects: impl Fn(&Principal, &ClientStreams) -> bool) {
        for (principal, streams) in self.state.lock().clients.iter() {
            if selects(principal, streams) {
                streams.cancel.send_replace(true);
            }
        }
    }
    async fn drain(&self, selects: impl Fn(&Principal, &ClientStreams) -> bool) {
        loop {
            let done = !self
                .state
                .lock()
                .clients
                .iter()
                .any(|(principal, streams)| selects(principal, streams));
            if done {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
pub(crate) struct NetworkSession {
    admission: Arc<NetworkAdmission>,
    principal: Principal,
    pub stop: watch::Receiver<bool>,
}
impl Drop for NetworkSession {
    fn drop(&mut self) {
        let mut state = self.admission.state.lock();
        if let Some(streams) = state.clients.get_mut(&self.principal) {
            streams.streams -= 1;
            if streams.streams == 0 {
                state.clients.remove(&self.principal);
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
    task: JoinHandle<()>,
}
/// How long the manager waits before rebinding a listener that failed to
/// open or stopped on its own, doubling up to the maximum.
const FIRST_REBIND_DELAY: Duration = Duration::from_secs(1);
const MAX_REBIND_DELAY: Duration = Duration::from_secs(30);
/// Direct addresses a pairing code names; hole punching finds the rest.
const MAX_ADVERTISED_ADDRS: usize = 4;
/// How long before its first relay ticket expires a P2P endpoint is rebuilt.
const RESTART_BEFORE_EXPIRY_SECS: u64 = 3600;

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
    /// Sessions and streams of both network ingresses.
    admission: Arc<NetworkAdmission>,
    streams: Arc<Semaphore>,
    p2p: Option<P2pGeneration>,
    p2p_effective: RemoteAccessState,
    p2p_error: Option<String>,
    devices: Option<Arc<DeviceRegistry>>,
    pairing: Arc<Mutex<PairingSlot>>,
}

pub(crate) async fn run(
    mut context: ConnectionContext,
    mut commands: mpsc::Receiver<Command>,
    next_connection: Arc<AtomicU64>,
) {
    let store = context.device_settings.clone();
    let desired = store.lock().settings().clone();
    let mut stop = context.stop.clone();
    let admission = NetworkAdmission::new();
    context.network_admission = Some(admission.clone());
    let mut manager = Manager {
        store,
        context,
        credentials: None,
        network: None,
        effective: RemoteAccessState::Disabled,
        error: None,
        next_connection,
        rebind: None,
        admission,
        streams: Arc::new(Semaphore::new(MAX_REMOTE_STREAMS)),
        p2p: None,
        p2p_effective: RemoteAccessState::Disabled,
        p2p_error: None,
        devices: None,
        pairing: Arc::new(Mutex::new(PairingSlot::default())),
    };
    if desired.enabled
        && let Err(error) = manager.enable(desired.listen_address, true).await
    {
        manager.listener_failed(error.message);
    }
    if desired.p2p.enabled
        && let Err(error) = manager.start_p2p(desired.p2p.relay_url.clone()).await
    {
        manager.p2p_failed(error.message);
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
                if manager.p2p.as_ref().is_some_and(|p2p| p2p.task.is_finished()) {
                    manager.stop_p2p().await;
                    manager.p2p_failed("P2P endpoint stopped unexpectedly".to_string());
                }
                if manager.p2p.as_ref().is_some_and(|p2p| p2p.restart_due(yttt_control_client::now_unix())) {
                    // Its relay connection still presents the first ticket, which
                    // is about to expire; a new endpoint starts with the renewed one.
                    manager.restart_p2p().await;
                }
            }
        }
    }
    manager.stop_network().await;
    manager.stop_p2p().await;
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
    fn p2p_failed(&mut self, error: String) {
        self.p2p_effective = RemoteAccessState::Failed;
        self.p2p_error = Some(error);
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
        let now = super::now_millis();
        RemoteAccessStatus {
            settings: self.store.lock().settings().clone(),
            effective: self.effective,
            bound_address: self.network.as_ref().map(|network| network.address),
            error: self.error.clone(),
            clients: self.admission.summaries(),
            p2p: P2pStatus {
                effective: self.p2p_effective,
                endpoint_id: self.p2p.as_ref().map(|p2p| p2p.shared.endpoint.id()),
                error: self.p2p_error.clone(),
                pairing: self.pairing.lock().state(now),
                relay_account: self
                    .p2p
                    .as_ref()
                    .and_then(|p2p| p2p.relay_account.as_ref())
                    .map(|status| Box::new(status.lock().clone())),
            },
        }
    }
    fn devices(&mut self) -> Result<Arc<DeviceRegistry>, ProtocolFailure> {
        if let Some(devices) = &self.devices {
            return Ok(devices.clone());
        }
        let root = self.store.lock().root().to_path_buf();
        let devices = Arc::new(DeviceRegistry::load(&root).map_err(io_failure)?);
        self.devices = Some(devices.clone());
        Ok(devices)
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
            RemoteAccessRequest::ResetCredentials => self.reset_credentials().await?,
            RemoteAccessRequest::DisconnectClient { client_id } => {
                let selects =
                    |_: &Principal, streams: &ClientStreams| streams.client_id == client_id;
                self.admission.cancel(selects);
                self.admission.drain(selects).await;
            }
            RemoteAccessRequest::DisconnectAll => {
                self.admission.cancel(|_, _| true);
                self.admission.drain(|_, _| true).await;
            }
            RemoteAccessRequest::SetP2p { enabled, relay_url } => {
                if let Some(url) = &relay_url
                    && (url.len() > MAX_RELAY_URL_BYTES || !url.starts_with("http"))
                {
                    return Err(failure("the relay URL must be an http or https URL"));
                }
                let mut settings = self.store.lock().settings().clone();
                settings.p2p = P2pSettings {
                    enabled,
                    relay_url: relay_url.clone(),
                };
                self.stop_p2p().await;
                self.store.lock().save(settings).map_err(io_failure)?;
                if enabled && let Err(error) = self.start_p2p(relay_url).await {
                    self.p2p_failed(error.message.clone());
                    return Err(error);
                }
            }
            RemoteAccessRequest::SetRelayAccount { account } => {
                let root = self.store.lock().root().to_path_buf();
                match account {
                    Some(account) => {
                        if account.control_url.len() > MAX_RELAY_URL_BYTES
                            || !account.control_url.starts_with("http")
                        {
                            return Err(failure("the control URL must be an http or https URL"));
                        }
                        let stored = StoredRelayAccount {
                            session: ControlSession {
                                control_url: account.control_url.clone(),
                                account_id: account.account_id,
                                token: account.session.0.clone(),
                            },
                            ticket: None,
                        };
                        p2p::save_relay_account(&root, &stored).map_err(io_failure)?;
                    }
                    None => p2p::forget_relay_account(&root).map_err(io_failure)?,
                }
                if self.p2p.is_some() {
                    self.restart_p2p().await;
                }
            }
            RemoteAccessRequest::StartPairing => return self.start_pairing(),
            RemoteAccessRequest::ConfirmPairing {
                endpoint_id,
                preset,
            } => {
                let devices = self.devices()?;
                let host_label = self.context.identity.environment_id.clone();
                let mut pairing = self.pairing.lock();
                let label = pairing
                    .claimed_label(endpoint_id, super::now_millis())
                    .ok_or_else(|| failure("no device with that key is waiting to pair"))?;
                // Saved before the device hears it was accepted.
                devices
                    .insert(PairedDevice {
                        endpoint_id,
                        label,
                        preset,
                        paired_at_millis: super::now_millis(),
                        last_seen_millis: None,
                    })
                    .map_err(io_failure)?;
                pairing.decide(Decision::Accept { host_label });
            }
            RemoteAccessRequest::CancelPairing => {
                let mut pairing = self.pairing.lock();
                pairing.close(PairRejectReason::Declined);
                *pairing = PairingSlot::default();
            }
            RemoteAccessRequest::ListDevices => {
                return Ok(RemoteAccessResponse::Devices(self.devices()?.list()));
            }
            RemoteAccessRequest::RevokeDevice {
                endpoint_id,
                reset_tls,
            } => {
                // The gate and the admission check both read the registry, so after
                // this no new connection, stream or session of the device starts.
                self.devices()?.remove(endpoint_id).map_err(io_failure)?;
                if let Some(p2p) = &self.p2p {
                    p2p.shared.endpoint.close_peer(endpoint_id);
                }
                let selects = |principal: &Principal, _: &ClientStreams| {
                    *principal == Principal::Device(endpoint_id)
                };
                self.admission.cancel(selects);
                self.admission.drain(selects).await;
                if reset_tls {
                    self.reset_credentials().await?;
                }
            }
        }
        Ok(RemoteAccessResponse::Status(self.status()))
    }
    async fn reset_credentials(&mut self) -> Result<(), ProtocolFailure> {
        self.stop_network().await;
        let root = self.store.lock().root().to_path_buf();
        let Some(mut credentials) = Credentials::load(&root).map_err(io_failure)? else {
            // Nothing to rotate: TLS remote access was never set up.
            return Ok(());
        };
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
        Ok(())
    }
    fn start_pairing(&mut self) -> Result<RemoteAccessResponse, ProtocolFailure> {
        let p2p = self
            .p2p
            .as_ref()
            .ok_or_else(|| failure("turn on P2P remote access before pairing a device"))?;
        let endpoint = &p2p.shared.endpoint;
        let invitation = self.pairing.lock().open(super::now_millis());
        let mut direct_addrs: Vec<_> = endpoint
            .direct_addrs()
            .into_iter()
            .filter(|address| advertisable(address.ip()))
            .collect();
        // LAN IPv4 first: it is what a nearby device can dial, and a machine
        // can hold many temporary IPv6 addresses that only lengthen the code.
        direct_addrs.sort_by_key(|address| address.is_ipv6());
        direct_addrs.truncate(MAX_ADVERTISED_ADDRS.min(MAX_DIRECT_ADDRS));
        let code = PairingCode {
            host: endpoint.id(),
            relay_url: p2p.relay_url.lock().clone(),
            direct_addrs,
            invitation_id: invitation.id,
            secret: *invitation.secret,
            environment_id: self.context.identity.environment_id.clone(),
            profile_id: self.context.profile_id.clone(),
        };
        Ok(RemoteAccessResponse::PairingInvitation {
            code: SecretText(code.encode()),
            expires_millis: invitation.expires_millis,
        })
    }
    async fn start_p2p(&mut self, relay_url: Option<String>) -> Result<(), ProtocolFailure> {
        self.p2p_effective = RemoteAccessState::Starting;
        self.p2p_error = None;
        let root = self.store.lock().root().to_path_buf();
        let secret = Arc::new(p2p::load_or_create_host_key(&root).map_err(io_failure)?);
        let devices = self.devices()?;
        let account = p2p::load_relay_account(&root).map_err(io_failure)?;
        let mut relay_account = None;
        let mut first_ticket = None;
        let relay = match &account {
            None => RelaySetup::from(relay_url.map(|url| RelayTarget { url, token: None })),
            Some(stored) => {
                let mut status = RelayAccountStatus {
                    control_url: stored.session.control_url.clone(),
                    account_id: stored.session.account_id,
                    ..RelayAccountStatus::default()
                };
                let fetched = self.fetch_ticket(&stored.session, &secret).await;
                let ticket = match fetched {
                    Ok(ticket) => {
                        let cached = StoredRelayAccount {
                            session: stored.session.clone(),
                            ticket: Some(ticket.clone()),
                        };
                        if let Err(error) = p2p::save_relay_account(&root, &cached) {
                            eprintln!("could not cache the relay ticket: {error}");
                        }
                        Some(ticket)
                    }
                    Err(error) => {
                        status.error = Some(format!("relay account: {error}"));
                        let now = yttt_control_client::now_unix();
                        stored.ticket.clone().filter(|ticket| ticket.usable(now))
                    }
                };
                status.relay_url = ticket.as_ref().map(|ticket| ticket.relay_url.clone());
                status.ticket_expires_unix = ticket.as_ref().map(|ticket| ticket.expires_unix);
                relay_account = Some(Arc::new(Mutex::new(status)));
                let setup = ticket.as_ref().map_or(RelaySetup::Pending, |ticket| {
                    RelaySetup::Use(ticket.target())
                });
                first_ticket = ticket;
                setup
            }
        };
        let advertised = match &relay {
            RelaySetup::Use(target) => Some(target.url.clone()),
            RelaySetup::Off | RelaySetup::Pending => None,
        };
        let endpoint = IrohEndpoint::bind(EndpointConfig {
            secret: **secret,
            relay,
            alpns: vec![WORK_ALPN.to_vec(), PAIR_ALPN.to_vec()],
            gate: Arc::new(HostGate {
                devices: devices.clone(),
                pairing: self.pairing.clone(),
            }),
            direct: DirectPaths::Any,
        })
        .await
        .map_err(|error| failure(format!("cannot open the P2P endpoint: {error}")))?;
        let (cancel, stop) = watch::channel(false);
        let mut context = self.context.clone();
        context.stop = stop;
        let shared = P2pShared {
            endpoint,
            secret,
            devices,
            pairing: self.pairing.clone(),
            streams: self.streams.clone(),
            sequence: self.next_connection.clone(),
        };
        self.admission.open(IngressKind::P2pWork);
        let task = tokio::spawn(p2p::accept_loop(shared.clone(), context));
        let relay_url = Arc::new(Mutex::new(advertised));
        let restart_after_unix =
            Arc::new(AtomicU64::new(first_ticket.as_ref().map_or(0, |ticket| {
                ticket
                    .expires_unix
                    .saturating_sub(RESTART_BEFORE_EXPIRY_SECS)
            })));
        let keeper = match (account, &relay_account) {
            (Some(stored), Some(status)) => Some(tokio::spawn(keep_relay_ticket(
                root,
                stored.session,
                **shared.secret,
                shared.endpoint.clone(),
                first_ticket,
                status.clone(),
                relay_url.clone(),
                restart_after_unix.clone(),
            ))),
            _ => None,
        };
        self.p2p = Some(P2pGeneration {
            shared,
            cancel,
            task,
            relay_url,
            relay_account,
            keeper,
            restart_after_unix,
        });
        self.p2p_effective = RemoteAccessState::Listening;
        Ok(())
    }
    async fn stop_p2p(&mut self) {
        if let Some(p2p) = self.p2p.take() {
            self.p2p_effective = RemoteAccessState::Stopping;
            self.pairing
                .lock()
                .close(PairRejectReason::InvitationClosed);
            self.admission.close(IngressKind::P2pWork);
            if let Some(keeper) = &p2p.keeper {
                keeper.abort();
            }
            p2p.cancel.send_replace(true);
            let _ = p2p.task.await;
        }
        self.p2p_effective = RemoteAccessState::Disabled;
    }
    /// Rebuilds the P2P endpoint with the current settings and relay account.
    async fn restart_p2p(&mut self) {
        let relay_url = self.store.lock().settings().p2p.relay_url.clone();
        self.stop_p2p().await;
        if let Err(error) = self.start_p2p(relay_url).await {
            self.p2p_failed(error.message);
        }
    }
    /// Registers the Host's own key with its account and gets it a ticket.
    async fn fetch_ticket(
        &self,
        session: &ControlSession,
        secret: &[u8; 32],
    ) -> Result<RelayTicket, yttt_control_client::ControlError> {
        let label: String = format!("yttt Host {}", self.context.identity.environment_id)
            .chars()
            .take(64)
            .collect();
        session.register_device(secret, &label).await?;
        session.ticket(secret).await
    }
    async fn stop_network(&mut self) {
        if let Some(network) = self.network.take() {
            self.effective = RemoteAccessState::Stopping;
            self.admission.close(IngressKind::TlsWork);
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
        let credential = Arc::new(Credential::SharedSecret(AuthToken::from_bytes(
            credentials.work_secret,
        )));
        let (cancel, stop) = watch::channel(false);
        let mut context = self.context.clone();
        context.identity.ingress = IngressKind::TlsWork;
        context.identity.credential_generation = credentials.generation;
        context.stop = stop;
        self.admission.open(IngressKind::TlsWork);
        let task = tokio::spawn(network_loop(
            listener,
            credential,
            context,
            self.next_connection.clone(),
            self.streams.clone(),
        ));
        self.network = Some(NetworkGeneration {
            address,
            cancel,
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
    credential: Arc<Credential>,
    context: ConnectionContext,
    sequence: Arc<AtomicU64>,
    streams: Arc<Semaphore>,
) {
    let handshakes = Arc::new(Semaphore::new(MAX_PENDING_HANDSHAKES));
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
                let credential = credential.clone();
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
                        let _ = super::serve_connection(Box::new(stream), credential, connection).await;
                    }
                    let authenticated = permit.lock().is_none();
                    Some((peer, authenticated))
                });
            }
        }
    }
    drop(listener);
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
        if is_shared_address(address) {
            return Some(now);
        }
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
        if is_shared_address(address) {
            return;
        }
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

/// Loopback stands for every peer a port forward or tunnel relays, so its
/// failures cannot single out the misbehaving one. Such peers are bounded
/// only by the pending-handshake limit and the handshake deadline.
fn is_shared_address(address: IpAddr) -> bool {
    address.to_canonical().is_loopback()
}

fn failure(message: impl Into<String>) -> ProtocolFailure {
    ProtocolFailure::new(FailureCode::InvalidRequest, message, false)
}
fn io_failure(error: io::Error) -> ProtocolFailure {
    ProtocolFailure::new(FailureCode::Internal, error.to_string(), false)
}

/// Whether a pairing code should name this address. Loopback and link-local
/// addresses cannot reach another machine; leaving them out keeps the code
/// short and reveals less about this computer's network.
fn advertisable(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => !(ip.is_loopback() || ip.is_link_local() || ip.is_unspecified()),
        IpAddr::V6(ip) => {
            !(ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xffc0) == 0xfe80)
        }
    }
}

/// Keeps the Host's relay ticket fresh, caches each one for restarts, and
/// schedules the endpoint rebuild its first ticket requires.
#[allow(clippy::too_many_arguments)]
async fn keep_relay_ticket(
    root: std::path::PathBuf,
    session: ControlSession,
    secret: [u8; 32],
    endpoint: IrohEndpoint,
    first: Option<RelayTicket>,
    status: Arc<Mutex<RelayAccountStatus>>,
    relay_url: Arc<Mutex<Option<String>>>,
    restart_after_unix: Arc<AtomicU64>,
) {
    // Without a ticket yet, renew right away.
    let current = first.unwrap_or_else(|| RelayTicket {
        relay_url: String::new(),
        node_id: String::new(),
        token: String::new(),
        received_unix: 0,
        expires_unix: 0,
    });
    let failed_status = status.clone();
    let cached = session.clone();
    yttt_control_client::keep_ticket(
        session,
        secret,
        endpoint,
        current,
        move |ticket| {
            let stored = StoredRelayAccount {
                session: cached.clone(),
                ticket: Some(ticket.clone()),
            };
            if let Err(error) = p2p::save_relay_account(&root, &stored) {
                eprintln!("could not cache the relay ticket: {error}");
            }
            {
                let mut status = status.lock();
                status.relay_url = Some(ticket.relay_url.clone());
                status.ticket_expires_unix = Some(ticket.expires_unix);
                status.error = None;
            }
            *relay_url.lock() = Some(ticket.relay_url.clone());
            // The first ticket this endpoint's relay connection presents.
            let _ = restart_after_unix.compare_exchange(
                0,
                ticket
                    .expires_unix
                    .saturating_sub(RESTART_BEFORE_EXPIRY_SECS),
                Ordering::Relaxed,
                Ordering::Relaxed,
            );
        },
        move |error| {
            failed_status.lock().error = Some(format!("relay account: {error}"));
        },
    )
    .await;
}

#[cfg(test)]
mod advertise_tests {
    use super::advertisable;

    #[test]
    fn pairing_codes_name_only_addresses_another_machine_can_dial() {
        for address in [
            "192.168.1.20",
            "10.0.0.5",
            "203.0.113.7",
            "2001:db8::20",
            "fd00::1",
        ] {
            assert!(advertisable(address.parse().unwrap()), "{address}");
        }
        for address in [
            "127.0.0.1",
            "169.254.10.1",
            "0.0.0.0",
            "::1",
            "fe80::1",
            "::",
        ] {
            assert!(!advertisable(address.parse().unwrap()), "{address}");
        }
    }
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
    fn loopback_peers_are_never_delayed_or_refused() {
        // Everything a tunnel forwards arrives from loopback, so its failures
        // say nothing about the next peer.
        let mut penalties = HandshakePenalties::default();
        let now = tokio::time::Instant::now();
        for loopback in [
            IpAddr::from([127, 0, 0, 1]),
            IpAddr::from(std::net::Ipv6Addr::LOCALHOST),
            IpAddr::from(std::net::Ipv4Addr::LOCALHOST.to_ipv6_mapped()),
        ] {
            for _ in 0..8 {
                penalties.record_failure(loopback, now);
            }
            assert_eq!(penalties.admit(loopback, now), Some(now));
            assert_eq!(penalties.admit(loopback, now), Some(now));
        }
        assert!(penalties.addresses.is_empty());
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
