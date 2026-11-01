#![forbid(unsafe_code)]

mod diagnostics;
mod mirror;

use std::{
    collections::{HashMap, HashSet},
    hash::{Hash, Hasher},
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
    io::{ReadHalf, split},
    sync::{broadcast, mpsc, oneshot, watch},
};
use yttt_core::model::{host_workspace::HostWorkspaceState, ids::TerminalSessionId};
use yttt_protocol::{
    ClientRequest, ConnectionChannel, ControlMessage, HostEvent, ProtocolFailure, Request,
    ResourceCatalog, Response, ServerEvent,
    agent::{AgentSnapshotCursor, AgentSnapshotUpdate, ProjectTokenUsage},
    terminal::{TerminalInput, TerminalStreamUpdate},
    workspace::{WorkspaceId, WorkspaceLayoutUpdate, WorkspaceRequest, WorkspaceResponse},
};
use yttt_transport::{
    AuthToken, AuthenticatedHost, ClientIdentity, HandshakeError, SessionReader, SessionWriter,
    SessionWriterTask, SharedConnector, TransportConnector, TransportError, TransportStream,
    client_handshake,
};

const COMMAND_CAPACITY: usize = 256;
const EVENT_CAPACITY: usize = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
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
    AgentSnapshotsReset {
        host_epoch: u64,
    },
    AgentSnapshotUpdated(Box<AgentSnapshotUpdate>),
    /// The Host dropped this pane's Agent state.
    AgentStateCleared(TerminalSessionId),
    ProjectTokenUsageUpdated(Box<ProjectTokenUsage>),
    /// A workspace this Client opened has a newer Host layout.
    WorkspaceLayoutChanged(Box<WorkspaceLayoutUpdate>),
}

#[derive(Default)]
struct TerminalState {
    host_epoch: Option<u64>,
    mirrors: HashMap<TerminalSessionId, TerminalMirror>,
    known_sessions: HashSet<TerminalSessionId>,
    /// Terminals this client attached. The Host forgets attachments when the
    /// connection drops, so each new connection subscribes to them again.
    attached: HashSet<TerminalSessionId>,
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
    workspace_layouts: WorkspaceLayouts,
    next_request_id: Arc<AtomicU64>,
    shutdown: watch::Sender<bool>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
    client_id: yttt_core::model::ids::ClientInstanceId,
    visibility_scopes: Mutex<VisibilityScopes>,
    visibility: watch::Sender<Option<Arc<VisibleTerminals>>>,
}

impl Drop for ClientCoreInner {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
        if let Some(supervisor) = self.supervisor.lock().take() {
            supervisor.abort();
        }
    }
}

struct ClientCommand {
    request_id: Option<u64>,
    body: Request,
    reply: Option<oneshot::Sender<Result<Response, ClientCoreError>>>,
    deadline: tokio::time::Instant,
}

pub struct PendingClientRequest {
    response: oneshot::Receiver<Result<Response, ClientCoreError>>,
    deadline: tokio::time::Instant,
}

impl PendingClientRequest {
    pub async fn wait(self) -> Result<Response, ClientCoreError> {
        tokio::time::timeout_at(self.deadline, self.response)
            .await
            .map_err(|_| ClientCoreError::RequestTimeout)?
            .map_err(|_| ClientCoreError::SupervisorStopped)?
    }
}

/// The workspaces this Client opened, each with the newest Host layout it
/// received; `None` until the first one arrives, so a workspace is reopened on
/// a new connection even before its layout came back.
type WorkspaceLayouts = Arc<RwLock<HashMap<WorkspaceId, Option<HostWorkspaceState>>>>;

enum PendingRequest {
    User {
        reply: oneshot::Sender<Result<Response, ClientCoreError>>,
        /// Terminals the request detaches once it succeeds.
        detaches: Vec<TerminalSessionId>,
        /// The workspace the request opens, whose layout the Host then sends.
        opens: Option<WorkspaceId>,
    },
    Catalog,
    AgentSnapshots,
    Visibility,
    /// Resubscribes to an attached terminal on a new connection.
    Resubscribe(TerminalSessionId),
    /// Reopens a workspace on a new connection, which subscribes to its layout.
    Reopen(WorkspaceId),
}

struct ClientSessionContext {
    catalog_requests: mpsc::Sender<()>,
    events: broadcast::Sender<ClientEvent>,
    terminal_state: Arc<RwLock<TerminalState>>,
    identity: ClientIdentity,
    host_epoch: u64,
    current_host_epoch: Arc<RwLock<Option<u64>>>,
    catalog: Arc<RwLock<Option<Arc<ResourceCatalog>>>>,
    agent_snapshots: Arc<RwLock<HashMap<String, AgentSnapshotUpdate>>>,
    project_token_usage: Arc<RwLock<HashMap<String, ProjectTokenUsage>>>,
    workspace_layouts: WorkspaceLayouts,
    next_request_id: Arc<AtomicU64>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
    visibility: watch::Receiver<Option<Arc<VisibleTerminals>>>,
    shutdown: watch::Receiver<bool>,
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
        let workspace_layouts = WorkspaceLayouts::default();
        let next_request_id = Arc::new(AtomicU64::new(1));
        let diagnostics = Arc::new(ClientPipelineDiagnostics::default());
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
            workspace_layouts.clone(),
            next_request_id.clone(),
            diagnostics.clone(),
            shutdown.clone(),
            initial_tx,
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
            workspace_layouts,
            shutdown,
            diagnostics,
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

    /// The epoch of the Host this Client last connected to; it stays while
    /// the Client reconnects.
    pub fn host_epoch(&self) -> Option<u64> {
        self.inner.terminal_state.read().host_epoch
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

    /// The newest layout received for a workspace this Client opened.
    pub fn workspace_layout(&self, workspace_id: &WorkspaceId) -> Option<HostWorkspaceState> {
        self.inner
            .workspace_layouts
            .read()
            .get(workspace_id)
            .and_then(|held| held.clone())
    }

    /// The revision of the newest layout received for a workspace this Client
    /// opened.
    pub fn workspace_layout_revision(&self, workspace_id: &WorkspaceId) -> Option<u64> {
        self.inner
            .workspace_layouts
            .read()
            .get(workspace_id)
            .and_then(|held| held.as_ref().map(|layout| layout.revision))
    }

    pub fn enqueue_request(&self, body: Request) -> Result<PendingClientRequest, ClientCoreError> {
        if !matches!(self.state(), ConnectionState::Ready { .. }) {
            return Err(ClientCoreError::NotConnected);
        }
        let timeout = Self::request_timeout(&body);
        let (reply, response) = oneshot::channel();
        self.enqueue_command(body, Some(reply))?;
        Ok(PendingClientRequest {
            response,
            deadline: tokio::time::Instant::now() + timeout,
        })
    }

    /// Queue terminal input ahead of other requests without allocating a
    /// per-input response waiter.
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
    workspace_layouts: WorkspaceLayouts,
    next_request_id: Arc<AtomicU64>,
    diagnostics: Arc<ClientPipelineDiagnostics>,
    shutdown_tx: watch::Sender<bool>,
    initial: oneshot::Sender<Result<(), String>>,
) {
    let _shutdown_guard = ShutdownOnDrop(shutdown_tx.clone());
    let mut shutdown = shutdown_tx.subscribe();
    let mut initial_attempt = 0;
    let mut connection = loop {
        set_state(&state, &events, ConnectionState::Connecting);
        diagnostics.record_connection_attempt();
        let result = tokio::select! {
            result = establish_session(&connector, &identity, &token) => result,
            _ = shutdown.wait_for(|stopped| *stopped) => return,
        };
        match result {
            Ok(established) => break established,
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
        workspace_layouts: &workspace_layouts,
        events: &events,
    };
    install_host_epoch(connection.host.host_epoch, &epoch_state);
    identity.host_epoch_hint = Some(connection.host.host_epoch);
    set_state(
        &state,
        &events,
        ConnectionState::Ready {
            host_epoch: connection.host.host_epoch,
            connection_sequence: connection.host.connection_sequence,
        },
    );
    let _ = initial.send(Ok(()));
    let (catalog_requests, mut catalog_rx) = mpsc::channel(1);
    let mut attempt = 0_u32;
    loop {
        let session = ClientSessionContext {
            catalog_requests: catalog_requests.clone(),
            events: events.clone(),
            terminal_state: terminal_state.clone(),
            identity: identity.clone(),
            host_epoch: connection.host.host_epoch,
            current_host_epoch: host_epoch.clone(),
            catalog: catalog.clone(),
            agent_snapshots: agent_snapshots.clone(),
            project_token_usage: project_token_usage.clone(),
            workspace_layouts: workspace_layouts.clone(),
            next_request_id: next_request_id.clone(),
            diagnostics: diagnostics.clone(),
            visibility: visibility.clone(),
            shutdown: shutdown.clone(),
        };
        let disconnected = connected_session(
            connection,
            &mut commands,
            &mut interactive_commands,
            &mut catalog_rx,
            &session,
        )
        .await;
        let Some(message) = disconnected else {
            set_state(&state, &events, ConnectionState::Disconnected);
            return;
        };
        attempt = attempt.saturating_add(1);
        diagnostics.record_reconnect();
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
                result = establish_session(&connector, &identity, &token) => result,
                _ = shutdown.wait_for(|stopped| *stopped) => {
                    set_state(&state, &events, ConnectionState::Disconnected);
                    return;
                }
            };
            match result {
                Ok(new_connection) => {
                    connection = new_connection;
                    install_host_epoch(connection.host.host_epoch, &epoch_state);
                    identity.host_epoch_hint = Some(connection.host.host_epoch);
                    attempt = 0;
                    set_state(
                        &state,
                        &events,
                        ConnectionState::Ready {
                            host_epoch: connection.host.host_epoch,
                            connection_sequence: connection.host.connection_sequence,
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

/// The authenticated Session connection that carries all of a Client's traffic.
struct SessionConnection {
    reader: SessionReader<ReadHalf<TransportStream>>,
    writer: SessionWriter,
    writer_task: SessionWriterTask,
    host: AuthenticatedHost,
    /// Events that arrived before the binding was confirmed, applied once the
    /// session starts.
    buffered: Vec<HostEvent>,
}

/// Opens a Session connection and waits until the Host has bound it to this
/// Client session: the Host closes a connection that presents another
/// session's identity after the handshake, so only a served request proves it.
async fn establish_session(
    connector: &SharedConnector,
    identity: &ClientIdentity,
    token: &AuthToken,
) -> Result<SessionConnection, ConnectFailure> {
    const BIND_REQUEST_ID: u64 = u64::MAX;
    let mut session_identity = identity.clone();
    session_identity.channel = ConnectionChannel::Session;
    let (stream, host) = establish(connector, &session_identity, token).await?;
    let (reader, writer) = split(stream);
    let (writer, writer_task) = SessionWriter::spawn(writer);
    let mut reader = SessionReader::new(reader);
    writer
        .send_reliable(&ControlMessage::Request(ClientRequest::new(
            BIND_REQUEST_ID,
            Request::Ping { sent_millis: 0 },
        )))
        .map_err(|error| ConnectFailure::Retry(error.to_string()))?;
    let deadline = tokio::time::Instant::now() + REQUEST_TIMEOUT;
    let mut buffered = Vec::new();
    loop {
        let message = tokio::time::timeout_at(deadline, reader.receive())
            .await
            .map_err(|_| ConnectFailure::Retry("Host session binding timed out".to_string()))?
            .map_err(|error| ConnectFailure::Retry(error.to_string()))?;
        match message {
            ControlMessage::Response(response) if response.request_id == BIND_REQUEST_ID => {
                response
                    .result
                    .map_err(|failure| ConnectFailure::Fatal(failure.message))?;
                break;
            }
            ControlMessage::Event(event) => buffered.push(event),
            _ => {
                return Err(ConnectFailure::Fatal(
                    "Host sent an unexpected message while connecting".to_string(),
                ));
            }
        }
    }
    Ok(SessionConnection {
        reader,
        writer,
        writer_task,
        host,
        buffered,
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

/// Serves one Session connection until it fails or the client shuts down.
/// Returns why it disconnected, or `None` on shutdown.
async fn connected_session(
    connection: SessionConnection,
    commands: &mut mpsc::Receiver<ClientCommand>,
    interactive_commands: &mut mpsc::Receiver<ClientCommand>,
    catalog_rx: &mut mpsc::Receiver<()>,
    context: &ClientSessionContext,
) -> Option<String> {
    let SessionConnection {
        mut reader,
        writer,
        writer_task: _writer_task,
        buffered,
        ..
    } = connection;
    let mut shutdown = context.shutdown.clone();
    let mut pending = HashMap::new();
    let mut interactive_pending = HashMap::new();
    let mut visibility = context.visibility.clone();
    // The Host forgets a client's visibility when its connection drops, so every
    // session reports the current set until the Host declines the request.
    let mut visibility_supported = true;
    visibility.mark_changed();
    let next_internal_request_id = || context.next_request_id.fetch_add(1, Ordering::Relaxed);
    let actor = || Some(context.identity.client_instance_id.to_string());
    if send_internal_request(
        &writer,
        &mut pending,
        &next_internal_request_id,
        actor(),
        Request::ListResources,
        PendingRequest::Catalog,
    )
    .is_err()
    {
        return Some("failed to request Host resource catalog".to_string());
    }
    if send_internal_request(
        &writer,
        &mut pending,
        &next_internal_request_id,
        actor(),
        Request::ReadAgentSnapshots {
            acknowledged: agent_snapshot_cursors(context),
        },
        PendingRequest::AgentSnapshots,
    )
    .is_err()
    {
        return Some("failed to request Host Agent snapshots".to_string());
    }
    let attached = {
        let terminal_state = context.terminal_state.read();
        if terminal_state.host_epoch == Some(context.host_epoch) {
            terminal_state.attached.iter().cloned().collect::<Vec<_>>()
        } else {
            Vec::new()
        }
    };
    for session_id in attached {
        if send_internal_request(
            &writer,
            &mut pending,
            &next_internal_request_id,
            actor(),
            Request::RequestCheckpoint {
                session_id: session_id.clone(),
                after_sequence: None,
            },
            PendingRequest::Resubscribe(session_id),
        )
        .is_err()
        {
            return Some("failed to resubscribe to attached terminals".to_string());
        }
    }
    let reopened = context
        .workspace_layouts
        .read()
        .keys()
        .cloned()
        .collect::<Vec<_>>();
    for workspace_id in reopened {
        if send_internal_request(
            &writer,
            &mut pending,
            &next_internal_request_id,
            actor(),
            Request::Workspace(WorkspaceRequest::OpenExisting {
                workspace_id: workspace_id.clone(),
            }),
            PendingRequest::Reopen(workspace_id),
        )
        .is_err()
        {
            return Some("failed to reopen workspaces".to_string());
        }
    }
    for event in buffered {
        if handle_state_event(event, context).is_err() {
            return None;
        }
    }
    let (messages_tx, mut messages_rx) = mpsc::channel(COMMAND_CAPACITY);
    let _reader = AbortOnDrop(tokio::spawn(async move {
        loop {
            let message = reader
                .receive_observed()
                .await
                .map_err(|error| error.to_string());
            let failed = message.is_err();
            if messages_tx.send(message).await.is_err() || failed {
                break;
            }
        }
    }));
    let mut heartbeat = None::<(u64, tokio::time::Instant)>;
    let mut heartbeat_tick = tokio::time::interval_at(
        tokio::time::Instant::now() + HEARTBEAT_INTERVAL,
        HEARTBEAT_INTERVAL,
    );
    heartbeat_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut cleanup = tokio::time::interval(Duration::from_secs(1));
    cleanup.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let disconnected = 'connected: loop {
        tokio::select! {
            biased;
            changed = shutdown.changed() => {
                if changed.is_err() || *shutdown.borrow() {
                    break None;
                }
            }
            command = interactive_commands.recv(), if interactive_pending.len() < COMMAND_CAPACITY => {
                let Some(ClientCommand {
                    request_id,
                    body,
                    reply,
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
                        ControlMessage::TerminalInput { input }
                    }
                    (
                        body @ (
                            Request::TerminalInput(_)
                            | Request::ResizeTerminal(_)
                            | Request::ScrollTerminal(_)
                        ),
                        Some(request_id),
                    ) => ControlMessage::Request(ClientRequest {
                        request_id,
                        actor_device_id: actor(),
                        lease_epoch: None,
                        body,
                    }),
                    (_, _) => {
                        if let Some(reply) = reply {
                            let _ = reply.send(Err(ClientCoreError::Connection(
                                "non-interactive request was routed as interactive".to_string(),
                            )));
                        }
                        continue;
                    }
                };
                if let Err(error) = writer.send_interactive(&message) {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(ClientCoreError::Connection(error.to_string())));
                    }
                    break Some(error.to_string());
                }
                if let (Some(request_id), Some(reply)) = (request_id, reply) {
                    interactive_pending.insert(request_id, reply);
                }
            }
            message = messages_rx.recv() => {
                let (message, observation) = match message {
                    Some(Ok(message)) => message,
                    Some(Err(error)) => break Some(error),
                    None => break Some("Host session reader stopped".to_string()),
                };
                match message {
                    ControlMessage::Response(response) => {
                        if heartbeat.is_some_and(|(id, _)| id == response.request_id) {
                            if !matches!(&response.result, Ok(Response::Pong { .. })) {
                                break Some("Host rejected heartbeat".into());
                            }
                            if let Some((_, deadline)) = heartbeat.take() {
                                context.diagnostics.record_heartbeat(tokio::time::Instant::now().saturating_duration_since(deadline - HEARTBEAT_TIMEOUT));
                            }
                            continue;
                        }
                        if let Some(reply) = interactive_pending.remove(&response.request_id) {
                            handle_response(PendingRequest::User { reply, detaches: Vec::new(), opens: None }, response.result, context);
                            continue;
                        }
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
                        if more_agents && send_internal_request(
                            &writer, &mut pending, &next_internal_request_id, actor(),
                            Request::ReadAgentSnapshots { acknowledged: agent_snapshot_cursors(context) },
                            PendingRequest::AgentSnapshots,
                        ).is_err() {
                            break 'connected Some("failed to read next Agent snapshot page".to_string());
                        }
                    }
                    ControlMessage::Event(HostEvent { body: ServerEvent::Terminal(update), .. }) => {
                        context.diagnostics.record_ipc_read(observation);
                        // A gap means the Host broke its stream contract; the
                        // next connection starts every stream from a Snapshot.
                        if handle_terminal_frame(update, context).is_err() {
                            break Some("terminal stream sequence gap".to_string());
                        }
                    }
                    ControlMessage::Event(event) => {
                        if handle_state_event(event, context).is_err() {
                            break None;
                        }
                    }
                    ControlMessage::Request(_)
                    | ControlMessage::TerminalInput { .. }
                    | ControlMessage::Chunk(_) => {
                        break Some("Host sent a client message on the session".to_string());
                    }
                }
            }
            command = commands.recv(), if pending.len() < COMMAND_CAPACITY => {
                let Some(ClientCommand {
                    request_id,
                    body,
                    reply,
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
                            "one-way command was routed as a request".to_string(),
                        )));
                    }
                    continue;
                };
                let detaches = detached_terminals(&body);
                let opens = opened_workspace(&body);
                let message = ControlMessage::Request(ClientRequest {
                    request_id,
                    actor_device_id: actor(),
                    lease_epoch: None,
                    body,
                });
                if let Err(error) = writer.send_reliable(&message) {
                    if let Some(reply) = reply {
                        let _ = reply.send(Err(ClientCoreError::Connection(error.to_string())));
                    }
                    break Some(error.to_string());
                }
                if let Some(reply) = reply {
                    pending.insert(request_id, PendingRequest::User { reply, detaches, opens });
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
                    &writer,
                    &mut pending,
                    &next_internal_request_id,
                    actor(),
                    Request::ListResources,
                    PendingRequest::Catalog,
                )
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
                    &writer,
                    &mut pending,
                    &next_internal_request_id,
                    actor(),
                    Request::SetTerminalVisibility {
                        revision: visible.revision,
                        visible: visible.sessions.clone(),
                    },
                    PendingRequest::Visibility,
                )
                .is_err()
                {
                    break Some("failed to report terminal visibility".to_string());
                }
            }
            _ = heartbeat_tick.tick() => {
                if let Some((_, deadline)) = heartbeat {
                    if tokio::time::Instant::now() >= deadline {
                        break Some("Host heartbeat timed out".into());
                    }
                    continue;
                }
                let request_id = context.next_request_id.fetch_add(1, Ordering::Relaxed);
                let ping = ControlMessage::Request(ClientRequest::new(
                    request_id,
                    Request::Ping { sent_millis: request_id },
                ));
                if let Err(error) = writer.send_interactive(&ping) {
                    break Some(error.to_string());
                }
                heartbeat = Some((request_id, tokio::time::Instant::now() + HEARTBEAT_TIMEOUT));
            }
            _ = cleanup.tick() => {
                pending.retain(|_, request| !matches!(request, PendingRequest::User { reply, .. } if reply.is_closed()));
                interactive_pending.retain(|_, reply: &mut oneshot::Sender<Result<Response, ClientCoreError>>| !reply.is_closed());
            }
            () = writer.closed() => break Some("Host session writer closed".to_string()),
        }
    };
    for (_, request) in pending {
        if let PendingRequest::User { reply, .. } = request {
            let _ = reply.send(Err(ClientCoreError::NotConnected));
        }
    }
    for (_, reply) in interactive_pending {
        let _ = reply.send(Err(ClientCoreError::NotConnected));
    }
    disconnected
}

fn agent_snapshot_cursors(context: &ClientSessionContext) -> Vec<AgentSnapshotCursor> {
    context
        .agent_snapshots
        .read()
        .values()
        .map(|update| AgentSnapshotCursor {
            scope: update.scope.clone(),
            host_epoch: update.host_epoch,
            sequence: update.sequence,
        })
        .collect()
}

/// Applies a Host state event and publishes it. `Err` means the client is shutting down.
fn handle_state_event(event: HostEvent, context: &ClientSessionContext) -> Result<(), ()> {
    if *context.current_host_epoch.read() != Some(context.host_epoch) {
        return Ok(());
    }
    // Layouts publish only through the mirror, which drops stale revisions.
    let event = match event.body {
        ServerEvent::WorkspaceChanged(update) => {
            let WorkspaceLayoutUpdate {
                workspace_id,
                layout,
            } = *update;
            apply_workspace_layout(workspace_id, layout, context);
            return Ok(());
        }
        body => HostEvent {
            host_sequence: event.host_sequence,
            body,
        },
    };
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
            Err(mpsc::error::TrySendError::Closed(())) => return Err(()),
        }
    }
    if let ServerEvent::AgentStateCleared(update) = &event.body {
        context
            .agent_snapshots
            .write()
            .retain(|_, snapshot| snapshot.terminal_session_id != update.terminal_session_id);
        let _ = context.events.send(ClientEvent::AgentStateCleared(
            update.terminal_session_id.clone(),
        ));
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
    let _ = context.events.send(ClientEvent::Server(Box::new(event)));
    Ok(())
}

fn handle_response(
    pending: PendingRequest,
    result: Result<Response, ProtocolFailure>,
    context: &ClientSessionContext,
) {
    match pending {
        PendingRequest::User {
            reply,
            detaches,
            opens,
        } => {
            if let (
                Some(workspace_id),
                Ok(Response::Workspace(WorkspaceResponse::Opened { layout, .. })),
            ) = (opens, &result)
            {
                apply_workspace_layout(workspace_id, layout.clone(), context);
            }
            match &result {
                Ok(Response::TerminalSpawned { lease, .. })
                | Ok(Response::TerminalLease(lease)) => {
                    register_terminal(context, lease.session_id.clone());
                }
                Ok(Response::TerminalAttached { lease, checkpoint }) => {
                    register_terminal(context, lease.session_id.clone());
                    seed_terminal_mirror(
                        &checkpoint.viewport,
                        &context.terminal_state,
                        context.host_epoch,
                        &context.events,
                    );
                }
                Ok(Response::TerminalsTerminated { results }) => {
                    let mut terminal_state = context.terminal_state.write();
                    for result in results.iter().filter(|result| result.result.is_ok()) {
                        terminal_state.attached.remove(&result.session_id);
                    }
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
            if result.is_ok() && !detaches.is_empty() {
                let mut terminal_state = context.terminal_state.write();
                for session_id in &detaches {
                    terminal_state.attached.remove(session_id);
                }
            }
            let _ = reply.send(result.map_err(ClientCoreError::Protocol));
        }
        PendingRequest::Visibility => {}
        PendingRequest::Reopen(workspace_id) => match result {
            Ok(Response::Workspace(WorkspaceResponse::Opened { layout, .. })) => {
                apply_workspace_layout(workspace_id, layout, context);
            }
            // A workspace forgotten meanwhile is no longer opened.
            Err(failure) if failure.code == yttt_protocol::FailureCode::NotFound => {
                context.workspace_layouts.write().remove(&workspace_id);
            }
            _ => {}
        },
        PendingRequest::Resubscribe(session_id) => {
            // A terminal the Host no longer has stays unsubscribed; the catalog
            // reports it unavailable.
            if result.is_err() {
                context.terminal_state.write().attached.remove(&session_id);
            }
        }
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
                let TerminalState {
                    known_sessions,
                    mirrors,
                    attached,
                    ..
                } = &mut *terminal_state;
                mirrors.retain(|session_id, _| known_sessions.contains(session_id));
                attached.retain(|session_id| known_sessions.contains(session_id));
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

/// The workspace a `request` opens, which subscribes this Client to its layout.
fn opened_workspace(request: &Request) -> Option<WorkspaceId> {
    match request {
        Request::Workspace(
            WorkspaceRequest::Open { workspace_id }
            | WorkspaceRequest::OpenExisting { workspace_id },
        ) => Some(workspace_id.clone()),
        _ => None,
    }
}

/// Terminals whose Host attachment a successful `request` releases.
fn detached_terminals(request: &Request) -> Vec<TerminalSessionId> {
    match request {
        Request::DetachTerminal { session_id }
        | Request::TerminateTerminal { session_id, .. }
        | Request::AcknowledgeTerminalExit { session_id, .. } => vec![session_id.clone()],
        _ => Vec::new(),
    }
}

fn register_terminal(context: &ClientSessionContext, session_id: TerminalSessionId) {
    let mut terminal_state = context.terminal_state.write();
    if terminal_state.host_epoch == Some(context.host_epoch) {
        terminal_state.known_sessions.insert(session_id.clone());
        terminal_state.attached.insert(session_id);
    }
}

/// Installs an attach checkpoint as the initial mirror. Once a mirror exists only
/// the terminal's stream writes it, so frames are never interleaved from two sources.
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
    events: &broadcast::Sender<ClientEvent>,
) -> MirrorApply {
    let session_id = update.session_id().clone();
    let published_update = Arc::new(update);
    let mut state = terminal_state.write();
    if state.host_epoch != Some(host_epoch)
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
    drop(state);
    if apply == MirrorApply::Updated {
        let _ = events.send(ClientEvent::TerminalUpdated(published_update));
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

/// Keeps an opened workspace's Host layout when it is newer than the one this
/// Client holds, and publishes it.
fn apply_workspace_layout(
    workspace_id: WorkspaceId,
    layout: HostWorkspaceState,
    context: &ClientSessionContext,
) {
    let current_host_epoch = context.current_host_epoch.read();
    if *current_host_epoch != Some(context.host_epoch) {
        return;
    }
    let mut layouts = context.workspace_layouts.write();
    let held = layouts.entry(workspace_id.clone()).or_default();
    if held
        .as_ref()
        .is_some_and(|held| held.revision >= layout.revision)
    {
        return;
    }
    *held = Some(layout.clone());
    drop(layouts);
    let _ = context
        .events
        .send(ClientEvent::WorkspaceLayoutChanged(Box::new(
            WorkspaceLayoutUpdate {
                workspace_id,
                layout,
            },
        )));
}

/// Merges one terminal frame into its mirror. `Err` means the frame does not
/// continue the mirror.
fn handle_terminal_frame(
    update: TerminalStreamUpdate,
    context: &ClientSessionContext,
) -> Result<(), ()> {
    let merge_started_at = Instant::now();
    let apply = apply_terminal_update(
        update,
        &context.terminal_state,
        context.host_epoch,
        &context.events,
    );
    context
        .diagnostics
        .record_terminal_merge(merge_started_at.elapsed());
    if apply == MirrorApply::SequenceGap {
        return Err(());
    }
    Ok(())
}

fn send_internal_request(
    writer: &SessionWriter,
    pending: &mut HashMap<u64, PendingRequest>,
    next_request_id: &impl Fn() -> u64,
    actor_device_id: Option<String>,
    body: Request,
    request: PendingRequest,
) -> Result<(), ()> {
    let request_id = next_request_id();
    writer
        .send_reliable(&ControlMessage::Request(ClientRequest {
            request_id,
            actor_device_id,
            lease_epoch: None,
            body,
        }))
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
    workspace_layouts: &'a RwLock<HashMap<WorkspaceId, Option<HostWorkspaceState>>>,
    events: &'a broadcast::Sender<ClientEvent>,
}

fn install_host_epoch(next_host_epoch: u64, state: &HostEpochState<'_>) {
    let HostEpochState {
        current_host_epoch,
        terminal_state,
        catalog,
        agent_snapshots,
        project_token_usage,
        workspace_layouts,
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
            terminal_state.attached.clear();
            catalog.write().take();
            agent_snapshots.write().clear();
            project_token_usage.write().clear();
            // A new Host has new revisions; keep the opened set, forget the layouts.
            for held in workspace_layouts.write().values_mut() {
                *held = None;
            }
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
