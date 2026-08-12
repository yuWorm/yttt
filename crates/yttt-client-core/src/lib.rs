#![forbid(unsafe_code)]

mod mirror;

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
    time::Duration,
};

pub use mirror::{MirrorApply, TerminalMirror};
use parking_lot::RwLock;
use tokio::{
    io::split,
    sync::{broadcast, mpsc, oneshot, watch},
};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::{
    ClientRequest, ControlMessage, HostEvent, ProtocolFailure, Request, Response, ServerEvent,
    terminal::TerminalStreamUpdate,
};
use yttt_transport_local::{
    AuthToken, AuthenticatedHost, ClientIdentity, LocalEndpoint, LocalStream, client_handshake,
    connect, receive_control, send_control,
};

const COMMAND_CAPACITY: usize = 256;
const EVENT_CAPACITY: usize = 256;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
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
    MirrorUpdated(TerminalSessionId),
    TerminalUnavailable(TerminalSessionId),
}

pub struct ClientCore {
    inner: Arc<ClientCoreInner>,
}

struct ClientCoreInner {
    commands: mpsc::Sender<ClientCommand>,
    events: broadcast::Sender<ClientEvent>,
    state: watch::Receiver<ConnectionState>,
    mirrors: Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    known_sessions: Arc<RwLock<HashSet<TerminalSessionId>>>,
    shutdown: watch::Sender<bool>,
}

impl Drop for ClientCoreInner {
    fn drop(&mut self) {
        let _ = self.shutdown.send(true);
    }
}

struct ClientCommand {
    body: Request,
    reply: oneshot::Sender<Result<Response, ClientCoreError>>,
}

enum PendingRequest {
    User(oneshot::Sender<Result<Response, ClientCoreError>>),
    Checkpoint(TerminalSessionId),
    Catalog,
}

impl ClientCore {
    pub async fn connect(
        endpoint: LocalEndpoint,
        identity: ClientIdentity,
        token: AuthToken,
    ) -> Result<Self, ClientCoreError> {
        let (commands, command_rx) = mpsc::channel(COMMAND_CAPACITY);
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let (state_tx, state_rx) = watch::channel(ConnectionState::Disconnected);
        let (shutdown, shutdown_rx) = watch::channel(false);
        let mirrors = Arc::new(RwLock::new(HashMap::new()));
        let known_sessions = Arc::new(RwLock::new(HashSet::new()));
        let (initial_tx, initial_rx) = oneshot::channel();
        tokio::spawn(run_supervisor(
            endpoint,
            identity,
            token,
            command_rx,
            events.clone(),
            state_tx,
            mirrors.clone(),
            known_sessions.clone(),
            shutdown_rx,
            initial_tx,
        ));
        let initial = tokio::time::timeout(INITIAL_CONNECT_TIMEOUT, initial_rx)
            .await
            .map_err(|_| ClientCoreError::ConnectionTimeout)?
            .map_err(|_| ClientCoreError::SupervisorStopped)?;
        initial.map_err(ClientCoreError::Connection)?;
        Ok(Self {
            inner: Arc::new(ClientCoreInner {
                commands,
                events,
                state: state_rx,
                mirrors,
                known_sessions,
                shutdown,
            }),
        })
    }

    pub fn state(&self) -> ConnectionState {
        self.inner.state.borrow().clone()
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

    pub async fn request(&self, body: Request) -> Result<Response, ClientCoreError> {
        if !matches!(self.state(), ConnectionState::Ready { .. }) {
            return Err(ClientCoreError::NotConnected);
        }
        let timeout = Self::request_timeout(&body);
        let (reply, response) = oneshot::channel();
        self.inner
            .commands
            .try_send(ClientCommand { body, reply })
            .map_err(|error| match error {
                mpsc::error::TrySendError::Full(_) => ClientCoreError::Backpressure,
                mpsc::error::TrySendError::Closed(_) => ClientCoreError::SupervisorStopped,
            })?;
        tokio::time::timeout(timeout, response)
            .await
            .map_err(|_| ClientCoreError::RequestTimeout)?
            .map_err(|_| ClientCoreError::SupervisorStopped)?
    }

    fn request_timeout(request: &Request) -> Duration {
        match request {
            Request::RemoteFile(_) | Request::RemoteCommand(_) => REMOTE_REQUEST_TIMEOUT,
            _ => REQUEST_TIMEOUT,
        }
    }

    pub fn shutdown(&self) {
        let _ = self.inner.shutdown.send(true);
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_supervisor(
    endpoint: LocalEndpoint,
    mut identity: ClientIdentity,
    token: AuthToken,
    mut commands: mpsc::Receiver<ClientCommand>,
    events: broadcast::Sender<ClientEvent>,
    state: watch::Sender<ConnectionState>,
    mirrors: Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    known_sessions: Arc<RwLock<HashSet<TerminalSessionId>>>,
    mut shutdown: watch::Receiver<bool>,
    initial: oneshot::Sender<Result<(), String>>,
) {
    set_state(&state, &events, ConnectionState::Connecting);
    let (mut stream, host) = match establish(&endpoint, &identity, &token).await {
        Ok(connection) => connection,
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
    identity.host_epoch_hint = Some(host.host_epoch);
    set_state(
        &state,
        &events,
        ConnectionState::Ready {
            host_epoch: host.host_epoch,
            connection_sequence: host.connection_sequence,
        },
    );
    let _ = initial.send(Ok(()));
    let mut attempt = 0_u32;
    loop {
        let disconnected = connected_session(
            stream,
            &mut commands,
            &events,
            &mirrors,
            &known_sessions,
            &mut shutdown,
        )
        .await;
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
            match establish(&endpoint, &identity, &token).await {
                Ok((new_stream, new_host)) => {
                    stream = new_stream;
                    attempt = 0;
                    set_state(
                        &state,
                        &events,
                        ConnectionState::Ready {
                            host_epoch: new_host.host_epoch,
                            connection_sequence: new_host.connection_sequence,
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

async fn establish(
    endpoint: &LocalEndpoint,
    identity: &ClientIdentity,
    token: &AuthToken,
) -> Result<(LocalStream, AuthenticatedHost), ConnectFailure> {
    let mut stream = connect(endpoint)
        .await
        .map_err(|error| ConnectFailure::Retry(error.to_string()))?;
    let host = client_handshake(&mut stream, identity, token)
        .await
        .map_err(|error| ConnectFailure::Fatal(error.to_string()))?;
    Ok((stream, host))
}

async fn connected_session(
    stream: LocalStream,
    commands: &mut mpsc::Receiver<ClientCommand>,
    events: &broadcast::Sender<ClientEvent>,
    mirrors: &Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    known_sessions: &Arc<RwLock<HashSet<TerminalSessionId>>>,
    shutdown: &mut watch::Receiver<bool>,
) -> Option<String> {
    let (mut reader, mut writer) = split(stream);
    let mut pending = HashMap::new();
    let mut checkpoint_pending = HashSet::new();
    let mut next_request_id = 1_u64;
    if send_internal_request(
        &mut writer,
        &mut pending,
        &mut next_request_id,
        Request::ListResources,
        PendingRequest::Catalog,
    )
    .await
    .is_err()
    {
        return Some("failed to request Host resource catalog".to_string());
    }

    let disconnected = 'connected: loop {
        tokio::select! {
            command = commands.recv() => {
                let Some(command) = command else {
                    break None;
                };
                let request_id = next_request_id;
                next_request_id = next_request_id.saturating_add(1);
                let message = ControlMessage::Request(ClientRequest {
                    request_id,
                    body: command.body,
                });
                if let Err(error) = send_control(&mut writer, &message).await {
                    let _ = command.reply.send(Err(ClientCoreError::Connection(error.to_string())));
                    break Some(error.to_string());
                }
                pending.insert(request_id, PendingRequest::User(command.reply));
            }
            message = receive_control(&mut reader) => {
                let message = match message {
                    Ok(message) => message,
                    Err(error) => break Some(error.to_string()),
                };
                match message {
                    ControlMessage::Response(response) => {
                        let Some(pending_request) = pending.remove(&response.request_id) else {
                            continue;
                        };
                        if let PendingRequest::Checkpoint(session_id) = &pending_request {
                            checkpoint_pending.remove(session_id);
                        }
                        let checkpoints = handle_response(
                            pending_request,
                            response.result,
                            mirrors,
                            events,
                            known_sessions,
                        );
                        for session_id in checkpoints {
                            if checkpoint_pending.insert(session_id.clone())
                                && send_checkpoint_request(
                                    &mut writer,
                                    &mut pending,
                                    &mut next_request_id,
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
                    ControlMessage::Event(event) => {
                        let checkpoint =
                            handle_event(event.clone(), mirrors, known_sessions, events);
                        let _ = events.send(ClientEvent::Server(event));
                        if let Some(session_id) = checkpoint
                            && checkpoint_pending.insert(session_id.clone())
                            && send_checkpoint_request(
                                &mut writer,
                                &mut pending,
                                &mut next_request_id,
                                session_id,
                            )
                            .await
                            .is_err()
                        {
                            break Some("failed to request terminal checkpoint".to_string());
                        }
                    }
                    ControlMessage::Request(_) => {
                        break Some("Host sent a client request on the response stream".to_string());
                    }
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

fn handle_response(
    pending: PendingRequest,
    result: Result<Response, ProtocolFailure>,
    mirrors: &Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    events: &broadcast::Sender<ClientEvent>,
    known_sessions: &Arc<RwLock<HashSet<TerminalSessionId>>>,
) -> Vec<TerminalSessionId> {
    match pending {
        PendingRequest::User(reply) => {
            match &result {
                Ok(Response::TerminalSpawned { session_id, .. }) => {
                    known_sessions.write().insert(session_id.clone());
                }
                Ok(Response::TerminalAttached { lease, checkpoint }) => {
                    known_sessions.write().insert(lease.session_id.clone());
                    mirrors.write().insert(
                        lease.session_id.clone(),
                        TerminalMirror::new(checkpoint.viewport.clone()),
                    );
                    let _ = events.send(ClientEvent::MirrorUpdated(lease.session_id.clone()));
                }
                _ => {}
            }
            let _ = reply.send(result.map_err(ClientCoreError::Protocol));
            Vec::new()
        }
        PendingRequest::Checkpoint(session_id) => {
            if let Ok(Response::TerminalCheckpoint(checkpoint)) = result {
                known_sessions.write().insert(session_id.clone());
                mirrors
                    .write()
                    .insert(session_id.clone(), TerminalMirror::new(checkpoint.viewport));
                let _ = events.send(ClientEvent::MirrorUpdated(session_id));
            }
            Vec::new()
        }
        PendingRequest::Catalog => {
            let Ok(Response::Resources(catalog)) = result else {
                return Vec::new();
            };
            let active = catalog
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
            let mut sessions = Vec::with_capacity(catalog.terminals.len());
            let mut store = mirrors.write();
            store.retain(|session_id, _| active.contains(session_id));
            for placement in catalog.terminals {
                sessions.push(placement.session_id.clone());
                if let Some(viewport) = placement.viewport {
                    store.insert(placement.session_id.clone(), TerminalMirror::new(viewport));
                    let _ = events.send(ClientEvent::MirrorUpdated(placement.session_id));
                }
            }
            drop(store);
            for session_id in removed {
                let _ = events.send(ClientEvent::TerminalUnavailable(session_id));
            }
            sessions
        }
    }
}

fn handle_event(
    event: HostEvent,
    mirrors: &Arc<RwLock<HashMap<TerminalSessionId, TerminalMirror>>>,
    known_sessions: &Arc<RwLock<HashSet<TerminalSessionId>>>,
    events: &broadcast::Sender<ClientEvent>,
) -> Option<TerminalSessionId> {
    let ServerEvent::Terminal(update) = event.body else {
        return None;
    };
    let session_id = update_session_id(&update).clone();
    known_sessions.write().insert(session_id.clone());
    let apply = {
        let mut store = mirrors.write();
        match store.get_mut(&session_id) {
            Some(mirror) => mirror.apply(update),
            None => match update {
                TerminalStreamUpdate::Snapshot(viewport) => {
                    store.insert(session_id.clone(), TerminalMirror::new(viewport));
                    MirrorApply::Updated
                }
                _ => MirrorApply::SequenceGap,
            },
        }
    };
    match apply {
        MirrorApply::Updated => {
            let _ = events.send(ClientEvent::MirrorUpdated(session_id));
            None
        }
        MirrorApply::Ignored => None,
        MirrorApply::SequenceGap => Some(session_id),
    }
}

async fn send_checkpoint_request(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    pending: &mut HashMap<u64, PendingRequest>,
    next_request_id: &mut u64,
    session_id: TerminalSessionId,
) -> Result<(), ()> {
    send_internal_request(
        writer,
        pending,
        next_request_id,
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
    next_request_id: &mut u64,
    body: Request,
    request: PendingRequest,
) -> Result<(), ()> {
    let request_id = *next_request_id;
    *next_request_id = next_request_id.saturating_add(1);
    send_control(
        writer,
        &ControlMessage::Request(ClientRequest { request_id, body }),
    )
    .await
    .map_err(|_| ())?;
    pending.insert(request_id, request);
    Ok(())
}

fn update_session_id(update: &TerminalStreamUpdate) -> &TerminalSessionId {
    match update {
        TerminalStreamUpdate::Snapshot(viewport) => &viewport.session_id,
        TerminalStreamUpdate::Delta(delta) => &delta.session_id,
        TerminalStreamUpdate::RawTail { session_id, .. }
        | TerminalStreamUpdate::ResyncRequired { session_id, .. } => session_id,
    }
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
