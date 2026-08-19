#![forbid(unsafe_code)]

mod diagnostics;
mod mirror;

use std::{
    collections::{HashMap, HashSet},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use diagnostics::ClientPipelineDiagnostics;
pub use diagnostics::{ClientLatencyDiagnosticsSnapshot, ClientPipelineDiagnosticsSnapshot};
pub use mirror::{MirrorApply, TerminalMirror, TerminalMirrorMetadata};
use parking_lot::{Mutex, RwLock};
use tokio::{
    io::split,
    sync::{broadcast, mpsc, oneshot, watch},
};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::{
    ClientRequest, ConnectionChannel, ControlMessage, HostEvent, ProtocolFailure, Request,
    ResourceCatalog, Response, ServerEvent, TerminalInteractiveMessage,
    agent::{AgentSnapshotCursor, AgentSnapshotUpdate},
    terminal::{TerminalInput, TerminalProcessState, TerminalStreamUpdate},
};
use yttt_transport::{
    AuthToken, AuthenticatedHost, ClientIdentity, SharedConnector, TransportConnector,
    TransportStream, client_handshake, receive_control, receive_control_observed,
    receive_state_event, receive_terminal_interactive, send_control, send_terminal_interactive,
};

const COMMAND_CAPACITY: usize = 256;
const CHECKPOINT_CAPACITY: usize = 64;
const EVENT_CAPACITY: usize = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONTROL_RESPONSE_GRACE: Duration = Duration::from_millis(100);
const INITIAL_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);

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
    Server(HostEvent),
    TerminalUpdated(Arc<TerminalStreamUpdate>),
    TerminalUnavailable(TerminalSessionId),
    AgentSnapshotUpdated(Box<AgentSnapshotUpdate>),
}

pub struct ClientCore {
    inner: Arc<ClientCoreInner>,
}

struct ClientCoreInner {
    commands: mpsc::Sender<ClientCommand>,
    interactive_commands: mpsc::Sender<ClientCommand>,
    events: broadcast::Sender<ClientEvent>,
    state: watch::Receiver<ConnectionState>,
    mirrors: Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    supervisor: Mutex<Option<tokio::task::JoinHandle<()>>>,
    known_sessions: Arc<RwLock<HashSet<TerminalSessionId>>>,
    catalog: Arc<RwLock<Option<Arc<ResourceCatalog>>>>,
    agent_snapshots: Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>,
    next_request_id: Arc<AtomicU64>,
    data_channels: Arc<RwLock<HashMap<TerminalSessionId, watch::Sender<bool>>>>,
    shutdown: watch::Sender<bool>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
}

impl Drop for ClientCoreInner {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        for desired in self.data_channels.read().values() {
            let _ = desired.send(false);
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
}

pub struct PendingClientRequest {
    response: oneshot::Receiver<Result<Response, ClientCoreError>>,
    timeout: Duration,
    stopped_session: Option<TerminalSessionId>,
    data_channels: Arc<RwLock<HashMap<TerminalSessionId, watch::Sender<bool>>>>,
}

impl PendingClientRequest {
    pub async fn wait(self) -> Result<Response, ClientCoreError> {
        let response = tokio::time::timeout(self.timeout, self.response)
            .await
            .map_err(|_| ClientCoreError::RequestTimeout)?
            .map_err(|_| ClientCoreError::SupervisorStopped)??;
        if matches!(
            &response,
            Response::TerminalDetached
                | Response::TerminalTerminated(_)
                | Response::TerminalExitAcknowledged
        ) && let Some(session_id) = self.stopped_session
            && let Some(desired) = self.data_channels.read().get(&session_id)
        {
            let _ = desired.send(false);
        }
        if let Response::TerminalsTerminated { results } = &response {
            let channels = self.data_channels.read();
            for result in results.iter().filter(|result| result.result.is_ok()) {
                if let Some(desired) = channels.get(&result.session_id) {
                    let _ = desired.send(false);
                }
            }
        }
        Ok(response)
    }
}

enum PendingRequest {
    User(oneshot::Sender<Result<Response, ClientCoreError>>),
    Checkpoint(TerminalSessionId),
    Catalog,
    AgentSnapshots,
}

struct ClientSessionContext {
    checkpoint_requests: mpsc::Sender<TerminalSessionId>,
    catalog_requests: mpsc::Sender<()>,
    events: broadcast::Sender<ClientEvent>,
    mirrors: Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    connector: SharedConnector,
    identity: ClientIdentity,
    token: AuthToken,
    data_channels: Arc<RwLock<HashMap<TerminalSessionId, watch::Sender<bool>>>>,
    known_sessions: Arc<RwLock<HashSet<TerminalSessionId>>>,
    catalog: Arc<RwLock<Option<Arc<ResourceCatalog>>>>,
    agent_snapshots: Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>,
    next_request_id: Arc<AtomicU64>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
    deferred_terminal_events: Arc<Mutex<HashMap<TerminalSessionId, HostEvent>>>,
    shutdown: watch::Receiver<bool>,
}

impl ClientCore {
    pub async fn connect(
        connector: impl TransportConnector,
        identity: ClientIdentity,
        token: AuthToken,
    ) -> Result<Self, ClientCoreError> {
        let connector = SharedConnector::new(connector);
        let (checkpoint_requests, checkpoint_rx) = mpsc::channel(CHECKPOINT_CAPACITY);
        let (command_tx, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (interactive_tx, interactive_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (state_tx, state_rx) = watch::channel(ConnectionState::Connecting);
        let data_channels = Arc::new(RwLock::new(HashMap::new()));
        let (shutdown, shutdown_rx) = watch::channel(false);
        let mirrors = Arc::new(RwLock::new(HashMap::new()));
        let known_sessions = Arc::new(RwLock::new(HashSet::new()));
        let catalog = Arc::new(RwLock::new(None));
        let agent_snapshots = Arc::new(RwLock::new(HashMap::new()));
        let next_request_id = Arc::new(AtomicU64::new(1));
        let diagnostics = Arc::new(ClientPipelineDiagnostics::default());
        let (initial_tx, initial_rx) = oneshot::channel();
        let supervisor = tokio::spawn(run_supervisor(
            connector.clone(),
            identity.clone(),
            token.clone(),
            command_rx,
            interactive_rx,
            checkpoint_requests,
            checkpoint_rx,
            events.clone(),
            state_tx,
            mirrors.clone(),
            data_channels.clone(),
            known_sessions.clone(),
            catalog.clone(),
            agent_snapshots.clone(),
            next_request_id.clone(),
            diagnostics.clone(),
            shutdown_rx,
            initial_tx,
        ));
        let inner = Arc::new(ClientCoreInner {
            commands: command_tx,
            interactive_commands: interactive_tx,
            events,
            state: state_rx,
            mirrors,
            known_sessions,
            catalog,
            next_request_id,
            agent_snapshots,
            data_channels,
            shutdown,
            diagnostics,
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
            .mirrors
            .read()
            .get(session_id)
            .map(|mirror| mirror.viewport().clone())
    }

    pub fn terminal_metadata(
        &self,
        session_id: &TerminalSessionId,
    ) -> Option<TerminalMirrorMetadata> {
        self.inner
            .mirrors
            .read()
            .get(session_id)
            .map(TerminalMirror::metadata)
    }

    pub fn terminal_snapshots(&self) -> Vec<yttt_protocol::terminal::SemanticViewport> {
        let mirrors = self.inner.mirrors.read();
        let mut snapshots = mirrors
            .values()
            .map(|mirror| mirror.viewport().clone())
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.session_id.as_str().cmp(right.session_id.as_str()));
        snapshots
    }

    pub fn known_terminal_ids(&self) -> Vec<TerminalSessionId> {
        self.inner.known_sessions.read().iter().cloned().collect()
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

    pub fn enqueue_request(&self, body: Request) -> Result<PendingClientRequest, ClientCoreError> {
        if !matches!(self.state(), ConnectionState::Ready { .. }) {
            return Err(ClientCoreError::NotConnected);
        }
        let timeout = Self::request_timeout(&body);
        let stopped_session = match &body {
            Request::DetachTerminal { session_id }
            | Request::TerminateTerminal { session_id, .. }
            | Request::AcknowledgeTerminalExit { session_id, .. } => Some(session_id.clone()),
            _ => None,
        };
        let (reply, response) = oneshot::channel();
        self.enqueue_command(body, Some(reply))?;
        Ok(PendingClientRequest {
            response,
            timeout,
            stopped_session,
            data_channels: self.inner.data_channels.clone(),
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
        sender
            .try_send(ClientCommand {
                request_id,
                body,
                reply,
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
        for desired in self.inner.data_channels.read().values() {
            let _ = desired.send(false);
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
    checkpoint_requests: mpsc::Sender<TerminalSessionId>,
    mut checkpoint_rx: mpsc::Receiver<TerminalSessionId>,
    events: broadcast::Sender<ClientEvent>,
    state: watch::Sender<ConnectionState>,
    mirrors: Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    data_channels: Arc<RwLock<HashMap<TerminalSessionId, watch::Sender<bool>>>>,
    known_sessions: Arc<RwLock<HashSet<TerminalSessionId>>>,
    catalog: Arc<RwLock<Option<Arc<ResourceCatalog>>>>,
    agent_snapshots: Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>,
    next_request_id: Arc<AtomicU64>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
    mut shutdown: watch::Receiver<bool>,
    initial: oneshot::Sender<Result<(), String>>,
) {
    set_state(&state, &events, ConnectionState::Connecting);
    let mut lanes = match establish_session_lanes(&connector, &identity, &token).await {
        Ok(lanes) => lanes,
        Err(error) => {
            let message = error.message().to_string();
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
    };
    identity.host_epoch_hint = Some(lanes.host.host_epoch);
    set_state(
        &state,
        &events,
        ConnectionState::Ready {
            host_epoch: lanes.host.host_epoch,
            connection_sequence: lanes.host.connection_sequence,
        },
    );
    let _ = initial.send(Ok(()));
    let deferred_terminal_events = Arc::new(Mutex::new(HashMap::new()));
    let (catalog_requests, mut catalog_rx) = mpsc::channel(1);
    let mut attempt = 0_u32;
    loop {
        let session = ClientSessionContext {
            checkpoint_requests: checkpoint_requests.clone(),
            catalog_requests: catalog_requests.clone(),
            events: events.clone(),
            mirrors: mirrors.clone(),
            connector: connector.clone(),
            identity: identity.clone(),
            token: token.clone(),
            data_channels: data_channels.clone(),
            known_sessions: known_sessions.clone(),
            catalog: catalog.clone(),
            agent_snapshots: agent_snapshots.clone(),
            next_request_id: next_request_id.clone(),
            diagnostics: diagnostics.clone(),
            deferred_terminal_events: deferred_terminal_events.clone(),
            shutdown: shutdown.clone(),
        };
        let control = connected_control_session(
            lanes.control,
            &mut commands,
            &mut checkpoint_rx,
            &mut catalog_rx,
            &session,
        );
        let interactive =
            connected_interactive_session(lanes.interactive, &mut interactive_commands, &session);
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
        };
        let disconnected = if control_exited {
            lane_result
        } else {
            tokio::select! {
                biased;
                result = &mut control => result,
                _ = tokio::time::sleep(CONTROL_RESPONSE_GRACE) => lane_result,
            }
        };
        let Some(message) = disconnected else {
            set_state(&state, &events, ConnectionState::Disconnected);
            return;
        };
        attempt = attempt.saturating_add(1);
        set_state(
            &state,
            &events,
            ConnectionState::Reconnecting {
                attempt,
                message: message.clone(),
            },
        );
        let delay = Duration::from_millis((50_u64 << attempt.min(5)).min(1_000));
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
            match establish_session_lanes(&connector, &identity, &token).await {
                Ok(new_lanes) => {
                    lanes = new_lanes;
                    identity.host_epoch_hint = Some(lanes.host.host_epoch);
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
                    let delay = Duration::from_millis((50_u64 << attempt.min(5)).min(1_000));
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
}

async fn establish_session_lanes(
    connector: &SharedConnector,
    identity: &ClientIdentity,
    token: &AuthToken,
) -> Result<SessionLanes, ConnectFailure> {
    let (control, host) = establish(connector, identity, token).await?;
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
    })
}

async fn establish(
    connector: &SharedConnector,
    identity: &ClientIdentity,
    token: &AuthToken,
) -> Result<(TransportStream, AuthenticatedHost), ConnectFailure> {
    let mut stream = connector
        .connect()
        .await
        .map_err(|error| ConnectFailure::Retry(error.to_string()))?;
    let host = client_handshake(&mut stream, identity, token)
        .await
        .map_err(|error| ConnectFailure::Fatal(error.to_string()))?;
    Ok((stream, host))
}

async fn connected_control_session(
    stream: TransportStream,
    commands: &mut mpsc::Receiver<ClientCommand>,
    checkpoint_rx: &mut mpsc::Receiver<TerminalSessionId>,
    catalog_rx: &mut mpsc::Receiver<()>,
    context: &ClientSessionContext,
) -> Option<String> {
    let mut shutdown = context.shutdown.clone();
    let (mut reader, mut writer) = split(stream);
    let mut pending = HashMap::new();
    let mut checkpoint_pending = HashSet::new();
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
    let control_reader = tokio::spawn(async move {
        loop {
            let message = receive_control(&mut reader)
                .await
                .map_err(|error| error.to_string());
            let failed = message.is_err();
            if control_messages_tx.send(message).await.is_err() || failed {
                break;
            }
        }
    });

    let disconnected = 'connected: loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(ClientCommand {
                    request_id,
                    body,
                    reply,
                }) = command else {
                    break None;
                };
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
                        if let PendingRequest::Checkpoint(session_id) = &pending_request {
                            checkpoint_pending.remove(session_id);
                        }
                        let completed_checkpoint = match (&pending_request, &response.result) {
                            (
                                PendingRequest::Checkpoint(session_id),
                                Ok(Response::TerminalCheckpoint(_)),
                            ) => Some(session_id.clone()),
                            _ => None,
                        };
                        let checkpoints =
                            handle_response(pending_request, response.result, context);
                        if let Some(session_id) = completed_checkpoint
                            && let Some(event) =
                                context.deferred_terminal_events.lock().remove(&session_id)
                        {
                            let _ = context.events.send(ClientEvent::Server(event));
                        }
                        for session_id in checkpoints {
                            if checkpoint_pending.insert(session_id.clone())
                                && send_checkpoint_request(
                                    &mut writer,
                                    &mut pending,
                                    &next_internal_request_id,
                                    Some(context.identity.client_instance_id.to_string()),
                                    session_id,
                                )
                                .await
                                .is_err()
                            {
                                break 'connected Some(
                                    "failed to request terminal checkpoint".to_string(),
                                );
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
            checkpoint = checkpoint_rx.recv() => {
                let Some(session_id) = checkpoint else {
                    break None;
                };
                if checkpoint_pending.insert(session_id.clone())
                    && send_checkpoint_request(
                        &mut writer,
                        &mut pending,
                        &next_internal_request_id,
                        Some(context.identity.client_instance_id.to_string()),
                        session_id,
                    )
                    .await
                    .is_err()
                {
                    break Some("failed to request terminal checkpoint".to_string());
                }
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break None;
                }
            }
        }
    };
    control_reader.abort();
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
    let interactive_reader = tokio::spawn(async move {
        loop {
            let message = receive_terminal_interactive(&mut reader)
                .await
                .map_err(|error| error.to_string());
            let failed = message.is_err();
            if interactive_messages_tx.send(message).await.is_err() || failed {
                break;
            }
        }
    });
    let mut pending = HashMap::new();
    let disconnected = loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(ClientCommand {
                    request_id,
                    body,
                    reply,
                }) = command else {
                    break None;
                };
                let message = match (body, request_id) {
                    (Request::TerminalInput(input), None) => {
                        TerminalInteractiveMessage::Input(input)
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
                let Some(reply) = pending.remove(&response.request_id) else {
                    continue;
                };
                let _ = handle_response(PendingRequest::User(reply), response.result, context);
            }
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break None;
                }
            }
        }
    };
    interactive_reader.abort();
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
                let checkpoint = handle_event(
                    event.clone(),
                    &context.mirrors,
                    &context.known_sessions,
                    Some(&context.agent_snapshots),
                    &context.events,
                );
                if checkpoint.is_some()
                    && matches!(&event.body, ServerEvent::TerminalExit { .. })
                {
                    let ServerEvent::TerminalExit { session_id, .. } = &event.body else {
                        unreachable!();
                    };
                    context
                        .deferred_terminal_events
                        .lock()
                        .insert(session_id.clone(), event);
                } else {
                    let _ = context.events.send(ClientEvent::Server(event));
                }
                if let Some(session_id) = checkpoint {
                    match context.checkpoint_requests.try_send(session_id) {
                        Ok(()) | Err(mpsc::error::TrySendError::Full(_)) => {}
                        Err(mpsc::error::TrySendError::Closed(_)) => return None,
                    }
                }
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
) -> Vec<TerminalSessionId> {
    let mirrors = &context.mirrors;
    let events = &context.events;
    let known_sessions = &context.known_sessions;
    let catalog = &context.catalog;
    let agent_snapshots = &context.agent_snapshots;
    match pending {
        PendingRequest::User(reply) => {
            match &result {
                Ok(Response::TerminalSpawned { lease, .. }) => {
                    known_sessions.write().insert(lease.session_id.clone());
                    start_terminal_data_channel(lease.session_id.clone(), context);
                }
                Ok(Response::TerminalAttached { lease, checkpoint }) => {
                    known_sessions.write().insert(lease.session_id.clone());
                    mirrors.write().insert(
                        lease.session_id.clone(),
                        TerminalMirror::new(checkpoint.viewport.clone()),
                    );
                    let _ = events.send(ClientEvent::TerminalUpdated(Arc::new(
                        TerminalStreamUpdate::Snapshot(checkpoint.viewport.clone()),
                    )));
                    start_terminal_data_channel(lease.session_id.clone(), context);
                }
                Ok(Response::TerminalLease(lease)) => {
                    known_sessions.write().insert(lease.session_id.clone());
                    start_terminal_data_channel(lease.session_id.clone(), context);
                }
                Ok(Response::TerminalViewport(read)) | Ok(Response::TerminalScrolled(read)) => {
                    let session_id = read.viewport.session_id.clone();
                    known_sessions.write().insert(session_id.clone());
                    mirrors
                        .write()
                        .insert(session_id, TerminalMirror::new(read.viewport.clone()));
                    let _ = events.send(ClientEvent::TerminalUpdated(Arc::new(
                        TerminalStreamUpdate::Snapshot(read.viewport.clone()),
                    )));
                }
                Ok(Response::Resources(resources)) => {
                    catalog.write().replace(Arc::new(resources.clone()));
                }
                Ok(Response::AgentSnapshots(snapshots)) => {
                    apply_agent_snapshots(snapshots.clone(), agent_snapshots, events);
                }
                _ => {}
            }
            let _ = reply.send(result.map_err(ClientCoreError::Protocol));
            Vec::new()
        }
        PendingRequest::Checkpoint(session_id) => {
            if let Ok(Response::TerminalCheckpoint(checkpoint)) = result {
                known_sessions.write().insert(session_id.clone());
                let update = TerminalStreamUpdate::Snapshot(checkpoint.viewport.clone());
                mirrors
                    .write()
                    .insert(session_id, TerminalMirror::new(checkpoint.viewport));
                let _ = events.send(ClientEvent::TerminalUpdated(Arc::new(update)));
            }
            Vec::new()
        }
        PendingRequest::AgentSnapshots => {
            if let Ok(Response::AgentSnapshots(snapshots)) = result {
                apply_agent_snapshots(snapshots, agent_snapshots, events);
            }
            Vec::new()
        }
        PendingRequest::Catalog => {
            let Ok(Response::Resources(resources)) = result else {
                return Vec::new();
            };
            let resources = Arc::new(resources);
            catalog.write().replace(resources.clone());
            let active = resources
                .terminals
                .iter()
                .map(|placement| placement.session_id.clone())
                .collect::<HashSet<_>>();
            let removed = {
                let mut known = known_sessions.write();
                let removed = known
                    .iter()
                    .filter(|session_id| !active.contains(*session_id))
                    .cloned()
                    .collect::<Vec<_>>();
                *known = active.clone();
                removed
            };
            mirrors
                .write()
                .retain(|session_id, _| active.contains(session_id));
            let sessions = resources
                .terminals
                .iter()
                .map(|placement| placement.session_id.clone())
                .collect::<Vec<_>>();
            for session_id in removed {
                let _ = events.send(ClientEvent::TerminalUnavailable(session_id));
            }
            sessions
        }
    }
}

fn start_terminal_data_channel(session_id: TerminalSessionId, context: &ClientSessionContext) {
    let connector = context.connector.clone();
    let mut identity = context.identity.clone();
    let token = context.token.clone();
    let mirrors = context.mirrors.clone();
    let known_sessions = context.known_sessions.clone();
    let events = context.events.clone();
    let data_channels = context.data_channels.clone();
    let checkpoint_requests = context.checkpoint_requests.clone();
    let diagnostics = context.diagnostics.clone();
    let mut shutdown = context.shutdown.clone();
    let mut channels = data_channels.write();
    if channels
        .get(&session_id)
        .is_some_and(|desired| desired.send(true).is_ok())
    {
        return;
    }
    channels.remove(&session_id);
    let (desired, mut desired_rx) = watch::channel(true);
    channels.insert(session_id.clone(), desired);
    drop(channels);
    identity.channel = ConnectionChannel::TerminalData;
    identity.can_force_stop = false;
    identity.terminal_session_id = Some(session_id.clone());
    let token = token.clone();
    tokio::spawn(async move {
        let mut attempt = 0_u32;
        'worker: loop {
            if *shutdown.borrow() {
                break;
            }
            while !*desired_rx.borrow() {
                tokio::select! {
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
            }
            let (mut stream, _) = match establish(&connector, &identity, &token).await {
                Ok(connection) => {
                    attempt = 0;
                    connection
                }
                Err(_) => {
                    attempt = attempt.saturating_add(1);
                    let delay = Duration::from_millis((50_u64 << attempt.min(5)).min(1_000));
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
                        diagnostics.record_ipc_read(observation);
                        let ControlMessage::Event(event) = message else {
                            break;
                        };
                        let terminal_exited = match handle_terminal_data_event(
                            event,
                            &mirrors,
                            &known_sessions,
                            &events,
                            &checkpoint_requests,
                            &diagnostics,
                        )
                        .await
                        {
                            Ok(terminal_exited) => terminal_exited,
                            Err(()) => break 'worker,
                        };
                        if terminal_exited
                            && let Some(desired) = data_channels.read().get(&session_id)
                        {
                            let _ = desired.send(false);
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
            let delay = Duration::from_millis((50_u64 << attempt.min(5)).min(1_000));
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
        data_channels.write().remove(&session_id);
    });
}

fn handle_event(
    event: HostEvent,
    mirrors: &Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    known_sessions: &Arc<RwLock<HashSet<TerminalSessionId>>>,
    agent_snapshots: Option<&Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>>,
    events: &broadcast::Sender<ClientEvent>,
) -> Option<TerminalSessionId> {
    let update = match event.body {
        ServerEvent::AgentSnapshot(update) => {
            if let Some(agent_snapshots) = agent_snapshots {
                apply_agent_snapshots(vec![*update], agent_snapshots, events);
            }
            return None;
        }
        ServerEvent::Terminal(update) => update,
        ServerEvent::TerminalExit {
            session_id,
            session_epoch,
            final_sequence,
            ..
        } => {
            let final_viewport_is_ready = mirrors.read().get(&session_id).is_some_and(|mirror| {
                let viewport = mirror.viewport();
                viewport.session_epoch == session_epoch
                    && viewport.sequence >= final_sequence
                    && matches!(viewport.process_state, TerminalProcessState::Exited { .. })
            });
            return (!final_viewport_is_ready).then_some(session_id);
        }
        _ => return None,
    };
    let session_id = update.session_id().clone();
    known_sessions.write().insert(session_id.clone());
    let published_update = Arc::new(update);
    let apply = {
        let mut store = mirrors.write();
        match store.get_mut(&session_id) {
            Some(mirror) => mirror.apply(published_update.as_ref()),
            None => match published_update.as_ref() {
                TerminalStreamUpdate::Snapshot(viewport) => {
                    store.insert(session_id.clone(), TerminalMirror::new(viewport.clone()));
                    MirrorApply::Updated
                }
                _ => MirrorApply::SequenceGap,
            },
        }
    };
    match apply {
        MirrorApply::Updated => {
            let _ = events.send(ClientEvent::TerminalUpdated(published_update));
            None
        }
        MirrorApply::Ignored => None,
        MirrorApply::SequenceGap => Some(session_id),
    }
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
    events: &broadcast::Sender<ClientEvent>,
) {
    for update in updates {
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

async fn handle_terminal_data_event(
    event: HostEvent,
    mirrors: &Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    known_sessions: &Arc<RwLock<HashSet<TerminalSessionId>>>,
    events: &broadcast::Sender<ClientEvent>,
    checkpoint_requests: &mpsc::Sender<TerminalSessionId>,
    diagnostics: &ClientPipelineDiagnostics,
) -> Result<bool, ()> {
    let terminal_exited = matches!(
        &event.body,
        ServerEvent::Terminal(TerminalStreamUpdate::Snapshot(viewport))
            if matches!(
                viewport.process_state,
                TerminalProcessState::Exited { .. }
            )
    ) || matches!(
        &event.body,
        ServerEvent::Terminal(TerminalStreamUpdate::Delta(delta))
            if matches!(
                &delta.process_state,
                Some(TerminalProcessState::Exited { .. })
            )
    );
    let merge_started_at = Instant::now();
    let checkpoint = handle_event(event, mirrors, known_sessions, None, events);
    diagnostics.record_terminal_merge(merge_started_at.elapsed());
    if let Some(session_id) = checkpoint {
        diagnostics.record_checkpoint_resync();
        checkpoint_requests.send(session_id).await.map_err(|_| ())?;
    }
    Ok(terminal_exited)
}

async fn send_checkpoint_request(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    pending: &mut HashMap<u64, PendingRequest>,
    next_request_id: &impl Fn() -> u64,
    actor_device_id: Option<String>,
    session_id: TerminalSessionId,
) -> Result<(), ()> {
    send_internal_request(
        writer,
        pending,
        next_request_id,
        actor_device_id,
        Request::RequestCheckpoint {
            session_id: session_id.clone(),
            after_sequence: None,
        },
        PendingRequest::Checkpoint(session_id),
    )
    .await
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

enum ConnectFailure {
    Retry(String),
    Fatal(String),
}

impl ConnectFailure {
    fn message(&self) -> &str {
        match self {
            Self::Retry(message) | Self::Fatal(message) => message,
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
    #[error("Host request timed out")]
    RequestTimeout,
    #[error("Host rejected request: {0:?}")]
    Protocol(ProtocolFailure),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn terminal_data_gap_queues_checkpoint_without_rebroadcasting_raw_event() {
        let session_id = TerminalSessionId::new("gap");
        let mirrors = Arc::new(RwLock::new(HashMap::new()));
        let known_sessions = Arc::new(RwLock::new(HashSet::new()));
        let (events, mut event_rx) = broadcast::channel(4);
        let (checkpoint_requests, mut checkpoints) = mpsc::channel(1);
        let diagnostics = ClientPipelineDiagnostics::default();
        let event = HostEvent {
            host_sequence: 1,
            body: ServerEvent::Terminal(TerminalStreamUpdate::ResyncRequired {
                session_id: session_id.clone(),
                available_from_sequence: 4,
            }),
        };

        assert!(
            !handle_terminal_data_event(
                event.clone(),
                &mirrors,
                &known_sessions,
                &events,
                &checkpoint_requests,
                &diagnostics,
            )
            .await
            .unwrap()
        );
        assert_eq!(checkpoints.recv().await, Some(session_id.clone()));
        assert!(known_sessions.read().contains(&session_id));
        assert!(
            tokio::time::timeout(Duration::from_millis(10), event_rx.recv())
                .await
                .is_err(),
            "terminal data must reach observers only through the merged mirror"
        );
    }
}
