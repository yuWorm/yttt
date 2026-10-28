#![forbid(unsafe_code)]

mod diagnostics;
mod mirror;

use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use diagnostics::ClientPipelineDiagnostics;
pub use diagnostics::{ClientLatencyDiagnosticsSnapshot, ClientPipelineDiagnosticsSnapshot};
pub use mirror::{MirrorApply, TerminalMirror, TerminalMirrorMetadata};
use parking_lot::{Mutex, RwLock};
use tokio::{
    io::split,
    sync::{Semaphore, broadcast, mpsc, oneshot, watch},
};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::{
    ClientRequest, ConnectionChannel, ControlMessage, HostEvent, HostResponse, ProtocolFailure,
    Request, ResourceCatalog, Response, ServerEvent, TerminalInteractiveMessage,
    agent::{AgentSnapshotCursor, AgentSnapshotUpdate, ProjectTokenUsage},
    terminal::{TerminalInput, TerminalProcessState, TerminalStreamUpdate},
};
use yttt_transport::{
    AuthToken, AuthenticatedHost, ClientIdentity, HandshakeError, SharedConnector,
    TransportConnector, TransportError, TransportStream, client_handshake, receive_control,
    receive_control_observed, receive_state_event, receive_terminal_interactive, send_control,
    send_terminal_interactive,
};

const COMMAND_CAPACITY: usize = 256;
const EVENT_CAPACITY: usize = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONTROL_RESPONSE_GRACE: Duration = Duration::from_millis(100);
const INITIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(5);
const HEARTBEAT_TIMEOUT: Duration = Duration::from_secs(15);

struct AbortOnDrop<T>(tokio::task::JoinHandle<T>);
impl<T> Drop for AbortOnDrop<T> {
    fn drop(&mut self) {
        self.0.abort();
    }
}

struct ShutdownOnDrop(watch::Sender<bool>);
impl Drop for ShutdownOnDrop {
    fn drop(&mut self) {
        self.0.send_replace(true);
    }
}

fn retry_delay(attempt: u32, identity: &ClientIdentity) -> Duration {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    identity.session_nonce.0.hash(&mut hash);
    identity.terminal_session_id.hash(&mut hash);
    attempt.hash(&mut hash);
    let ceiling = (100_u64 << attempt.min(6)).min(5_000);
    Duration::from_millis(ceiling / 2 + hash.finish() % (ceiling / 2 + 1))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    Disconnected,
    Connecting,
    Ready {
        host_epoch: u64,
        connection_sequence: u64,
    },
    Reconnecting {
        attempt: u32,
        message: String,
    },
    HostLost {
        message: String,
    },
}

#[derive(Clone, Debug)]
pub enum ClientEvent {
    Connection(ConnectionState),
    Server(Box<HostEvent>),
    TerminalUpdated(Arc<TerminalStreamUpdate>),
    TerminalUnavailable(TerminalSessionId),
    ResourceCatalogUpdated(Arc<ResourceCatalog>),
    AgentSnapshotsReset { host_epoch: u64 },
    AgentSnapshotUpdated(Box<AgentSnapshotUpdate>),
    ProjectTokenUsageUpdated(Box<ProjectTokenUsage>),
}

struct TerminalDataChannel {
    generation: u64,
    desired: watch::Sender<bool>,
}

#[derive(Default)]
struct TerminalState {
    host_epoch: Option<u64>,
    next_worker_generation: u64,
    mirrors: HashMap<TerminalSessionId, TerminalMirror>,
    known_sessions: HashSet<TerminalSessionId>,
    data_channels: HashMap<TerminalSessionId, TerminalDataChannel>,
    /// `TerminalExit` events held until the data lane delivers the final frame
    /// or stops, so observers never see an exit before its last viewport.
    deferred_exits: HashMap<TerminalSessionId, HostEvent>,
}

impl TerminalState {
    /// Releases a deferred exit once nothing can still deliver its final frame.
    fn take_ready_exit(&mut self, session_id: &TerminalSessionId) -> Option<HostEvent> {
        let event = self.deferred_exits.get(session_id)?;
        let ServerEvent::TerminalExit {
            session_epoch,
            final_sequence,
            ..
        } = &event.body
        else {
            return self.deferred_exits.remove(session_id);
        };
        if data_lane_open(self, session_id)
            && !final_viewport_is_ready(self, session_id, *session_epoch, *final_sequence)
        {
            return None;
        }
        self.deferred_exits.remove(session_id)
    }
}

fn final_viewport_is_ready(
    state: &TerminalState,
    session_id: &TerminalSessionId,
    session_epoch: u64,
    final_sequence: u64,
) -> bool {
    state.mirrors.get(session_id).is_some_and(|mirror| {
        let viewport = mirror.viewport();
        viewport.session_epoch == session_epoch
            && viewport.sequence >= final_sequence
            && matches!(viewport.process_state, TerminalProcessState::Exited { .. })
    })
}

/// The union of terminals the client's windows render, reported to the Host so
/// hidden terminals stop streaming frames.
#[derive(Clone, Debug, PartialEq, Eq)]
struct VisibleTerminals {
    revision: u64,
    sessions: Vec<TerminalSessionId>,
}

#[derive(Default)]
struct VisibilityScopes {
    scopes: HashMap<u64, HashSet<TerminalSessionId>>,
    revision: u64,
}

#[derive(Default)]
struct LocalControlRecovery {
    enabled: AtomicBool,
    eligible: AtomicBool,
}

impl LocalControlRecovery {
    fn observe(
        &self,
        status: &yttt_protocol::session::ControlStatus,
        client_id: &yttt_core::model::ids::ClientInstanceId,
    ) {
        self.eligible
            .store(status.owner.as_ref() == Some(client_id), Ordering::Release);
    }

    fn can_recover(&self) -> bool {
        self.enabled.load(Ordering::Acquire) && self.eligible.load(Ordering::Acquire)
    }
}

fn should_reacquire_local_control(
    local_control_recovery: &LocalControlRecovery,
    was_controller: bool,
    status: &yttt_protocol::session::ControlStatus,
) -> bool {
    local_control_recovery.can_recover()
        && was_controller
        && status.owner.is_none()
        && status.transfer.is_none()
}

pub struct ClientCore {
    inner: Arc<ClientCoreInner>,
}

struct ClientCoreInner {
    commands: mpsc::Sender<ClientCommand>,
    interactive_commands: mpsc::Sender<ClientCommand>,
    events: broadcast::Sender<ClientEvent>,
    state: watch::Receiver<ConnectionState>,
    terminal_state: Arc<RwLock<TerminalState>>,
    supervisor: Mutex<Option<tokio::task::JoinHandle<()>>>,
    catalog: Arc<RwLock<Option<Arc<ResourceCatalog>>>>,
    agent_snapshots: Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>,
    project_token_usage: Arc<RwLock<HashMap<String, ProjectTokenUsage>>>,
    next_request_id: Arc<AtomicU64>,
    shutdown: watch::Sender<bool>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
    control: Arc<RwLock<Option<yttt_protocol::session::ControlStatus>>>,
    local_control_recovery: Arc<LocalControlRecovery>,
    client_id: yttt_core::model::ids::ClientInstanceId,
    visibility_scopes: Mutex<VisibilityScopes>,
    visibility: watch::Sender<Option<Arc<VisibleTerminals>>>,
}

impl Drop for ClientCoreInner {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for channel in self.terminal_state.read().data_channels.values() {
            let _ = channel.desired.send(false);
        }
        if let Some(supervisor) = self.supervisor.lock().take() {
            supervisor.abort();
        }
    }
}

struct ClientCommand {
    request_id: Option<u64>,
    body: Request,
    reply: Option<oneshot::Sender<Result<Response, ClientCoreError>>>,
    control: Option<yttt_protocol::session::ControlContext>,
    deadline: tokio::time::Instant,
}

struct TerminalWorkerFence {
    session_id: TerminalSessionId,
    host_epoch: u64,
    generation: u64,
}

enum PendingTerminalStops {
    None,
    One(TerminalWorkerFence),
    Many(Vec<TerminalWorkerFence>),
}

impl PendingTerminalStops {
    fn stop_all(&self, terminal_state: &RwLock<TerminalState>) {
        match self {
            Self::None => {}
            Self::One(fence) => stop_terminal_data_channel_if_current(
                terminal_state,
                &fence.session_id,
                fence.host_epoch,
                fence.generation,
            ),
            Self::Many(fences) => {
                for fence in fences {
                    stop_terminal_data_channel_if_current(
                        terminal_state,
                        &fence.session_id,
                        fence.host_epoch,
                        fence.generation,
                    );
                }
            }
        }
    }

    fn stop_for(&self, terminal_state: &RwLock<TerminalState>, session_id: &TerminalSessionId) {
        match self {
            Self::None => {}
            Self::One(fence) if fence.session_id == *session_id => {
                stop_terminal_data_channel_if_current(
                    terminal_state,
                    &fence.session_id,
                    fence.host_epoch,
                    fence.generation,
                );
            }
            Self::One(_) => {}
            Self::Many(fences) => {
                for fence in fences
                    .iter()
                    .filter(|fence| fence.session_id == *session_id)
                {
                    stop_terminal_data_channel_if_current(
                        terminal_state,
                        &fence.session_id,
                        fence.host_epoch,
                        fence.generation,
                    );
                }
            }
        }
    }
}

pub struct PendingClientRequest {
    response: oneshot::Receiver<Result<Response, ClientCoreError>>,
    deadline: tokio::time::Instant,
    terminal_stops: PendingTerminalStops,
    terminal_state: Arc<RwLock<TerminalState>>,
}

impl PendingClientRequest {
    pub async fn wait(self) -> Result<Response, ClientCoreError> {
        let response = tokio::time::timeout_at(self.deadline, self.response)
            .await
            .map_err(|_| ClientCoreError::RequestTimeout)?
            .map_err(|_| ClientCoreError::SupervisorStopped)??;
        if matches!(
            &response,
            Response::TerminalDetached
                | Response::TerminalTerminated(_)
                | Response::TerminalExitAcknowledged
        ) {
            self.terminal_stops.stop_all(&self.terminal_state);
        }
        if let Response::TerminalsTerminated { results } = &response {
            for result in results.iter().filter(|result| result.result.is_ok()) {
                self.terminal_stops
                    .stop_for(&self.terminal_state, &result.session_id);
            }
        }
        Ok(response)
    }
}

enum PendingRequest {
    User(oneshot::Sender<Result<Response, ClientCoreError>>),
    Catalog,
    AgentSnapshots,
    Visibility,
}

struct ClientSessionContext {
    data_connects: Arc<Semaphore>,
    catalog_requests: mpsc::Sender<()>,
    events: broadcast::Sender<ClientEvent>,
    terminal_state: Arc<RwLock<TerminalState>>,
    connector: SharedConnector,
    identity: ClientIdentity,
    host_epoch: u64,
    current_host_epoch: Arc<RwLock<Option<u64>>>,
    token: AuthToken,
    catalog: Arc<RwLock<Option<Arc<ResourceCatalog>>>>,
    agent_snapshots: Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>,
    project_token_usage: Arc<RwLock<HashMap<String, ProjectTokenUsage>>>,
    next_request_id: Arc<AtomicU64>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
    visibility: watch::Receiver<Option<Arc<VisibleTerminals>>>,
    shutdown: watch::Receiver<bool>,
    control: Arc<RwLock<Option<yttt_protocol::session::ControlStatus>>>,
    local_control_recovery: Arc<LocalControlRecovery>,
}

impl ClientCore {
    pub async fn connect(
        connector: impl TransportConnector,
        identity: ClientIdentity,
        token: AuthToken,
    ) -> Result<Self, ClientCoreError> {
        let connector = SharedConnector::new(connector);
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (interactive_tx, interactive_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (state_tx, state_rx) = watch::channel(ConnectionState::Connecting);
        let terminal_state = Arc::new(RwLock::new(TerminalState::default()));
        let (shutdown, _) = watch::channel(false);
        let host_epoch = Arc::new(RwLock::new(None));
        let catalog = Arc::new(RwLock::new(None));
        let agent_snapshots = Arc::new(RwLock::new(HashMap::new()));
        let project_token_usage = Arc::new(RwLock::new(HashMap::new()));
        let next_request_id = Arc::new(AtomicU64::new(1));
        let diagnostics = Arc::new(ClientPipelineDiagnostics::default());
        let control = Arc::new(RwLock::new(None));
        let local_control_recovery = Arc::new(LocalControlRecovery::default());
        let (visibility, visibility_rx) = watch::channel(None);
        let (initial_tx, initial_rx) = oneshot::channel();
        let supervisor = tokio::spawn(run_supervisor(
            connector.clone(),
            identity.clone(),
            token.clone(),
            command_rx,
            interactive_rx,
            visibility_rx,
            events.clone(),
            state_tx,
            terminal_state.clone(),
            host_epoch,
            catalog.clone(),
            agent_snapshots.clone(),
            project_token_usage.clone(),
            next_request_id.clone(),
            diagnostics.clone(),
            shutdown.clone(),
            initial_tx,
            control.clone(),
            local_control_recovery.clone(),
        ));
        let inner = Arc::new(ClientCoreInner {
            commands: command_tx,
            interactive_commands: interactive_tx,
            events,
            state: state_rx,
            terminal_state,
            catalog,
            next_request_id,
            agent_snapshots,
            project_token_usage,
            shutdown,
            diagnostics,
            control,
            local_control_recovery,
            client_id: identity.client_instance_id.clone(),
            visibility_scopes: Mutex::default(),
            visibility,
            supervisor: Mutex::new(Some(supervisor)),
        });
        let initial = tokio::time::timeout(INITIAL_CONNECT_TIMEOUT, initial_rx)
            .await
            .map_err(|_| ClientCoreError::ConnectionTimeout)?
            .map_err(|_| ClientCoreError::SupervisorStopped)?;
        initial.map_err(ClientCoreError::Connection)?;
        Ok(Self { inner })
    }

    pub fn state(&self) -> ConnectionState {
        self.inner.state.borrow().clone()
    }

    pub fn client_id(&self) -> &yttt_core::model::ids::ClientInstanceId {
        &self.inner.client_id
    }

    pub fn control_status(&self) -> Option<yttt_protocol::session::ControlStatus> {
        self.inner.control.read().clone()
    }

    pub fn is_controller(&self) -> bool {
        self.inner
            .control
            .read()
            .as_ref()
            .is_some_and(|status| status.owner.as_ref() == Some(self.client_id()))
    }

    /// Permits this local client to recover control only after it reconnects as
    /// the prior controller and the Host still has no owner or transfer.
    pub fn enable_local_control_recovery(&self) {
        self.inner
            .local_control_recovery
            .enabled
            .store(true, Ordering::Release);
        if let Some(status) = self.inner.control.read().as_ref() {
            self.inner
                .local_control_recovery
                .observe(status, self.client_id());
        }
    }

    /// Records the terminals one window (`scope`) currently renders. The Host
    /// streams frames only for the union across scopes once any scope reports;
    /// clients that never report receive every attached terminal.
    pub fn set_visible_terminals(
        &self,
        scope: u64,
        sessions: impl IntoIterator<Item = TerminalSessionId>,
    ) {
        let sessions = sessions.into_iter().collect::<HashSet<_>>();
        let mut scopes = self.inner.visibility_scopes.lock();
        if scopes.scopes.get(&scope) == Some(&sessions) {
            return;
        }
        scopes.scopes.insert(scope, sessions);
        self.publish_visibility(&mut scopes);
    }

    /// Withdraws a window's terminals, for example when it closes.
    pub fn clear_visible_terminals(&self, scope: u64) {
        let mut scopes = self.inner.visibility_scopes.lock();
        if scopes.scopes.remove(&scope).is_some() {
            self.publish_visibility(&mut scopes);
        }
    }

    fn publish_visibility(&self, scopes: &mut VisibilityScopes) {
        let mut sessions = scopes
            .scopes
            .values()
            .flatten()
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect::<Vec<_>>();
        sessions.sort_unstable_by(|left, right| left.as_str().cmp(right.as_str()));
        if self
            .inner
            .visibility
            .borrow()
            .as_ref()
            .is_some_and(|current| current.sessions == sessions)
        {
            return;
        }
        scopes.revision += 1;
        self.inner
            .visibility
            .send_replace(Some(Arc::new(VisibleTerminals {
                revision: scopes.revision,
                sessions,
            })));
    }

    pub fn diagnostics(&self) -> ClientPipelineDiagnosticsSnapshot {
        self.inner.diagnostics.snapshot()
    }

    pub fn subscribe_state(&self) -> watch::Receiver<ConnectionState> {
        self.inner.state.clone()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<ClientEvent> {
        self.inner.events.subscribe()
    }

    pub fn terminal_snapshot(
        &self,
        session_id: &TerminalSessionId,
    ) -> Option<yttt_protocol::terminal::SemanticViewport> {
        self.inner
            .terminal_state
            .read()
            .mirrors
            .get(session_id)
            .map(|mirror| mirror.viewport().clone())
    }

    pub fn terminal_metadata(
        &self,
        session_id: &TerminalSessionId,
    ) -> Option<TerminalMirrorMetadata> {
        self.inner
            .terminal_state
            .read()
            .mirrors
            .get(session_id)
            .map(TerminalMirror::metadata)
    }

    pub fn terminal_snapshots(&self) -> Vec<yttt_protocol::terminal::SemanticViewport> {
        let terminal_state = self.inner.terminal_state.read();
        let mut snapshots = terminal_state
            .mirrors
            .values()
            .map(|mirror| mirror.viewport().clone())
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.session_id.as_str().cmp(right.session_id.as_str()));
        snapshots
    }

    pub fn known_terminal_ids(&self) -> Vec<TerminalSessionId> {
        self.inner
            .terminal_state
            .read()
            .known_sessions
            .iter()
            .cloned()
            .collect()
    }

    pub fn resource_catalog(&self) -> Option<Arc<ResourceCatalog>> {
        self.inner.catalog.read().clone()
    }

    pub fn agent_snapshots(&self) -> Vec<AgentSnapshotUpdate> {
        let snapshots = self.inner.agent_snapshots.read();
        let mut snapshots = snapshots.values().cloned().collect::<Vec<_>>();
        snapshots.sort_by(|left, right| {
            agent_address_key(&left.scope).cmp(&agent_address_key(&right.scope))
        });
        snapshots
    }

    pub fn project_token_usage(&self, project_id: &str) -> Option<ProjectTokenUsage> {
        self.inner
            .project_token_usage
            .read()
            .get(project_id)
            .cloned()
    }

    pub fn enqueue_request(&self, body: Request) -> Result<PendingClientRequest, ClientCoreError> {
        if !matches!(self.state(), ConnectionState::Ready { .. }) {
            return Err(ClientCoreError::NotConnected);
        }
        let timeout = Self::request_timeout(&body);
        let terminal_stops = terminal_stops_for_request(&self.inner.terminal_state.read(), &body);
        let (reply, response) = oneshot::channel();
        self.enqueue_command(body, Some(reply))?;
        Ok(PendingClientRequest {
            response,
            deadline: tokio::time::Instant::now() + timeout,
            terminal_stops,
            terminal_state: self.inner.terminal_state.clone(),
        })
    }

    /// Queue terminal input on the dedicated interactive stream without
    /// allocating a per-input response waiter.
    pub fn send_terminal_input(&self, input: TerminalInput) -> Result<(), ClientCoreError> {
        self.enqueue_command(Request::TerminalInput(input), None)
    }

    fn enqueue_command(
        &self,
        body: Request,
        reply: Option<oneshot::Sender<Result<Response, ClientCoreError>>>,
    ) -> Result<(), ClientCoreError> {
        if !matches!(self.state(), ConnectionState::Ready { .. }) {
            return Err(ClientCoreError::NotConnected);
        }
        let interactive = matches!(
            &body,
            Request::TerminalInput(_) | Request::ResizeTerminal(_) | Request::ScrollTerminal(_)
        );
        let request_id = reply
            .as_ref()
            .map(|_| self.inner.next_request_id.fetch_add(1, Ordering::Relaxed));
        let sender = if interactive {
            &self.inner.interactive_commands
        } else {
            &self.inner.commands
        };
        let deadline = tokio::time::Instant::now() + Self::request_timeout(&body);
        sender
            .try_send(ClientCommand {
                deadline,
                request_id,
                body,
                reply,
                control: self
                    .inner
                    .control
                    .read()
                    .as_ref()
                    .map(|status| status.context),
            })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ClientCoreError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => ClientCoreError::SupervisorStopped,
            })
    }

    pub async fn request(&self, body: Request) -> Result<Response, ClientCoreError> {
        self.enqueue_request(body)?.wait().await
    }

    pub async fn shutdown(&self) {
        let _ = self.inner.shutdown.send(true);
        for channel in self.inner.terminal_state.read().data_channels.values() {
            let _ = channel.desired.send(false);
        }
        let supervisor = self.inner.supervisor.lock().take();
        if let Some(supervisor) = supervisor {
            let _ = supervisor.await;
        }
    }

    fn request_timeout(request: &Request) -> Duration {
        match request {
            Request::RemoteFile(_) | Request::RemoteCommand(_) => REMOTE_REQUEST_TIMEOUT,
            _ => REQUEST_TIMEOUT,
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_supervisor(
    connector: SharedConnector,
    mut identity: ClientIdentity,
    token: AuthToken,
    mut commands: mpsc::Receiver<ClientCommand>,
    mut interactive_commands: mpsc::Receiver<ClientCommand>,
    visibility: watch::Receiver<Option<Arc<VisibleTerminals>>>,
    events: broadcast::Sender<ClientEvent>,
    state: watch::Sender<ConnectionState>,
    terminal_state: Arc<RwLock<TerminalState>>,
    host_epoch: Arc<RwLock<Option<u64>>>,
    catalog: Arc<RwLock<Option<Arc<ResourceCatalog>>>>,
    agent_snapshots: Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>,
    project_token_usage: Arc<RwLock<HashMap<String, ProjectTokenUsage>>>,
    next_request_id: Arc<AtomicU64>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
    shutdown_tx: watch::Sender<bool>,
    initial: oneshot::Sender<Result<(), String>>,
    profile_control: Arc<RwLock<Option<yttt_protocol::session::ControlStatus>>>,
    local_control_recovery: Arc<LocalControlRecovery>,
) {
    let _shutdown_guard = ShutdownOnDrop(shutdown_tx.clone());
    let mut shutdown = shutdown_tx.subscribe();
    let mut initial_attempt = 0;
    let mut lanes = loop {
        set_state(&state, &events, ConnectionState::Connecting);
        diagnostics.record_connection_attempt();
        let result = tokio::select! {
            result = establish_session_lanes(&connector, &identity, &token) => result,
            _ = shutdown.wait_for(|stopped| *stopped) => return,
        };
        match result {
            Ok(lanes) => break lanes,
            Err(ConnectFailure::Retry(message)) => {
                initial_attempt += 1;
                set_state(
                    &state,
                    &events,
                    ConnectionState::Reconnecting {
                        attempt: initial_attempt,
                        message,
                    },
                );
                tokio::select! {
                    _ = tokio::time::sleep(retry_delay(initial_attempt, &identity)) => {},
                    _ = shutdown.wait_for(|stopped| *stopped) => return,
                }
            }
            Err(ConnectFailure::Fatal(message)) => {
                set_state(
                    &state,
                    &events,
                    ConnectionState::HostLost {
                        message: message.clone(),
                    },
                );
                let _ = initial.send(Err(message));
                return;
            }
        }
    };
    let epoch_state = HostEpochState {
        current_host_epoch: &host_epoch,
        terminal_state: &terminal_state,
        catalog: &catalog,
        agent_snapshots: &agent_snapshots,
        project_token_usage: &project_token_usage,
        profile_control: &profile_control,
        events: &events,
    };
    install_host_epoch(lanes.host.host_epoch, &epoch_state);
    identity.host_epoch_hint = Some(lanes.host.host_epoch);
    update_profile_control(
        &profile_control,
        &lanes.control_status,
        &host_epoch,
        lanes.host.host_epoch,
        &identity.client_instance_id,
        &local_control_recovery,
    );
    set_state(
        &state,
        &events,
        ConnectionState::Ready {
            host_epoch: lanes.host.host_epoch,
            connection_sequence: lanes.host.connection_sequence,
        },
    );
    let _ = initial.send(Ok(()));
    let (catalog_requests, mut catalog_rx) = mpsc::channel(1);
    let mut attempt = 0_u32;
    let data_connects = Arc::new(Semaphore::new(4));
    loop {
        let session = ClientSessionContext {
            data_connects: data_connects.clone(),
            catalog_requests: catalog_requests.clone(),
            events: events.clone(),
            terminal_state: terminal_state.clone(),
            connector: connector.clone(),
            identity: identity.clone(),
            host_epoch: lanes.host.host_epoch,
            current_host_epoch: host_epoch.clone(),
            token: token.clone(),
            catalog: catalog.clone(),
            agent_snapshots: agent_snapshots.clone(),
            project_token_usage: project_token_usage.clone(),
            next_request_id: next_request_id.clone(),
            diagnostics: diagnostics.clone(),
            visibility: visibility.clone(),
            shutdown: shutdown.clone(),
            control: profile_control.clone(),
            local_control_recovery: local_control_recovery.clone(),
        };
        let disconnected = {
            let control =
                connected_control_session(lanes.control, &mut commands, &mut catalog_rx, &session);
            let interactive = connected_interactive_session(
                lanes.interactive,
                &mut interactive_commands,
                &session,
            );
            let state_events = connected_state_event_session(lanes.state_events, &session);
            tokio::pin!(control, interactive, state_events);
            let (control_exited, lane_result) = tokio::select! {
                biased;
                result = &mut control => (true, result.map(|message| format!("control: {message}"))),
                result = &mut interactive => {
                    (false, result.map(|message| format!("interactive: {message}")))
                },
                result = &mut state_events => {
                    (false, result.map(|message| format!("state events: {message}")))
                },
                _ = shutdown.wait_for(|stopped| *stopped) => (true, None),
            };
            if control_exited {
                lane_result
            } else {
                tokio::select! {
                    biased;
                    result = &mut control => result,
                    _ = tokio::time::sleep(CONTROL_RESPONSE_GRACE) => lane_result,
                }
            }
        };
        let Some(message) = disconnected else {
            set_state(&state, &events, ConnectionState::Disconnected);
            return;
        };
        let was_controller = profile_control.read().as_ref().is_some_and(|status| {
            status.owner.as_ref() == Some(&identity.client_instance_id) && status.transfer.is_none()
        });
        attempt = attempt.saturating_add(1);
        diagnostics.record_reconnect();
        *profile_control.write() = None;
        while let Ok(command) = commands.try_recv() {
            if let Some(reply) = command.reply {
                let _ = reply.send(Err(ClientCoreError::NotConnected));
            }
        }
        while let Ok(command) = interactive_commands.try_recv() {
            if let Some(reply) = command.reply {
                let _ = reply.send(Err(ClientCoreError::NotConnected));
            }
        }
        set_state(
            &state,
            &events,
            ConnectionState::Reconnecting {
                attempt,
                message: message.clone(),
            },
        );
        let delay = retry_delay(attempt, &identity);
        tokio::select! {
            _ = tokio::time::sleep(delay) => {}
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    set_state(&state, &events, ConnectionState::Disconnected);
                    return;
                }
            }
        }
        loop {
            set_state(&state, &events, ConnectionState::Connecting);
            diagnostics.record_connection_attempt();
            let result = tokio::select! {
                result = establish_session_lanes(&connector, &identity, &token) => result,
                _ = shutdown.wait_for(|stopped| *stopped) => {
                    set_state(&state, &events, ConnectionState::Disconnected);
                    return;
                }
            };
            let result = match result {
                Ok(mut new_lanes) => {
                    install_host_epoch(new_lanes.host.host_epoch, &epoch_state);
                    if should_reacquire_local_control(
                        &local_control_recovery,
                        was_controller,
                        &new_lanes.control_status,
                    ) {
                        reacquire_local_control(
                            &mut new_lanes.control,
                            &identity,
                            new_lanes.control_status.context,
                        )
                        .await
                        .map(|status| {
                            new_lanes.control_status = status;
                            new_lanes
                        })
                    } else {
                        Ok(new_lanes)
                    }
                }
                result => result,
            };
            match result {
                Ok(new_lanes) => {
                    lanes = new_lanes;
                    identity.host_epoch_hint = Some(lanes.host.host_epoch);
                    update_profile_control(
                        &profile_control,
                        &lanes.control_status,
                        &host_epoch,
                        lanes.host.host_epoch,
                        &identity.client_instance_id,
                        &local_control_recovery,
                    );
                    attempt = 0;
                    set_state(
                        &state,
                        &events,
                        ConnectionState::Ready {
                            host_epoch: lanes.host.host_epoch,
                            connection_sequence: lanes.host.connection_sequence,
                        },
                    );
                    break;
                }
                Err(ConnectFailure::Retry(message)) => {
                    attempt = attempt.saturating_add(1);
                    set_state(
                        &state,
                        &events,
                        ConnectionState::Reconnecting { attempt, message },
                    );
                    let delay = retry_delay(attempt, &identity);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() {
                                set_state(&state, &events, ConnectionState::Disconnected);
                                return;
                            }
                        }
                    }
                }
                Err(ConnectFailure::Fatal(message)) => {
                    set_state(&state, &events, ConnectionState::HostLost { message });
                    return;
                }
            }
        }
    }
}

struct SessionLanes {
    control: TransportStream,
    interactive: TransportStream,
    state_events: TransportStream,
    host: AuthenticatedHost,
    control_status: yttt_protocol::session::ControlStatus,
}

async fn establish_session_lanes(
    connector: &SharedConnector,
    identity: &ClientIdentity,
    token: &AuthToken,
) -> Result<SessionLanes, ConnectFailure> {
    let (mut control, host) = establish(connector, identity, token).await?;
    send_control(
        &mut control,
        &ControlMessage::Request(ClientRequest::new(
            u64::MAX,
            Request::ProfileControl(yttt_protocol::session::ProfileControlRequest::Status),
        )),
    )
    .await
    .map_err(|error| ConnectFailure::Retry(error.to_string()))?;
    let status = tokio::time::timeout(REQUEST_TIMEOUT, receive_control(&mut control))
        .await
        .map_err(|_| ConnectFailure::Retry("Host control status timed out".into()))?
        .map_err(|error| ConnectFailure::Retry(error.to_string()))?;
    let ControlMessage::Response(HostResponse {
        result: Ok(Response::ProfileControl(control_status)),
        ..
    }) = status
    else {
        return Err(ConnectFailure::Fatal(
            "Host did not provide authenticated profile control state".to_string(),
        ));
    };
    let mut lane_identity = identity.clone();
    lane_identity.host_epoch_hint = Some(host.host_epoch);
    lane_identity.can_force_stop = false;
    lane_identity.terminal_session_id = None;
    lane_identity.channel = ConnectionChannel::TerminalInteractive;
    let (interactive, interactive_host) = establish(connector, &lane_identity, token).await?;
    lane_identity.channel = ConnectionChannel::StateEvents;
    let (state_events, state_host) = establish(connector, &lane_identity, token).await?;
    if interactive_host.host_epoch != host.host_epoch || state_host.host_epoch != host.host_epoch {
        return Err(ConnectFailure::Retry(
            "Host epoch changed while establishing resource channels".to_string(),
        ));
    }
    Ok(SessionLanes {
        control,
        interactive,
        state_events,
        host,
        control_status,
    })
}

async fn establish(
    connector: &SharedConnector,
    identity: &ClientIdentity,
    token: &AuthToken,
) -> Result<(TransportStream, AuthenticatedHost), ConnectFailure> {
    let mut stream = tokio::time::timeout(CONNECT_TIMEOUT, connector.connect())
        .await
        .map_err(|_| ConnectFailure::Retry("Host transport connection timed out".into()))?
        .map_err(|error| match error {
            TransportError::Permanent(message) => ConnectFailure::Fatal(message),
            error => ConnectFailure::Retry(error.to_string()),
        })?;
    let host = client_handshake(&mut stream, identity, token)
        .await
        .map_err(ConnectFailure::from)?;
    Ok((stream, host))
}

async fn reacquire_local_control(
    control: &mut TransportStream,
    identity: &ClientIdentity,
    context: yttt_protocol::session::ControlContext,
) -> Result<yttt_protocol::session::ControlStatus, ConnectFailure> {
    const REACQUIRE_REQUEST_ID: u64 = u64::MAX - 1;
    send_control(
        control,
        &ControlMessage::Request(ClientRequest {
            request_id: REACQUIRE_REQUEST_ID,
            actor_device_id: Some(identity.client_instance_id.to_string()),
            lease_epoch: None,
            control: Some(context),
            body: Request::ProfileControl(
                yttt_protocol::session::ProfileControlRequest::ReacquireIfUnowned { context },
            ),
        }),
    )
    .await
    .map_err(|error| ConnectFailure::Retry(error.to_string()))?;
    let response = tokio::time::timeout(REQUEST_TIMEOUT, receive_control(control))
        .await
        .map_err(|_| ConnectFailure::Retry("Host control reacquisition timed out".into()))?
        .map_err(|error| ConnectFailure::Retry(error.to_string()))?;
    let ControlMessage::Response(HostResponse {
        request_id: REACQUIRE_REQUEST_ID,
        result: Ok(Response::ProfileControl(status)),
    }) = response
    else {
        return Err(ConnectFailure::Fatal(
            "Host did not return profile control state after local recovery request".to_string(),
        ));
    };
    Ok(status)
}

async fn connected_control_session(
    stream: TransportStream,
    commands: &mut mpsc::Receiver<ClientCommand>,
    catalog_rx: &mut mpsc::Receiver<()>,
    context: &ClientSessionContext,
) -> Option<String> {
    let mut shutdown = context.shutdown.clone();
    let (mut reader, mut writer) = split(stream);
    let mut pending = HashMap::new();
    let mut visibility = context.visibility.clone();
    // The Host forgets a client's visibility when its connection drops, so every
    // control session reports the current set until the Host declines the request.
    let mut visibility_supported = true;
    visibility.mark_changed();
    let next_internal_request_id = || context.next_request_id.fetch_add(1, Ordering::Relaxed);
    if send_internal_request(
        &mut writer,
        &mut pending,
        &next_internal_request_id,
        Some(context.identity.client_instance_id.to_string()),
        Request::ListResources,
        PendingRequest::Catalog,
    )
    .await
    .is_err()
    {
        return Some("failed to request Host resource catalog".to_string());
    }
    let acknowledged = context
        .agent_snapshots
        .read()
        .values()
        .map(|update| AgentSnapshotCursor {
            scope: update.scope.clone(),
            host_epoch: update.host_epoch,
            sequence: update.sequence,
        })
        .collect();
    if send_internal_request(
        &mut writer,
        &mut pending,
        &next_internal_request_id,
        Some(context.identity.client_instance_id.to_string()),
        Request::ReadAgentSnapshots { acknowledged },
        PendingRequest::AgentSnapshots,
    )
    .await
    .is_err()
    {
        return Some("failed to request Host Agent snapshots".to_string());
    }
    let (control_messages_tx, mut control_messages_rx) = mpsc::channel(COMMAND_CAPACITY);
    let _control_reader = AbortOnDrop(tokio::spawn(async move {
        loop {
            let message = receive_control(&mut reader)
                .await
                .map_err(|error| error.to_string());
            let failed = message.is_err();
            if control_messages_tx.send(message).await.is_err() || failed {
                break;
            }
        }
    }));

    let mut cleanup = tokio::time::interval(Duration::from_secs(1));
    cleanup.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let disconnected = 'connected: loop {
        tokio::select! {
            _ = cleanup.tick() => {
                pending.retain(|_, request| !matches!(request, PendingRequest::User(reply) if reply.is_closed()));
            }
            command = commands.recv(), if pending.len() < COMMAND_CAPACITY => {
                let Some(ClientCommand {
                    request_id,
                    body,
                    reply,
                    control,
                    deadline,
                }) = command else {
                    break None;
                };
                if deadline <= tokio::time::Instant::now() || reply.as_ref().is_some_and(|reply| reply.is_closed()) {
                    if let Some(reply) = reply { let _ = reply.send(Err(ClientCoreError::RequestTimeout)); }
                    continue;
                }
                let Some(request_id) = request_id else {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(ClientCoreError::Connection(
                            "one-way command was routed to the control stream".to_string(),
                        )));
                    }
                    continue;
                };
                let message = ControlMessage::Request(ClientRequest {
                    request_id,
                    actor_device_id: Some(context.identity.client_instance_id.to_string()),
                    lease_epoch: None,
                    control,
                    body,
                });
                if let Err(error) = send_control(&mut writer, &message).await {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(ClientCoreError::Connection(error.to_string())));
                    }
                    break Some(error.to_string());
                }
                if let Some(reply) = reply {
                    pending.insert(request_id, PendingRequest::User(reply));
                }
            }
            message = control_messages_rx.recv() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => break Some(error),
                    None => break Some("Host control response reader stopped".to_string()),
                };
                match message {
                    ControlMessage::Response(response) => {
                        let Some(pending_request) = pending.remove(&response.request_id) else {
                            continue;
                        };
                        if matches!(
                            (&pending_request, &response.result),
                            (PendingRequest::Visibility, Err(failure))
                                if failure.code == yttt_protocol::FailureCode::Unsupported
                        ) {
                            visibility_supported = false;
                        }
                        let more_agents = matches!((&pending_request, &response.result),
                            (PendingRequest::AgentSnapshots, Ok(Response::AgentSnapshots { has_more: true, .. })));
                        handle_response(pending_request, response.result, context);
                        if more_agents {
                            let acknowledged = context.agent_snapshots.read().values().map(|update| AgentSnapshotCursor {
                                scope: update.scope.clone(), host_epoch: update.host_epoch, sequence: update.sequence,
                            }).collect();
                            if send_internal_request(
                                &mut writer, &mut pending, &next_internal_request_id,
                                Some(context.identity.client_instance_id.to_string()),
                                Request::ReadAgentSnapshots { acknowledged }, PendingRequest::AgentSnapshots,
                            ).await.is_err() {
                                break 'connected Some("failed to read next Agent snapshot page".to_string());
                            }
                        }
                    }
                    ControlMessage::Event(_)
                    | ControlMessage::Request(_)
                    | ControlMessage::TerminalInput(_) => {
                        break Some("Host sent a non-response message on the control stream".to_string());
                    }
                }
            }
            refresh = catalog_rx.recv() => {
                if refresh.is_none() {
                    break None;
                }
                if pending.values().any(|request| matches!(request, PendingRequest::Catalog)) {
                    continue;
                }
                if send_internal_request(
                    &mut writer,
                    &mut pending,
                    &next_internal_request_id,
                    Some(context.identity.client_instance_id.to_string()),
                    Request::ListResources,
                    PendingRequest::Catalog,
                )
                .await
                .is_err()
                {
                    break Some("failed to refresh Host resource catalog".to_string());
                }
            }
            changed = visibility.changed(), if visibility_supported => {
                if changed.is_err() {
                    visibility_supported = false;
                    continue;
                }
                let Some(visible) = visibility.borrow_and_update().clone() else {
                    continue;
                };
                if send_internal_request(
                    &mut writer,
                    &mut pending,
                    &next_internal_request_id,
                    Some(context.identity.client_instance_id.to_string()),
                    Request::SetTerminalVisibility {
                        revision: visible.revision,
                        visible: visible.sessions.clone(),
                    },
                    PendingRequest::Visibility,
                )
                .await
                .is_err()
                {
                    break Some("failed to report terminal visibility".to_string());
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break None;
                }
            }
        }
    };
    for (_, request) in pending {
        if let PendingRequest::User(reply) = request {
            let _ = reply.send(Err(ClientCoreError::NotConnected));
        }
    }
    disconnected
}

async fn connected_interactive_session(
    stream: TransportStream,
    commands: &mut mpsc::Receiver<ClientCommand>,
    context: &ClientSessionContext,
) -> Option<String> {
    let mut shutdown = context.shutdown.clone();
    let (mut reader, mut writer) = split(stream);
    let (interactive_messages_tx, mut interactive_messages_rx) = mpsc::channel(COMMAND_CAPACITY);
    let _interactive_reader = AbortOnDrop(tokio::spawn(async move {
        loop {
            let message = receive_terminal_interactive(&mut reader)
                .await
                .map_err(|error| error.to_string());
            let failed = message.is_err();
            if interactive_messages_tx.send(message).await.is_err() || failed {
                break;
            }
        }
    }));
    let mut pending = HashMap::new();
    let mut heartbeat = None::<(u64, tokio::time::Instant)>;
    let mut heartbeat_tick = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let disconnected = loop {
        tokio::select! {
            _ = heartbeat_tick.tick() => {
                pending.retain(|_, reply: &mut oneshot::Sender<Result<Response, ClientCoreError>>| !reply.is_closed());
                if let Some((_, deadline)) = heartbeat {
                    if tokio::time::Instant::now() >= deadline {
                        break Some("Host heartbeat timed out".into());
                    }
                    continue;
                }
                let request_id = context.next_request_id.fetch_add(1, Ordering::Relaxed);
                let message = TerminalInteractiveMessage::Request(ClientRequest::new(request_id,
                    Request::Ping { sent_millis: request_id }));
                if let Err(error) = send_terminal_interactive(&mut writer, &message).await {
                    break Some(error.to_string());
                }
                heartbeat = Some((request_id, tokio::time::Instant::now() + HEARTBEAT_TIMEOUT));
            }
            command = commands.recv(), if pending.len() < COMMAND_CAPACITY => {
                let Some(ClientCommand {
                    request_id,
                    body,
                    reply,
                    control,
                    deadline,
                }) = command else {
                    break None;
                };
                if deadline <= tokio::time::Instant::now() || reply.as_ref().is_some_and(|reply| reply.is_closed()) {
                    if let Some(reply) = reply { let _ = reply.send(Err(ClientCoreError::RequestTimeout)); }
                    continue;
                }
                let message = match (body, request_id) {
                    (Request::TerminalInput(input), None) => {
                        TerminalInteractiveMessage::Input { input, control }
                    }
                    (
                        body @ (
                            Request::TerminalInput(_)
                            | Request::ResizeTerminal(_)
                            | Request::ScrollTerminal(_)
                        ),
                        Some(request_id),
                    ) => TerminalInteractiveMessage::Request(ClientRequest {
                        request_id,
                        actor_device_id: Some(context.identity.client_instance_id.to_string()),
                        lease_epoch: None,
                        control,
                        body,
                    }),
                    (_, _) => {
                        if let Some(reply) = reply {
                            let _ = reply.send(Err(ClientCoreError::Connection(
                                "non-interactive request was routed to the interactive stream"
                                    .to_string(),
                            )));
                        }
                        continue;
                    }
                };
                if let Err(error) = send_terminal_interactive(&mut writer, &message).await {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(ClientCoreError::Connection(error.to_string())));
                    }
                    break Some(error.to_string());
                }
                if let (Some(request_id), Some(reply)) = (request_id, reply) {
                    pending.insert(request_id, reply);
                }
            }
            message = interactive_messages_rx.recv() => {
                let message = match message {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => break Some(error),
                    None => break Some("Host interactive response reader stopped".to_string()),
                };
                let TerminalInteractiveMessage::Response(response) = message else {
                    break Some(
                        "Host sent a non-response message on the interactive stream".to_string(),
                    );
                };
                if heartbeat.is_some_and(|(id, _)| id == response.request_id) {
                    // Older Hosts reject Ping on this lane but still prove it is live.
                    if !matches!(&response.result,
                        Ok(Response::Pong { .. }) | Err(ProtocolFailure { code: yttt_protocol::FailureCode::InvalidRequest, .. })
                    ) {
                        break Some("Host rejected heartbeat".into());
                    }
                    if let Some((_, deadline)) = heartbeat.take() {
                        context.diagnostics.record_heartbeat(tokio::time::Instant::now().saturating_duration_since(deadline - HEARTBEAT_TIMEOUT));
                    }
                    continue;
                }
                let Some(reply) = pending.remove(&response.request_id) else {
                    continue;
                };
                handle_response(PendingRequest::User(reply), response.result, context);
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break None;
                }
            }
        }
    };
    for (_, reply) in pending {
        let _ = reply.send(Err(ClientCoreError::NotConnected));
    }
    disconnected
}

async fn connected_state_event_session(
    mut stream: TransportStream,
    context: &ClientSessionContext,
) -> Option<String> {
    let mut shutdown = context.shutdown.clone();
    loop {
        tokio::select! {
            event = receive_state_event(&mut stream) => {
                let event = match event {
                    Ok(event) => event,
                    Err(error) => return Some(error.to_string()),
                };
                if *context.current_host_epoch.read() != Some(context.host_epoch) {
                    continue;
                }
                if let ServerEvent::ProfileControl(status) = &event.body {
                    update_profile_control(
                        &context.control,
                        status,
                        &context.current_host_epoch,
                        context.host_epoch,
                        &context.identity.client_instance_id,
                        &context.local_control_recovery,
                    );
                }
                if let ServerEvent::ProjectTokenUsage(summary) = &event.body {
                    apply_project_token_usage(
                        summary.clone(),
                        &context.project_token_usage,
                        &context.current_host_epoch,
                        context.host_epoch,
                        &context.events,
                    );
                }
                if matches!(
                    &event.body,
                    ServerEvent::ResourceCatalogChanged
                        | ServerEvent::SshStateChanged(_)
                        | ServerEvent::ProjectChanged(_)
                ) {
                    match context.catalog_requests.try_send(()) {
                        Ok(()) | Err(mpsc::error::TrySendError::Full(())) => {}
                        Err(mpsc::error::TrySendError::Closed(())) => return None,
                    }
                }
                if let ServerEvent::AgentSnapshot(update) = &event.body {
                    apply_agent_snapshots(
                        vec![(**update).clone()],
                        &context.agent_snapshots,
                        &context.current_host_epoch,
                        context.host_epoch,
                        &context.events,
                    );
                }
                if defer_exit_until_final_frame(&context.terminal_state, context.host_epoch, &event) {
                    continue;
                }
                let _ = context.events.send(ClientEvent::Server(Box::new(event)));
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    return None;
                }
            }
        }
    }
}

fn handle_response(
    pending: PendingRequest,
    result: Result<Response, ProtocolFailure>,
    context: &ClientSessionContext,
) {
    if let Ok(Response::ProfileControl(status)) = &result {
        update_profile_control(
            &context.control,
            status,
            &context.current_host_epoch,
            context.host_epoch,
            &context.identity.client_instance_id,
            &context.local_control_recovery,
        );
    }
    match pending {
        PendingRequest::User(reply) => {
            match &result {
                Ok(Response::TerminalSpawned { lease, .. })
                | Ok(Response::TerminalLease(lease)) => {
                    register_terminal(
                        &context.terminal_state,
                        context.host_epoch,
                        lease.session_id.clone(),
                    );
                    start_terminal_data_channel(lease.session_id.clone(), context);
                }
                Ok(Response::TerminalAttached { lease, checkpoint }) => {
                    register_terminal(
                        &context.terminal_state,
                        context.host_epoch,
                        lease.session_id.clone(),
                    );
                    seed_terminal_mirror(
                        &checkpoint.viewport,
                        &context.terminal_state,
                        context.host_epoch,
                        &context.events,
                    );
                    start_terminal_data_channel(lease.session_id.clone(), context);
                }
                Ok(Response::Resources(resources)) => {
                    let current_host_epoch = context.current_host_epoch.read();
                    if resources.host_epoch == context.host_epoch
                        && *current_host_epoch == Some(context.host_epoch)
                    {
                        context.catalog.write().replace(Arc::new(resources.clone()));
                    }
                }
                Ok(Response::AgentSnapshots {
                    snapshots,
                    project_token_usage: summaries,
                    ..
                }) => {
                    apply_agent_snapshots(
                        snapshots.clone(),
                        &context.agent_snapshots,
                        &context.current_host_epoch,
                        context.host_epoch,
                        &context.events,
                    );
                    apply_project_token_usage_batch(
                        summaries.clone(),
                        &context.project_token_usage,
                        &context.current_host_epoch,
                        context.host_epoch,
                        &context.events,
                    );
                }
                _ => {}
            }
            let _ = reply.send(result.map_err(ClientCoreError::Protocol));
        }
        PendingRequest::Visibility => {}
        PendingRequest::AgentSnapshots => {
            if let Ok(Response::AgentSnapshots {
                snapshots,
                project_token_usage: summaries,
                ..
            }) = result
            {
                apply_agent_snapshots(
                    snapshots,
                    &context.agent_snapshots,
                    &context.current_host_epoch,
                    context.host_epoch,
                    &context.events,
                );
                apply_project_token_usage_batch(
                    summaries,
                    &context.project_token_usage,
                    &context.current_host_epoch,
                    context.host_epoch,
                    &context.events,
                );
            }
        }
        PendingRequest::Catalog => {
            let Ok(Response::Resources(resources)) = result else {
                return;
            };
            if resources.host_epoch != context.host_epoch {
                return;
            }
            let current_host_epoch = context.current_host_epoch.read();
            if *current_host_epoch != Some(context.host_epoch) {
                return;
            }
            let resources = Arc::new(resources);
            context.catalog.write().replace(resources.clone());
            let active = resources
                .terminals
                .iter()
                .map(|placement| placement.session_id.clone())
                .collect::<HashSet<_>>();
            let removed = {
                let mut terminal_state = context.terminal_state.write();
                if terminal_state.host_epoch != Some(context.host_epoch) {
                    return;
                }
                let removed = terminal_state
                    .known_sessions
                    .iter()
                    .filter(|session_id| !active.contains(*session_id))
                    .cloned()
                    .collect::<Vec<_>>();
                terminal_state.known_sessions = active;
                {
                    let TerminalState {
                        known_sessions,
                        mirrors,
                        data_channels,
                        deferred_exits,
                        ..
                    } = &mut *terminal_state;
                    mirrors.retain(|session_id, _| known_sessions.contains(session_id));
                    deferred_exits.retain(|session_id, _| known_sessions.contains(session_id));
                    data_channels.retain(|session_id, channel| {
                        let keep = known_sessions.contains(session_id);
                        if !keep {
                            let _ = channel.desired.send(false);
                        }
                        keep
                    });
                }
                removed
            };
            for session_id in removed {
                let _ = context
                    .events
                    .send(ClientEvent::TerminalUnavailable(session_id));
            }
            let _ = context
                .events
                .send(ClientEvent::ResourceCatalogUpdated(resources));
        }
    }
}

fn register_terminal(
    terminal_state: &RwLock<TerminalState>,
    host_epoch: u64,
    session_id: TerminalSessionId,
) {
    let mut terminal_state = terminal_state.write();
    if terminal_state.host_epoch == Some(host_epoch) {
        terminal_state.known_sessions.insert(session_id);
    }
}

fn start_terminal_data_channel(session_id: TerminalSessionId, context: &ClientSessionContext) {
    let connector = context.connector.clone();
    let data_connects = context.data_connects.clone();
    let mut identity = context.identity.clone();
    let token = context.token.clone();
    let terminal_state = context.terminal_state.clone();
    let events = context.events.clone();
    let diagnostics = context.diagnostics.clone();
    let mut shutdown = context.shutdown.clone();
    let host_epoch = context.host_epoch;
    let (worker_generation, mut desired_rx) = {
        let mut state = terminal_state.write();
        if state.host_epoch != Some(host_epoch) || !state.known_sessions.contains(&session_id) {
            return;
        }
        if state
            .data_channels
            .get(&session_id)
            .is_some_and(|channel| !channel.desired.is_closed() && *channel.desired.borrow())
        {
            return;
        }
        if let Some(previous) = state.data_channels.remove(&session_id) {
            let _ = previous.desired.send(false);
        }
        let worker_generation = state.next_worker_generation;
        state.next_worker_generation = state.next_worker_generation.wrapping_add(1);
        let (desired, desired_rx) = watch::channel(true);
        state.data_channels.insert(
            session_id.clone(),
            TerminalDataChannel {
                generation: worker_generation,
                desired,
            },
        );
        (worker_generation, desired_rx)
    };
    identity.channel = ConnectionChannel::TerminalData;
    identity.can_force_stop = false;
    identity.terminal_session_id = Some(session_id.clone());
    tokio::spawn(async move {
        let mut attempt = 0_u32;
        'worker: loop {
            if *shutdown.borrow() || !*desired_rx.borrow() {
                break;
            }
            let result = tokio::select! {
                result = async {
                    let _permit = data_connects.acquire().await.expect("data connection semaphore remains open");
                    establish(&connector, &identity, &token).await
                } => result,
                _ = desired_rx.wait_for(|desired| !*desired) => break,
                _ = shutdown.wait_for(|stopped| *stopped) => break,
            };
            let (mut stream, _) = match result {
                Ok(connection) => connection,
                Err(ConnectFailure::Fatal(_)) => break,
                Err(ConnectFailure::Retry(_)) => {
                    attempt = attempt.saturating_add(1);
                    let delay = retry_delay(attempt, &identity);
                    tokio::select! {
                        _ = tokio::time::sleep(delay) => {}
                        changed = desired_rx.changed() => {
                            if changed.is_err() {
                                break 'worker;
                            }
                        }
                        changed = shutdown.changed() => {
                            if changed.is_err() || *shutdown.borrow() {
                                break 'worker;
                            }
                        }
                    }
                    continue;
                }
            };
            loop {
                tokio::select! {
                    message = receive_control_observed(&mut stream) => {
                        let (message, observation) = match message {
                            Ok(result) => result,
                            Err(_) => break,
                        };
                        attempt = 0;
                        diagnostics.record_ipc_read(observation);
                        let ControlMessage::Event(event) = message else {
                            break;
                        };
                        // A gap means the Host broke its stream contract; a fresh
                        // lane starts from a Host snapshot.
                        let Ok(terminal_exited) = handle_terminal_data_event(
                            event,
                            &terminal_state,
                            host_epoch,
                            worker_generation,
                            &events,
                            &diagnostics,
                        ) else {
                            break;
                        };
                        if terminal_exited {
                            stop_terminal_data_channel_if_current(
                                &terminal_state,
                                &session_id,
                                host_epoch,
                                worker_generation,
                            );
                        }
                    }
                    changed = desired_rx.changed() => {
                        if changed.is_err() {
                            break 'worker;
                        }
                        if !*desired_rx.borrow() {
                            break;
                        }
                    }
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() {
                            break 'worker;
                        }
                    }
                }
            }
            if !*desired_rx.borrow() {
                continue;
            }
            attempt = attempt.saturating_add(1);
            let delay = retry_delay(attempt, &identity);
            tokio::select! {
                _ = tokio::time::sleep(delay) => {}
                changed = desired_rx.changed() => {
                    if changed.is_err() {
                        break;
                    }
                }
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() {
                        break;
                    }
                }
            }
        }
        let exit = {
            let mut state = terminal_state.write();
            if state
                .data_channels
                .get(&session_id)
                .is_some_and(|channel| channel.generation == worker_generation)
            {
                state.data_channels.remove(&session_id);
            }
            if state.host_epoch == Some(host_epoch) {
                state.take_ready_exit(&session_id)
            } else {
                None
            }
        };
        if let Some(event) = exit {
            let _ = events.send(ClientEvent::Server(Box::new(event)));
        }
    });
}

fn terminal_stops_for_request(
    terminal_state: &TerminalState,
    request: &Request,
) -> PendingTerminalStops {
    match request {
        Request::DetachTerminal { session_id }
        | Request::TerminateTerminal { session_id, .. }
        | Request::AcknowledgeTerminalExit { session_id, .. } => {
            terminal_worker_fence(terminal_state, session_id)
                .map_or(PendingTerminalStops::None, PendingTerminalStops::One)
        }
        Request::TerminateMany { requests } => {
            let mut fences = requests
                .iter()
                .filter_map(|request| terminal_worker_fence(terminal_state, &request.session_id))
                .collect::<Vec<_>>();
            match fences.len() {
                0 => PendingTerminalStops::None,
                1 => PendingTerminalStops::One(fences.pop().expect("one terminal worker fence")),
                _ => PendingTerminalStops::Many(fences),
            }
        }
        _ => PendingTerminalStops::None,
    }
}

fn terminal_worker_fence(
    terminal_state: &TerminalState,
    session_id: &TerminalSessionId,
) -> Option<TerminalWorkerFence> {
    let host_epoch = terminal_state.host_epoch?;
    let channel = terminal_state.data_channels.get(session_id)?;
    Some(TerminalWorkerFence {
        session_id: session_id.clone(),
        host_epoch,
        generation: channel.generation,
    })
}

fn stop_terminal_data_channel_if_current(
    terminal_state: &RwLock<TerminalState>,
    session_id: &TerminalSessionId,
    host_epoch: u64,
    worker_generation: u64,
) {
    let mut terminal_state = terminal_state.write();
    if terminal_state.host_epoch != Some(host_epoch) {
        return;
    }
    if terminal_state
        .data_channels
        .get(session_id)
        .is_some_and(|channel| channel.generation == worker_generation)
        && let Some(channel) = terminal_state.data_channels.remove(session_id)
    {
        let _ = channel.desired.send(false);
    }
}

fn update_profile_control(
    cache: &RwLock<Option<yttt_protocol::session::ControlStatus>>,
    status: &yttt_protocol::session::ControlStatus,
    current_host_epoch: &RwLock<Option<u64>>,
    host_epoch: u64,
    client_id: &yttt_core::model::ids::ClientInstanceId,
    local_control_recovery: &LocalControlRecovery,
) {
    let current_host_epoch = current_host_epoch.read();
    if *current_host_epoch != Some(host_epoch) || status.context.host_epoch != host_epoch {
        return;
    }
    let mut current = cache.write();
    if current.as_ref().is_none_or(|current| {
        status.context.host_epoch > current.context.host_epoch
            || (status.context.host_epoch == current.context.host_epoch
                && status.revision >= current.revision)
    }) {
        *current = Some(status.clone());
        local_control_recovery.observe(status, client_id);
    }
}

/// Returns `true` when `event` is a `TerminalExit` held back until its data lane
/// delivers the final frame; the lane releases it.
fn defer_exit_until_final_frame(
    terminal_state: &RwLock<TerminalState>,
    host_epoch: u64,
    event: &HostEvent,
) -> bool {
    let ServerEvent::TerminalExit {
        session_id,
        session_epoch,
        final_sequence,
        ..
    } = &event.body
    else {
        return false;
    };
    let mut state = terminal_state.write();
    if state.host_epoch != Some(host_epoch)
        || !data_lane_open(&state, session_id)
        || final_viewport_is_ready(&state, session_id, *session_epoch, *final_sequence)
    {
        return false;
    }
    state
        .deferred_exits
        .insert(session_id.clone(), event.clone());
    true
}

fn data_lane_open(state: &TerminalState, session_id: &TerminalSessionId) -> bool {
    state
        .data_channels
        .get(session_id)
        .is_some_and(|channel| !channel.desired.is_closed() && *channel.desired.borrow())
}

/// Installs an attach checkpoint as the initial mirror. Once a mirror exists only
/// its data lane writes it, so frames are never interleaved from two sources.
fn seed_terminal_mirror(
    viewport: &yttt_protocol::terminal::SemanticViewport,
    terminal_state: &RwLock<TerminalState>,
    host_epoch: u64,
    events: &broadcast::Sender<ClientEvent>,
) {
    let mut state = terminal_state.write();
    if state.host_epoch != Some(host_epoch)
        || !state.known_sessions.contains(&viewport.session_id)
        || state.mirrors.contains_key(&viewport.session_id)
    {
        return;
    }
    state.mirrors.insert(
        viewport.session_id.clone(),
        TerminalMirror::new(viewport.clone()),
    );
    let _ = events.send(ClientEvent::TerminalUpdated(Arc::new(
        TerminalStreamUpdate::Snapshot(viewport.clone()),
    )));
}

fn apply_terminal_update(
    update: TerminalStreamUpdate,
    terminal_state: &RwLock<TerminalState>,
    host_epoch: u64,
    worker_generation: u64,
    events: &broadcast::Sender<ClientEvent>,
) -> MirrorApply {
    let session_id = update.session_id().clone();
    let published_update = Arc::new(update);
    let mut state = terminal_state.write();
    if state.host_epoch != Some(host_epoch)
        || state
            .data_channels
            .get(&session_id)
            .is_none_or(|channel| channel.generation != worker_generation)
        || (!state.known_sessions.contains(&session_id) && !state.mirrors.contains_key(&session_id))
    {
        return MirrorApply::Ignored;
    }
    let apply = match state.mirrors.get_mut(&session_id) {
        Some(mirror) => mirror.apply(published_update.as_ref()),
        None => match published_update.as_ref() {
            TerminalStreamUpdate::Snapshot(viewport) => {
                state
                    .mirrors
                    .insert(session_id.clone(), TerminalMirror::new(viewport.clone()));
                MirrorApply::Updated
            }
            _ => MirrorApply::SequenceGap,
        },
    };
    let exit = state.take_ready_exit(&session_id);
    drop(state);
    if apply == MirrorApply::Updated {
        let _ = events.send(ClientEvent::TerminalUpdated(published_update));
    }
    if let Some(event) = exit {
        let _ = events.send(ClientEvent::Server(Box::new(event)));
    }
    apply
}

fn agent_address_key(scope: &yttt_protocol::agent::AgentHookScope) -> String {
    format!(
        "{}\u{1f}{}\u{1f}{}",
        scope.project_id, scope.tab_id, scope.pane_id
    )
}

fn apply_agent_snapshots(
    updates: Vec<AgentSnapshotUpdate>,
    snapshots: &Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>,
    current_host_epoch: &RwLock<Option<u64>>,
    host_epoch: u64,
    events: &broadcast::Sender<ClientEvent>,
) {
    let current_host_epoch = current_host_epoch.read();
    if *current_host_epoch != Some(host_epoch) {
        return;
    }
    for update in updates {
        if update.host_epoch != host_epoch {
            continue;
        }
        let key = agent_address_key(&update.scope);
        let applied = {
            let mut snapshots = snapshots.write();
            let stale = snapshots.get(&key).is_some_and(|current| {
                current.host_epoch > update.host_epoch
                    || (current.host_epoch == update.host_epoch
                        && (current.scope.generation > update.scope.generation
                            || (current.scope.generation == update.scope.generation
                                && current.sequence >= update.sequence)))
            });
            if stale {
                false
            } else {
                snapshots.insert(key, update.clone());
                true
            }
        };
        if applied {
            let _ = events.send(ClientEvent::AgentSnapshotUpdated(Box::new(update)));
        }
    }
}

fn apply_project_token_usage_batch(
    summaries: Vec<ProjectTokenUsage>,
    usage: &Arc<RwLock<HashMap<String, ProjectTokenUsage>>>,
    current_host_epoch: &RwLock<Option<u64>>,
    host_epoch: u64,
    events: &broadcast::Sender<ClientEvent>,
) {
    for summary in summaries {
        apply_project_token_usage(summary, usage, current_host_epoch, host_epoch, events);
    }
}

fn apply_project_token_usage(
    summary: ProjectTokenUsage,
    usage: &Arc<RwLock<HashMap<String, ProjectTokenUsage>>>,
    current_host_epoch: &RwLock<Option<u64>>,
    host_epoch: u64,
    events: &broadcast::Sender<ClientEvent>,
) {
    let current_host_epoch = current_host_epoch.read();
    if *current_host_epoch != Some(host_epoch) {
        return;
    }
    let applied = {
        let mut usage = usage.write();
        match usage.get(&summary.project_id) {
            Some(current) if current.updated_at >= summary.updated_at => false,
            _ => {
                usage.insert(summary.project_id.clone(), summary.clone());
                true
            }
        }
    };
    if applied {
        let _ = events.send(ClientEvent::ProjectTokenUsageUpdated(Box::new(summary)));
    }
}

/// Applies one data-lane frame. `Ok` reports whether it carried the process
/// exit; `Err` means the frame does not continue the mirror and the lane must reset.
fn handle_terminal_data_event(
    event: HostEvent,
    terminal_state: &RwLock<TerminalState>,
    host_epoch: u64,
    worker_generation: u64,
    events: &broadcast::Sender<ClientEvent>,
    diagnostics: &ClientPipelineDiagnostics,
) -> Result<bool, ()> {
    let ServerEvent::Terminal(update) = event.body else {
        return Ok(false);
    };
    let terminal_exited = match &update {
        TerminalStreamUpdate::Snapshot(viewport) => {
            matches!(viewport.process_state, TerminalProcessState::Exited { .. })
        }
        TerminalStreamUpdate::Delta(delta) => {
            matches!(
                delta.process_state,
                Some(TerminalProcessState::Exited { .. })
            )
        }
        TerminalStreamUpdate::RawTail { .. } | TerminalStreamUpdate::ResyncRequired { .. } => false,
    };
    let merge_started_at = Instant::now();
    let apply = apply_terminal_update(
        update,
        terminal_state,
        host_epoch,
        worker_generation,
        events,
    );
    diagnostics.record_terminal_merge(merge_started_at.elapsed());
    if apply == MirrorApply::SequenceGap {
        diagnostics.record_data_lane_reset();
        return Err(());
    }
    Ok(terminal_exited)
}

async fn send_internal_request(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    pending: &mut HashMap<u64, PendingRequest>,
    next_request_id: &impl Fn() -> u64,
    actor_device_id: Option<String>,
    body: Request,
    request: PendingRequest,
) -> Result<(), ()> {
    let request_id = next_request_id();
    send_control(
        writer,
        &ControlMessage::Request(ClientRequest {
            request_id,
            actor_device_id,
            lease_epoch: None,
            control: None,
            body,
        }),
    )
    .await
    .map_err(|_| ())?;
    pending.insert(request_id, request);
    Ok(())
}

fn set_state(
    state: &watch::Sender<ConnectionState>,
    events: &broadcast::Sender<ClientEvent>,
    next: ConnectionState,
) {
    state.send_replace(next.clone());
    let _ = events.send(ClientEvent::Connection(next));
}

struct HostEpochState<'a> {
    current_host_epoch: &'a RwLock<Option<u64>>,
    terminal_state: &'a RwLock<TerminalState>,
    catalog: &'a RwLock<Option<Arc<ResourceCatalog>>>,
    agent_snapshots: &'a RwLock<HashMap<String, AgentSnapshotUpdate>>,
    project_token_usage: &'a RwLock<HashMap<String, ProjectTokenUsage>>,
    profile_control: &'a RwLock<Option<yttt_protocol::session::ControlStatus>>,
    events: &'a broadcast::Sender<ClientEvent>,
}

fn install_host_epoch(next_host_epoch: u64, state: &HostEpochState<'_>) {
    let HostEpochState {
        current_host_epoch,
        terminal_state,
        catalog,
        agent_snapshots,
        project_token_usage,
        profile_control,
        events,
    } = state;
    let mut current_host_epoch = current_host_epoch.write();
    match *current_host_epoch {
        Some(current) if current == next_host_epoch => {}
        None => {
            terminal_state.write().host_epoch = Some(next_host_epoch);
            *current_host_epoch = Some(next_host_epoch);
        }
        Some(_) => {
            let mut terminal_state = terminal_state.write();
            terminal_state.host_epoch = Some(next_host_epoch);
            terminal_state.mirrors.clear();
            terminal_state.deferred_exits.clear();
            for (_, channel) in terminal_state.data_channels.drain() {
                let _ = channel.desired.send(false);
            }
            catalog.write().take();
            agent_snapshots.write().clear();
            project_token_usage.write().clear();
            profile_control.write().take();
            *current_host_epoch = Some(next_host_epoch);
            let _ = events.send(ClientEvent::AgentSnapshotsReset {
                host_epoch: next_host_epoch,
            });
            for session_id in terminal_state.known_sessions.drain() {
                let _ = events.send(ClientEvent::TerminalUnavailable(session_id));
            }
        }
    }
}

#[derive(Debug)]
enum ConnectFailure {
    Retry(String),
    Fatal(String),
}

impl From<HandshakeError> for ConnectFailure {
    fn from(error: HandshakeError) -> Self {
        use yttt_protocol::RejectReason;
        let retryable = match &error {
            HandshakeError::Timeout => true,
            HandshakeError::Wire(yttt_transport::WireError::Codec(
                yttt_protocol::ProtocolCodecError::Io(_),
            )) => true,
            HandshakeError::Rejected(reason) => match reason {
                RejectReason::StaleHostEpoch
                | RejectReason::AlreadyConnected
                | RejectReason::HostShuttingDown => true,
                RejectReason::VersionMismatch { .. }
                | RejectReason::ProfileMismatch
                | RejectReason::AuthenticationFailed
                | RejectReason::InvalidMessage
                | RejectReason::PermissionDenied => false,
            },
            HandshakeError::Wire(_)
            | HandshakeError::UnexpectedMessage
            | HandshakeError::IdentityMismatch
            | HandshakeError::AuthenticationFailed => false,
        };
        if retryable {
            Self::Retry(error.to_string())
        } else {
            Self::Fatal(error.to_string())
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ClientCoreError {
    #[error("Host connection failed: {0}")]
    Connection(String),
    #[error("Host connection timed out")]
    ConnectionTimeout,
    #[error("Host client supervisor stopped")]
    SupervisorStopped,
    #[error("Host is not connected")]
    NotConnected,
    #[error("Host request queue is backpressured")]
    Backpressure,
    #[error("Host request timed out; the remote operation may have completed")]
    RequestTimeout,
    #[error("Host rejected request: {0:?}")]
    Protocol(ProtocolFailure),
}

#[cfg(test)]
mod recovery_tests;
