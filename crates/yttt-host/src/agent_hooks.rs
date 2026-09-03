use std::{
    collections::{BTreeMap, HashMap},
    io::{self, Read as _, Write as _},
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex as StdMutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
    },
    thread::{self, JoinHandle},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use parking_lot::Mutex;
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use tokio::sync::broadcast;
use yttt_agent_core::{
    AGENT_ACTIVITY_STALE_AFTER_MILLIS, AgentEventKind, AgentExitReason, AgentInstanceId,
    AgentProcessExit, AgentProcessState, AgentProvider, AgentReducer, ProviderId,
};
use yttt_agent_providers::{CLAUDE_PROVIDER_ID, GROK_PROVIDER_ID, builtin_providers};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::{
    agent::{
        AGENT_HOOK_DELIVERY_PROTOCOL, AgentHookAcknowledgement, AgentHookDelivery, AgentHookScope,
        AgentSnapshotCursor, AgentSnapshotUpdate,
    },
    terminal::TerminalSpawnSpec,
};

use crate::agent_processes::DetectedAgentProcess;

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(1);
const ACCEPT_IDLE: Duration = Duration::from_millis(10);
const EVENT_CAPACITY: usize = 64;
const MAX_AGENT_RECORDS: usize = 256;
const AGENT_PROCESS_MISSED_SAMPLES_BEFORE_EXIT: u8 = 2;
const MAX_PENDING_DELIVERIES: usize = 256;
const MAX_RETIRED_DELIVERY_STREAMS: usize = 4;
const CONNECTION_WORKERS: usize = 4;
const CONNECTION_QUEUE_CAPACITY: usize = 64;
const TOKEN_HEADER: &str = "x-yttt-agent-hook-token";
const SCOPE_HEADER: &str = "x-yttt-agent-hook-scope";
const ENVIRONMENT_VARIABLES: [&str; 3] = [
    "YTTT_AGENT_HOOK_ENDPOINT",
    "YTTT_AGENT_HOOK_TOKEN",
    "YTTT_AGENT_HOOK_SCOPE",
];

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct AgentAddress {
    project_id: String,
    tab_id: String,
    pane_id: String,
}

impl From<&AgentHookScope> for AgentAddress {
    fn from(scope: &AgentHookScope) -> Self {
        Self {
            project_id: scope.project_id.clone(),
            tab_id: scope.tab_id.clone(),
            pane_id: scope.pane_id.clone(),
        }
    }
}

struct AgentRecord {
    scope: AgentHookScope,
    terminal_session_id: TerminalSessionId,
    sequence: u64,
    reducer: AgentReducer,
    delivery: Option<AgentDeliveryState>,
    terminal_exited: bool,
}

#[derive(Clone)]
struct AgentProcessObservation {
    scope: AgentHookScope,
    process: DetectedAgentProcess,
    missed_samples: u8,
}

enum AgentProcessAction {
    Start {
        terminal_session_id: TerminalSessionId,
        scope: AgentHookScope,
        provider_id: ProviderId,
    },
    Finish {
        scope: AgentHookScope,
        provider_id: ProviderId,
        reason: AgentExitReason,
    },
}

struct AgentDeliveryState {
    stream_id: String,
    accepted_sequence: u64,
    pending: BTreeMap<u64, Vec<AgentEventKind>>,
    retired: Vec<(String, u64)>,
}

struct HookDeliveryMetadata {
    stream_id: String,
    sequence: u64,
}

struct AgentState {
    host_epoch: u64,
    secret: Arc<[u8; 32]>,
    providers: HashMap<String, Arc<dyn AgentProvider>>,
    bindings: Mutex<HashMap<AgentHookScope, TerminalSessionId>>,
    records: Mutex<HashMap<AgentAddress, AgentRecord>>,
    terminal_exits: Mutex<HashMap<AgentHookScope, AgentProcessExit>>,
    process_observations: Mutex<HashMap<TerminalSessionId, AgentProcessObservation>>,
    events: broadcast::Sender<AgentSnapshotUpdate>,
}

pub struct HostAgentHookRuntime {
    endpoint: Arc<str>,
    state: Arc<AgentState>,
    next_resource_epoch: AtomicU64,
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
    worker_threads: Vec<JoinHandle<()>>,
}

impl HostAgentHookRuntime {
    pub fn start(host_epoch: u64) -> io::Result<Arc<Self>> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let first = uuid::Uuid::new_v4();
        let second = uuid::Uuid::new_v4();
        let mut secret = [0_u8; 32];
        secret[..16].copy_from_slice(first.as_bytes());
        secret[16..].copy_from_slice(second.as_bytes());
        let secret = Arc::new(secret);
        let endpoint: Arc<str> = format!("http://127.0.0.1:{port}").into();
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        let providers = builtin_providers()
            .into_iter()
            .map(|provider| (provider.descriptor().id.to_string(), provider))
            .collect();
        let state = Arc::new(AgentState {
            host_epoch,
            secret,
            providers,
            bindings: Mutex::new(HashMap::new()),
            records: Mutex::new(HashMap::new()),
            terminal_exits: Mutex::new(HashMap::new()),
            process_observations: Mutex::new(HashMap::new()),
            events,
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let (connection_tx, connection_rx) = mpsc::sync_channel(CONNECTION_QUEUE_CAPACITY);
        let connection_rx = Arc::new(StdMutex::new(connection_rx));
        let mut worker_threads = Vec::with_capacity(CONNECTION_WORKERS);
        for index in 0..CONNECTION_WORKERS {
            let worker_state = state.clone();
            let worker_shutdown = shutdown.clone();
            let worker_rx = connection_rx.clone();
            worker_threads.push(
                thread::Builder::new()
                    .name(format!("yttt-host-agent-hook-{index}"))
                    .spawn(move || connection_loop(worker_rx, worker_state, worker_shutdown))?,
            );
        }
        let thread_shutdown = shutdown.clone();
        let accept_thread = thread::Builder::new()
            .name("yttt-host-agent-hooks".to_string())
            .spawn(move || accept_loop(listener, connection_tx, thread_shutdown))?;
        Ok(Arc::new(Self {
            endpoint,
            state,
            next_resource_epoch: AtomicU64::new(1),
            shutdown,
            accept_thread: Some(accept_thread),
            worker_threads,
        }))
    }

    pub fn secure_terminal_environment(&self, spec: &mut TerminalSpawnSpec) -> AgentHookScope {
        spec.environment
            .retain(|(name, _)| !ENVIRONMENT_VARIABLES.contains(&name.as_str()));
        spec.removed_environment
            .retain(|name| !ENVIRONMENT_VARIABLES.contains(&name.as_str()));

        let scope = AgentHookScope {
            project_id: spec.project_id.to_string(),
            tab_id: spec.session_id.as_str().to_string(),
            pane_id: spec.session_id.as_str().to_string(),
            generation: self.next_resource_epoch.fetch_add(1, Ordering::Relaxed),
        };
        let encoded_scope = serde_json::to_vec(&scope)
            .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
            .expect("Agent hook scope must serialize");
        let token = scope_token(&self.state.secret, &encoded_scope);
        spec.environment.extend([
            (
                ENVIRONMENT_VARIABLES[0].to_string(),
                self.endpoint.to_string(),
            ),
            (ENVIRONMENT_VARIABLES[1].to_string(), token),
            (ENVIRONMENT_VARIABLES[2].to_string(), encoded_scope),
        ]);

        let address = AgentAddress::from(&scope);
        self.state
            .terminal_exits
            .lock()
            .retain(|candidate, _| AgentAddress::from(candidate) != address);
        self.state.records.lock().remove(&address);
        self.state
            .process_observations
            .lock()
            .remove(&spec.session_id);
        let mut bindings = self.state.bindings.lock();
        bindings.retain(|candidate, _| AgentAddress::from(candidate) != address);
        bindings.insert(scope.clone(), spec.session_id.clone());
        scope
    }

    pub fn cancel_terminal(&self, scope: &AgentHookScope) {
        self.state.bindings.lock().remove(scope);
        self.state.terminal_exits.lock().remove(scope);
        self.state
            .process_observations
            .lock()
            .retain(|_, observation| observation.scope != *scope);
    }

    pub fn terminal_exited(&self, session_id: &TerminalSessionId, code: Option<i32>) {
        self.state.process_observations.lock().remove(session_id);
        let now = now_millis();
        let exit = AgentProcessExit {
            code,
            reason: if code.unwrap_or_default() == 0 {
                AgentExitReason::Completed
            } else {
                AgentExitReason::Failed
            },
        };
        let exited_scopes = self
            .state
            .bindings
            .lock()
            .iter()
            .filter(|(_, bound_session_id)| *bound_session_id == session_id)
            .map(|(scope, _)| scope.clone())
            .collect::<Vec<_>>();
        let mut terminal_exits = self.state.terminal_exits.lock();
        terminal_exits.extend(exited_scopes.into_iter().map(|scope| (scope, exit)));
        let mut updates = Vec::new();
        let mut records = self.state.records.lock();
        for record in records
            .values_mut()
            .filter(|record| &record.terminal_session_id == session_id)
        {
            record.terminal_exited = true;
            if record
                .reducer
                .process_exited(record.scope.generation, exit, now)
            {
                updates.push(snapshot_update(self.state.host_epoch, record));
            }
        }
        drop(records);
        drop(terminal_exits);
        for update in updates {
            let _ = self.state.events.send(update);
        }
    }

    pub(crate) fn reconcile_process_scan(
        &self,
        roots: &[(TerminalSessionId, u32)],
        detected: &HashMap<TerminalSessionId, DetectedAgentProcess>,
    ) {
        let scopes_by_session = {
            let bindings = self.state.bindings.lock();
            bindings
                .iter()
                .filter(|(_, session_id)| {
                    roots
                        .iter()
                        .any(|(active_session_id, _)| active_session_id == *session_id)
                })
                .map(|(scope, session_id)| (session_id.clone(), scope.clone()))
                .collect::<HashMap<_, _>>()
        };
        let mut actions = Vec::new();
        {
            let mut observations = self.state.process_observations.lock();
            observations.retain(|session_id, observation| {
                let same_scope = scopes_by_session
                    .get(session_id)
                    .is_some_and(|scope| scope == &observation.scope);
                if !same_scope {
                    actions.push(AgentProcessAction::Finish {
                        scope: observation.scope.clone(),
                        provider_id: observation.process.provider_id.clone(),
                        reason: AgentExitReason::KilledByUser,
                    });
                }
                same_scope
            });
            for (session_id, scope) in &scopes_by_session {
                match detected.get(session_id) {
                    Some(process) => {
                        let changed = observations.get(session_id).is_none_or(|observation| {
                            observation.scope != *scope || observation.process != *process
                        });
                        if changed {
                            if let Some(previous) = observations.insert(
                                session_id.clone(),
                                AgentProcessObservation {
                                    scope: scope.clone(),
                                    process: process.clone(),
                                    missed_samples: 0,
                                },
                            ) {
                                actions.push(AgentProcessAction::Finish {
                                    scope: previous.scope,
                                    provider_id: previous.process.provider_id,
                                    reason: AgentExitReason::Completed,
                                });
                            }
                            actions.push(AgentProcessAction::Start {
                                terminal_session_id: session_id.clone(),
                                scope: scope.clone(),
                                provider_id: process.provider_id.clone(),
                            });
                        } else if let Some(observation) = observations.get_mut(session_id) {
                            observation.missed_samples = 0;
                        }
                    }
                    None => {
                        let finished = observations.get_mut(session_id).and_then(|observation| {
                            observation.missed_samples =
                                observation.missed_samples.saturating_add(1);
                            (observation.missed_samples >= AGENT_PROCESS_MISSED_SAMPLES_BEFORE_EXIT)
                                .then(|| observation.clone())
                        });
                        if let Some(finished) = finished {
                            observations.remove(session_id);
                            actions.push(AgentProcessAction::Finish {
                                scope: finished.scope,
                                provider_id: finished.process.provider_id,
                                reason: AgentExitReason::Completed,
                            });
                        }
                    }
                }
            }
        }
        self.apply_process_actions(actions);
    }

    fn apply_process_actions(&self, actions: Vec<AgentProcessAction>) {
        if actions.is_empty() {
            return;
        }
        let now = now_millis();
        let mut updates = Vec::new();
        let mut records = self.state.records.lock();
        for action in actions {
            match action {
                AgentProcessAction::Start {
                    terminal_session_id,
                    scope,
                    provider_id,
                } => {
                    let address = AgentAddress::from(&scope);
                    let should_replace = records.get(&address).is_none_or(|record| {
                        record.scope != scope
                            || record.reducer.snapshot().provider_id != provider_id
                            || record.reducer.snapshot().process_state == AgentProcessState::Exited
                    });
                    if !should_replace {
                        continue;
                    }
                    let prior_sequence = records
                        .get(&address)
                        .map(|record| record.sequence)
                        .unwrap_or_default();
                    if !records.contains_key(&address) && records.len() >= MAX_AGENT_RECORDS {
                        let oldest = records
                            .iter()
                            .min_by_key(|(_, record)| record.reducer.snapshot().updated_at)
                            .map(|(address, _)| address.clone());
                        if let Some(oldest) = oldest {
                            records.remove(&oldest);
                        }
                    }
                    let mut reducer =
                        AgentReducer::new(AgentInstanceId::random(), provider_id, now);
                    reducer.process_starting(scope.generation, now);
                    reducer.process_started(scope.generation, now);
                    records.insert(
                        address.clone(),
                        AgentRecord {
                            scope,
                            terminal_session_id,
                            sequence: prior_sequence,
                            reducer,
                            delivery: None,
                            terminal_exited: false,
                        },
                    );
                    let record = records
                        .get_mut(&address)
                        .expect("detected Agent record was inserted");
                    updates.push(snapshot_update(self.state.host_epoch, record));
                }
                AgentProcessAction::Finish {
                    scope,
                    provider_id,
                    reason,
                } => {
                    let address = AgentAddress::from(&scope);
                    let Some(record) = records.get_mut(&address) else {
                        continue;
                    };
                    if record.scope != scope
                        || record.reducer.snapshot().provider_id != provider_id
                        || record.reducer.snapshot().process_state == AgentProcessState::Exited
                    {
                        continue;
                    }
                    if record.reducer.process_exited(
                        scope.generation,
                        AgentProcessExit { code: None, reason },
                        now,
                    ) {
                        updates.push(snapshot_update(self.state.host_epoch, record));
                    }
                }
            }
        }
        drop(records);
        for update in updates {
            let _ = self.state.events.send(update);
        }
    }

    pub fn decay_stale_activity(&self) {
        let now = now_millis();
        let mut updates = Vec::new();
        {
            let mut records = self.state.records.lock();
            for record in records.values_mut() {
                if record
                    .reducer
                    .decay_stale_activity(now, AGENT_ACTIVITY_STALE_AFTER_MILLIS)
                {
                    updates.push(snapshot_update(self.state.host_epoch, record));
                }
            }
        }
        for update in updates {
            let _ = self.state.events.send(update);
        }
    }

    pub fn snapshots_after(
        &self,
        acknowledged: &[AgentSnapshotCursor],
    ) -> Vec<AgentSnapshotUpdate> {
        let acknowledged = acknowledged
            .iter()
            .map(|cursor| (AgentAddress::from(&cursor.scope), cursor))
            .collect::<HashMap<_, _>>();
        let records = self.state.records.lock();
        let mut snapshots = records
            .iter()
            .filter(|(address, record)| {
                acknowledged.get(*address).is_none_or(|cursor| {
                    cursor.host_epoch != self.state.host_epoch
                        || cursor.scope.generation != record.scope.generation
                        || cursor.sequence < record.sequence
                })
            })
            .map(|(_, record)| current_snapshot(self.state.host_epoch, record))
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| {
            (
                &left.scope.project_id,
                &left.scope.tab_id,
                &left.scope.pane_id,
            )
                .cmp(&(
                    &right.scope.project_id,
                    &right.scope.tab_id,
                    &right.scope.pane_id,
                ))
        });
        snapshots
    }

    pub fn active_agent_count(&self) -> usize {
        self.state
            .records
            .lock()
            .values()
            .filter(|record| record.reducer.snapshot().process_state != AgentProcessState::Exited)
            .count()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<AgentSnapshotUpdate> {
        self.state.events.subscribe()
    }
}

impl Drop for HostAgentHookRuntime {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
        for thread in self.worker_threads.drain(..) {
            let _ = thread.join();
        }
    }
}

fn snapshot_update(host_epoch: u64, record: &mut AgentRecord) -> AgentSnapshotUpdate {
    record.sequence = record.sequence.saturating_add(1);
    current_snapshot(host_epoch, record)
}

fn current_snapshot(host_epoch: u64, record: &AgentRecord) -> AgentSnapshotUpdate {
    AgentSnapshotUpdate {
        scope: record.scope.clone(),
        terminal_session_id: record.terminal_session_id.clone(),
        host_epoch,
        sequence: record.sequence,
        snapshot: record.reducer.snapshot().clone(),
    }
}

fn accept_loop(
    listener: TcpListener,
    connections: SyncSender<TcpStream>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => match connections.try_send(stream) {
                Ok(()) | Err(TrySendError::Full(_)) => {}
                Err(TrySendError::Disconnected(_)) => break,
            },
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_IDLE);
            }
            Err(_) => break,
        }
    }
}

fn connection_loop(
    connections: Arc<StdMutex<Receiver<TcpStream>>>,
    state: Arc<AgentState>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        let received = match connections.lock() {
            Ok(connections) => connections.recv_timeout(ACCEPT_IDLE),
            Err(_) => return,
        };
        match received {
            Ok(stream) => handle_connection(stream, &state),
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

enum HookIngestOutcome {
    Unsequenced,
    Sequenced {
        acknowledgement: AgentHookAcknowledgement,
        pending: bool,
    },
}

struct DeliveryApplication {
    events: Vec<AgentEventKind>,
    acknowledgement: AgentHookAcknowledgement,
    pending: bool,
}

fn handle_connection(mut stream: TcpStream, state: &AgentState) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));
    let request = read_http_request(&mut stream)
        .and_then(|request| decode_request(request, &state.secret))
        .and_then(|request| ingest_request(state, request));
    let (status, reason, body) = match request {
        Ok(HookIngestOutcome::Unsequenced) => (204, "No Content", Vec::new()),
        Ok(HookIngestOutcome::Sequenced {
            acknowledgement,
            pending,
        }) => (
            if pending { 202 } else { 200 },
            if pending { "Accepted" } else { "OK" },
            serde_json::to_vec(&acknowledgement).expect("hook acknowledgement must serialize"),
        ),
        Err(HttpRequestError::Conflict(acknowledgement)) => (
            409,
            "Conflict",
            serde_json::to_vec(&acknowledgement).expect("hook acknowledgement must serialize"),
        ),
        Err(HttpRequestError::Unauthorized) => (403, "Forbidden", Vec::new()),
        Err(HttpRequestError::NotFound) => (404, "Not Found", Vec::new()),
        Err(HttpRequestError::Invalid) => (400, "Bad Request", Vec::new()),
    };
    if body.is_empty() {
        let _ = write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
        );
    } else {
        let _ = write!(
            stream,
            "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        );
        let _ = stream.write_all(&body);
    }
    let _ = stream.flush();
}

struct DecodedHookRequest {
    scope: AgentHookScope,
    source: String,
    event: String,
    payload: Value,
    delivery: Option<HookDeliveryMetadata>,
}

fn incoming_provider_is_compatibility_duplicate(current: &str, incoming: &str) -> bool {
    current == GROK_PROVIDER_ID && incoming == CLAUDE_PROVIDER_ID
}

fn ingest_request(
    state: &AgentState,
    request: DecodedHookRequest,
) -> Result<HookIngestOutcome, HttpRequestError> {
    let terminal_session_id = state
        .bindings
        .lock()
        .get(&request.scope)
        .cloned()
        .ok_or(HttpRequestError::Unauthorized)?;
    let provider = state
        .providers
        .get(&request.source)
        .ok_or(HttpRequestError::NotFound)?;
    let provider_id = provider.descriptor().id;
    let normalized = provider
        .normalize_hook(yttt_agent_core::ProviderHookEvent {
            name: &request.event,
            payload: &request.payload,
        })
        .map_err(|_| HttpRequestError::Invalid)?;
    let starts_session = normalized
        .iter()
        .any(|event| matches!(event, AgentEventKind::SessionStarted { .. }));
    let address = AgentAddress::from(&request.scope);
    let terminal_exits = state.terminal_exits.lock();
    let terminal_exit = terminal_exits.get(&request.scope).copied();
    let now = now_millis();
    let (update, outcome) = {
        let mut records = state.records.lock();
        if records
            .get(&address)
            .is_some_and(|record| record.scope.generation > request.scope.generation)
        {
            return Err(HttpRequestError::Unauthorized);
        }
        let (same_scope, replace_record, compatibility_duplicate) = match records.get(&address) {
            Some(record) if record.scope == request.scope => {
                let snapshot = record.reducer.snapshot();
                let current_provider = snapshot.provider_id.as_str();
                let compatibility_duplicate = incoming_provider_is_compatibility_duplicate(
                    current_provider,
                    provider_id.as_str(),
                ) && !(starts_session
                    && snapshot.process_state == AgentProcessState::Exited);
                (
                    true,
                    starts_session
                        && !compatibility_duplicate
                        && (snapshot.process_state == AgentProcessState::Exited
                            || snapshot.provider_id != provider_id),
                    compatibility_duplicate,
                )
            }
            _ => (false, false, false),
        };
        if compatibility_duplicate {
            return Ok(HookIngestOutcome::Unsequenced);
        }
        if !same_scope || replace_record {
            let prior_sequence = records
                .get(&address)
                .map(|record| record.sequence)
                .unwrap_or_default();
            if records.len() >= MAX_AGENT_RECORDS {
                let oldest = records
                    .iter()
                    .min_by_key(|(_, record)| record.reducer.snapshot().updated_at)
                    .map(|(address, _)| address.clone());
                if let Some(oldest) = oldest {
                    records.remove(&oldest);
                }
            }
            let mut reducer =
                AgentReducer::new(AgentInstanceId::random(), provider_id.clone(), now);
            reducer.process_starting(request.scope.generation, now);
            reducer.process_started(request.scope.generation, now);
            if let Some(exit) = terminal_exit {
                reducer.process_exited(request.scope.generation, exit, now);
            }
            records.insert(
                address.clone(),
                AgentRecord {
                    scope: request.scope.clone(),
                    terminal_session_id,
                    sequence: prior_sequence,
                    reducer,
                    delivery: None,
                    terminal_exited: terminal_exit.is_some(),
                },
            );
        }
        let record = records
            .get_mut(&address)
            .expect("Agent record was inserted");
        let (events, outcome) = if let Some(delivery) = request.delivery {
            let application = queue_delivery(&mut record.delivery, delivery, normalized)?;
            (
                application.events,
                HookIngestOutcome::Sequenced {
                    acknowledgement: application.acknowledgement,
                    pending: application.pending,
                },
            )
        } else {
            (normalized, HookIngestOutcome::Unsequenced)
        };
        let mut changed = false;
        if !record.terminal_exited {
            for event in events {
                changed |= record.reducer.apply(request.scope.generation, event, now);
            }
        }
        (
            changed.then(|| snapshot_update(state.host_epoch, record)),
            outcome,
        )
    };
    if let Some(update) = update {
        let _ = state.events.send(update);
    }
    Ok(outcome)
}

fn queue_delivery(
    delivery_state: &mut Option<AgentDeliveryState>,
    delivery: HookDeliveryMetadata,
    events: Vec<AgentEventKind>,
) -> Result<DeliveryApplication, HttpRequestError> {
    let state = delivery_state.get_or_insert_with(|| AgentDeliveryState {
        stream_id: delivery.stream_id.clone(),
        accepted_sequence: 0,
        pending: BTreeMap::new(),
        retired: Vec::new(),
    });
    if state.stream_id != delivery.stream_id {
        if let Some((_, accepted_sequence)) = state
            .retired
            .iter()
            .find(|(stream_id, _)| stream_id == &delivery.stream_id)
        {
            let acknowledgement = AgentHookAcknowledgement {
                accepted_sequence: *accepted_sequence,
                next_sequence: accepted_sequence.saturating_add(1),
            };
            if delivery.sequence <= *accepted_sequence {
                return Ok(DeliveryApplication {
                    events: Vec::new(),
                    acknowledgement,
                    pending: false,
                });
            }
            return Err(HttpRequestError::Conflict(acknowledgement));
        }
        if delivery.sequence != 1 {
            return Err(HttpRequestError::Conflict(AgentHookAcknowledgement {
                accepted_sequence: 0,
                next_sequence: 1,
            }));
        }
        if state.retired.len() >= MAX_RETIRED_DELIVERY_STREAMS {
            state.retired.remove(0);
        }
        state.retired.push((
            std::mem::take(&mut state.stream_id),
            state.accepted_sequence,
        ));
        state.stream_id = delivery.stream_id.clone();
        state.accepted_sequence = 0;
        state.pending.clear();
    }

    if delivery.sequence <= state.accepted_sequence {
        return Ok(DeliveryApplication {
            events: Vec::new(),
            acknowledgement: AgentHookAcknowledgement {
                accepted_sequence: state.accepted_sequence,
                next_sequence: state.accepted_sequence.saturating_add(1),
            },
            pending: false,
        });
    }
    if !state.pending.contains_key(&delivery.sequence) {
        if state.pending.len() >= MAX_PENDING_DELIVERIES {
            return Err(HttpRequestError::Conflict(AgentHookAcknowledgement {
                accepted_sequence: state.accepted_sequence,
                next_sequence: state.accepted_sequence.saturating_add(1),
            }));
        }
        state.pending.insert(delivery.sequence, events);
    }

    let requested_sequence = delivery.sequence;
    let mut accepted_events = Vec::new();
    while let Some(events) = state
        .pending
        .remove(&state.accepted_sequence.saturating_add(1))
    {
        state.accepted_sequence = state.accepted_sequence.saturating_add(1);
        accepted_events.extend(events);
    }
    Ok(DeliveryApplication {
        events: accepted_events,
        acknowledgement: AgentHookAcknowledgement {
            accepted_sequence: state.accepted_sequence,
            next_sequence: state.accepted_sequence.saturating_add(1),
        },
        pending: state.accepted_sequence < requested_sequence,
    })
}

struct HttpRequest {
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum HttpRequestError {
    Invalid,
    Unauthorized,
    NotFound,
    Conflict(AgentHookAcknowledgement),
}

fn read_http_request(stream: &mut TcpStream) -> Result<HttpRequest, HttpRequestError> {
    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0_u8; 4096];
    let header_end = loop {
        let count = stream
            .read(&mut chunk)
            .map_err(|_| HttpRequestError::Invalid)?;
        if count == 0 {
            return Err(HttpRequestError::Invalid);
        }
        bytes.extend_from_slice(&chunk[..count]);
        if let Some(index) = find_bytes(&bytes, b"\r\n\r\n") {
            break index + 4;
        }
        if bytes.len() > MAX_HEADER_BYTES {
            return Err(HttpRequestError::Invalid);
        }
    };
    if header_end > MAX_HEADER_BYTES {
        return Err(HttpRequestError::Invalid);
    }

    let header_text =
        std::str::from_utf8(&bytes[..header_end - 4]).map_err(|_| HttpRequestError::Invalid)?;
    let mut lines = header_text.split("\r\n");
    let mut request_line = lines
        .next()
        .ok_or(HttpRequestError::Invalid)?
        .split_whitespace();
    if request_line.next() != Some("POST") {
        return Err(HttpRequestError::NotFound);
    }
    let path = request_line
        .next()
        .ok_or(HttpRequestError::Invalid)?
        .split('?')
        .next()
        .unwrap_or_default()
        .to_string();
    if request_line.next().is_none() {
        return Err(HttpRequestError::Invalid);
    }

    let mut headers = BTreeMap::new();
    for line in lines {
        let (name, value) = line.split_once(':').ok_or(HttpRequestError::Invalid)?;
        headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
    }
    let content_length = headers
        .get("content-length")
        .ok_or(HttpRequestError::Invalid)?
        .parse::<usize>()
        .map_err(|_| HttpRequestError::Invalid)?;
    if content_length > MAX_BODY_BYTES {
        return Err(HttpRequestError::Invalid);
    }
    while bytes.len().saturating_sub(header_end) < content_length {
        let count = stream
            .read(&mut chunk)
            .map_err(|_| HttpRequestError::Invalid)?;
        if count == 0 {
            return Err(HttpRequestError::Invalid);
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len().saturating_sub(header_end) > MAX_BODY_BYTES {
            return Err(HttpRequestError::Invalid);
        }
    }
    Ok(HttpRequest {
        path,
        headers,
        body: bytes[header_end..header_end + content_length].to_vec(),
    })
}

fn decode_request(
    request: HttpRequest,
    secret: &[u8; 32],
) -> Result<DecodedHookRequest, HttpRequestError> {
    let source = match request.path.as_str() {
        "/hook/codex" => "codex",
        "/hook/claude" => "claude",
        "/hook/grok" => "grok",
        "/hook/opencode" => "opencode",
        "/hook/pi" => "pi",
        "/hook/omp" => "omp",
        _ => return Err(HttpRequestError::NotFound),
    };
    let encoded_scope = request
        .headers
        .get(SCOPE_HEADER)
        .filter(|scope| scope.len() <= 4096)
        .ok_or(HttpRequestError::Invalid)?;
    let expected_token = scope_token(secret, encoded_scope);
    if !request
        .headers
        .get(TOKEN_HEADER)
        .is_some_and(|token| constant_time_equal(token.as_bytes(), expected_token.as_bytes()))
    {
        return Err(HttpRequestError::Unauthorized);
    }
    let scope_bytes = URL_SAFE_NO_PAD
        .decode(encoded_scope)
        .map_err(|_| HttpRequestError::Invalid)?;
    let scope: AgentHookScope =
        serde_json::from_slice(&scope_bytes).map_err(|_| HttpRequestError::Invalid)?;
    if scope.generation == 0
        || scope.project_id.trim().is_empty()
        || scope.tab_id.trim().is_empty()
        || scope.pane_id.trim().is_empty()
    {
        return Err(HttpRequestError::Invalid);
    }
    let raw: Value =
        serde_json::from_slice(&request.body).map_err(|_| HttpRequestError::Invalid)?;
    let (event, payload, delivery) = if raw.get("protocol").is_some() {
        let delivery: AgentHookDelivery<Value> =
            serde_json::from_value(raw).map_err(|_| HttpRequestError::Invalid)?;
        if delivery.protocol != AGENT_HOOK_DELIVERY_PROTOCOL
            || delivery.sequence == 0
            || delivery.stream_id.trim().is_empty()
            || delivery.stream_id.len() > 128
        {
            return Err(HttpRequestError::Invalid);
        }
        (
            non_empty_event(&delivery.event)?,
            delivery.payload,
            Some(HookDeliveryMetadata {
                stream_id: delivery.stream_id,
                sequence: delivery.sequence,
            }),
        )
    } else {
        let (event, payload) = provider_event(raw)?;
        (event, payload, None)
    };
    Ok(DecodedHookRequest {
        scope,
        source: source.to_string(),
        event,
        payload,
        delivery,
    })
}

fn provider_event(raw: Value) -> Result<(String, Value), HttpRequestError> {
    if let Some(event) = raw.get("event").and_then(Value::as_str) {
        let payload = raw.get("payload").cloned().unwrap_or(Value::Null);
        return non_empty_event(event).map(|event| (event, payload));
    }
    let event = raw
        .get("hook_event_name")
        .or_else(|| raw.get("hookEventName"))
        .and_then(Value::as_str)
        .ok_or(HttpRequestError::Invalid)?;
    non_empty_event(event).map(|event| (event, raw))
}

fn non_empty_event(event: &str) -> Result<String, HttpRequestError> {
    let event = event.trim();
    if event.is_empty() || event.len() > 128 {
        return Err(HttpRequestError::Invalid);
    }
    Ok(event.to_string())
}

fn scope_token(secret: &[u8; 32], scope: &str) -> String {
    let mut inner_pad = [0x36_u8; 64];
    let mut outer_pad = [0x5c_u8; 64];
    for (index, byte) in secret.iter().enumerate() {
        inner_pad[index] ^= byte;
        outer_pad[index] ^= byte;
    }
    let mut inner = Sha256::new();
    inner.update(inner_pad);
    inner.update(scope.as_bytes());
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    outer.update(outer_pad);
    outer.update(inner);
    URL_SAFE_NO_PAD.encode(outer.finalize())
}

fn constant_time_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter()
        .zip(right)
        .fold(0_u8, |difference, (left, right)| {
            difference | (left ^ right)
        })
        == 0
}

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_agent_core::AgentViewState;
    use yttt_core::model::ids::ProjectId;
    use yttt_protocol::terminal::{TerminalExecutionSpec, TerminalGeometry};

    fn spec(session: &str, _pane: &str) -> TerminalSpawnSpec {
        TerminalSpawnSpec {
            session_id: TerminalSessionId::new(session),
            project_id: ProjectId::new("project"),
            cwd: yttt_protocol::ProjectRelativePath::root(),
            execution: TerminalExecutionSpec::Command {
                shell: "/bin/sh".to_string(),
                program: "/bin/true".to_string(),
                args: Vec::new(),
                return_to_shell: false,
            },
            geometry: TerminalGeometry {
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
            },
            geometry_epoch: 1,
            query_palette: Vec::new(),
            palette_revision: 1,
            scrollback_limit: 100,
            environment: vec![("YTTT_AGENT_HOOK_TOKEN".to_string(), "forged".to_string())],
            removed_environment: ENVIRONMENT_VARIABLES
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        }
    }

    fn sequenced_request(
        scope: &AgentHookScope,
        sequence: u64,
        event: &str,
        payload: Value,
    ) -> DecodedHookRequest {
        DecodedHookRequest {
            scope: scope.clone(),
            source: "omp".to_string(),
            event: event.to_string(),
            payload,
            delivery: Some(HookDeliveryMetadata {
                stream_id: "test-stream".to_string(),
                sequence,
            }),
        }
    }

    fn unsequenced_request(
        scope: &AgentHookScope,
        source: &str,
        event: &str,
        payload: Value,
    ) -> DecodedHookRequest {
        DecodedHookRequest {
            scope: scope.clone(),
            source: source.to_string(),
            event: event.to_string(),
            payload,
            delivery: None,
        }
    }

    #[test]
    fn host_replaces_forged_hook_environment_and_scopes_each_terminal() {
        let runtime = HostAgentHookRuntime::start(7).unwrap();
        let mut first = spec("first", "first");
        let first_scope = runtime.secure_terminal_environment(&mut first);
        let mut second = spec("second", "second");
        let second_scope = runtime.secure_terminal_environment(&mut second);

        assert_ne!(first_scope, second_scope);
        assert!(first.removed_environment.is_empty());
        assert_eq!(
            first
                .environment
                .iter()
                .filter(|(name, _)| ENVIRONMENT_VARIABLES.contains(&name.as_str()))
                .count(),
            3
        );
        assert!(first.environment.iter().all(|(_, value)| value != "forged"));
    }

    #[test]
    fn token_for_one_scope_cannot_authenticate_another_scope() {
        let secret = [7_u8; 32];
        let first = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&AgentHookScope {
                project_id: "project".to_string(),
                tab_id: "tab".to_string(),
                pane_id: "first".to_string(),
                generation: 1,
            })
            .unwrap(),
        );
        let second = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&AgentHookScope {
                project_id: "project".to_string(),
                tab_id: "tab".to_string(),
                pane_id: "second".to_string(),
                generation: 2,
            })
            .unwrap(),
        );
        let request = HttpRequest {
            path: "/hook/codex".to_string(),
            headers: BTreeMap::from([
                (SCOPE_HEADER.to_string(), second),
                (TOKEN_HEADER.to_string(), scope_token(&secret, &first)),
            ]),
            body: br#"{"event":"turn-start","payload":null}"#.to_vec(),
        };

        assert!(matches!(
            decode_request(request, &secret),
            Err(HttpRequestError::Unauthorized)
        ));
    }

    #[test]
    fn decodes_native_grok_hook_requests() {
        let secret = [9_u8; 32];
        let scope = URL_SAFE_NO_PAD.encode(
            serde_json::to_vec(&AgentHookScope {
                project_id: "project".to_string(),
                tab_id: "tab".to_string(),
                pane_id: "grok".to_string(),
                generation: 1,
            })
            .unwrap(),
        );
        let request = HttpRequest {
            path: "/hook/grok".to_string(),
            headers: BTreeMap::from([
                (SCOPE_HEADER.to_string(), scope.clone()),
                (TOKEN_HEADER.to_string(), scope_token(&secret, &scope)),
            ]),
            body: br#"{"hookEventName":"SessionStart","sessionId":"grok-session"}"#.to_vec(),
        };

        let decoded = decode_request(request, &secret).unwrap();
        assert_eq!(decoded.source, "grok");
        assert_eq!(decoded.event, "SessionStart");
        assert_eq!(
            decoded.payload.get("sessionId").and_then(Value::as_str),
            Some("grok-session")
        );
    }

    #[test]
    fn native_grok_hooks_supersede_imported_claude_compatibility_hooks() {
        let runtime = HostAgentHookRuntime::start(7).unwrap();
        let mut terminal = spec("grok-provider", "grok-provider");
        let scope = runtime.secure_terminal_environment(&mut terminal);
        let mut updates = runtime.subscribe();
        let session = serde_json::json!({
            "sessionId": "grok-session",
            "model": "grok-code-fast"
        });

        ingest_request(
            &runtime.state,
            unsequenced_request(&scope, "claude", "SessionStart", session.clone()),
        )
        .unwrap();
        let compatibility = updates.try_recv().unwrap();
        assert_eq!(compatibility.snapshot.provider_id.as_str(), "claude");

        ingest_request(
            &runtime.state,
            unsequenced_request(&scope, "grok", "SessionStart", session.clone()),
        )
        .unwrap();
        let native = updates.try_recv().unwrap();
        assert_eq!(native.snapshot.provider_id.as_str(), "grok");
        assert!(native.sequence > compatibility.sequence);

        ingest_request(
            &runtime.state,
            unsequenced_request(&scope, "claude", "UserPromptSubmit", session.clone()),
        )
        .unwrap();
        assert!(updates.try_recv().is_err());
        assert_eq!(
            runtime.snapshots_after(&[])[0]
                .snapshot
                .provider_id
                .as_str(),
            "grok"
        );

        ingest_request(
            &runtime.state,
            unsequenced_request(&scope, "grok", "SessionEnd", session.clone()),
        )
        .unwrap();
        let exited = updates.try_recv().unwrap();
        assert_eq!(exited.snapshot.process_state, AgentProcessState::Exited);

        ingest_request(
            &runtime.state,
            unsequenced_request(&scope, "claude", "SessionStart", session),
        )
        .unwrap();
        let claude = updates.try_recv().unwrap();
        assert_eq!(claude.snapshot.provider_id.as_str(), "claude");
        assert!(claude.sequence > exited.sequence);
    }

    #[test]
    fn a_new_agent_session_replaces_an_exited_session_in_the_same_shell() {
        let runtime = HostAgentHookRuntime::start(7).unwrap();
        let mut terminal = spec("sequential-agent", "sequential-agent");
        let scope = runtime.secure_terminal_environment(&mut terminal);
        let mut updates = runtime.subscribe();

        ingest_request(
            &runtime.state,
            unsequenced_request(&scope, "omp", "session_start", Value::Null),
        )
        .unwrap();
        let first = updates.try_recv().unwrap();

        ingest_request(
            &runtime.state,
            unsequenced_request(&scope, "omp", "session_shutdown", Value::Null),
        )
        .unwrap();
        let exited = updates.try_recv().unwrap();
        assert_eq!(exited.snapshot.process_state, AgentProcessState::Exited);

        ingest_request(
            &runtime.state,
            unsequenced_request(&scope, "omp", "session_start", Value::Null),
        )
        .unwrap();
        let restarted = updates.try_recv().unwrap();
        assert_eq!(restarted.snapshot.process_state, AgentProcessState::Running);
        assert_ne!(restarted.snapshot.instance_id, first.snapshot.instance_id);
        assert!(restarted.sequence > exited.sequence);
    }

    #[test]
    fn process_scans_end_a_killed_agent_and_detect_its_replacement() {
        let runtime = HostAgentHookRuntime::start(7).unwrap();
        let mut terminal = spec("process-scan", "process-scan");
        let session_id = terminal.session_id.clone();
        runtime.secure_terminal_environment(&mut terminal);
        let roots = vec![(session_id.clone(), 10)];
        let mut updates = runtime.subscribe();

        runtime.reconcile_process_scan(
            &roots,
            &HashMap::from([(
                session_id.clone(),
                DetectedAgentProcess {
                    pid: 20,
                    provider_id: ProviderId::from_static("grok"),
                },
            )]),
        );
        let started = updates.try_recv().unwrap();
        assert_eq!(started.snapshot.provider_id.as_str(), "grok");
        assert_eq!(started.snapshot.process_state, AgentProcessState::Running);

        runtime.reconcile_process_scan(&roots, &HashMap::new());
        assert!(updates.try_recv().is_err());
        runtime.reconcile_process_scan(&roots, &HashMap::new());
        let exited = updates.try_recv().unwrap();
        assert_eq!(exited.snapshot.process_state, AgentProcessState::Exited);

        runtime.reconcile_process_scan(
            &roots,
            &HashMap::from([(
                session_id,
                DetectedAgentProcess {
                    pid: 21,
                    provider_id: ProviderId::from_static("grok"),
                },
            )]),
        );
        let restarted = updates.try_recv().unwrap();
        assert_eq!(restarted.snapshot.process_state, AgentProcessState::Running);
        assert_ne!(restarted.snapshot.instance_id, started.snapshot.instance_id);
        assert!(restarted.sequence > exited.sequence);
    }

    #[test]
    fn sequenced_hooks_buffer_gaps_and_ignore_retries() {
        let runtime = HostAgentHookRuntime::start(7).unwrap();
        let mut terminal = spec("ordered", "ordered");
        let scope = runtime.secure_terminal_environment(&mut terminal);
        let mut updates = runtime.subscribe();

        let pending = ingest_request(
            &runtime.state,
            sequenced_request(
                &scope,
                2,
                "agent_end",
                serde_json::json!({"willContinue": false}),
            ),
        )
        .unwrap();
        assert!(matches!(
            pending,
            HookIngestOutcome::Sequenced {
                acknowledgement: AgentHookAcknowledgement {
                    accepted_sequence: 0,
                    next_sequence: 1,
                },
                pending: true,
            }
        ));
        assert!(updates.try_recv().is_err());

        let accepted = ingest_request(
            &runtime.state,
            sequenced_request(&scope, 1, "agent_start", Value::Null),
        )
        .unwrap();
        assert!(matches!(
            accepted,
            HookIngestOutcome::Sequenced {
                acknowledgement: AgentHookAcknowledgement {
                    accepted_sequence: 2,
                    next_sequence: 3,
                },
                pending: false,
            }
        ));
        let completed = updates.try_recv().unwrap();
        assert_eq!(completed.snapshot.view_state(), AgentViewState::Completed);

        let retried = ingest_request(
            &runtime.state,
            sequenced_request(&scope, 1, "agent_start", Value::Null),
        )
        .unwrap();
        assert!(matches!(
            retried,
            HookIngestOutcome::Sequenced {
                acknowledgement: AgentHookAcknowledgement {
                    accepted_sequence: 2,
                    next_sequence: 3,
                },
                pending: false,
            }
        ));
        assert!(updates.try_recv().is_err());
        assert_eq!(
            runtime.snapshots_after(&[])[0].snapshot.view_state(),
            AgentViewState::Completed
        );
    }

    #[test]
    fn terminal_exit_is_final_even_when_a_hook_arrives_late() {
        let runtime = HostAgentHookRuntime::start(7).unwrap();
        let mut terminal = spec("final-exit", "final-exit");
        let session_id = terminal.session_id.clone();
        let scope = runtime.secure_terminal_environment(&mut terminal);
        let mut updates = runtime.subscribe();

        ingest_request(
            &runtime.state,
            sequenced_request(&scope, 1, "agent_start", Value::Null),
        )
        .unwrap();
        let _ = updates.try_recv().unwrap();
        runtime.terminal_exited(&session_id, Some(0));
        let completed = updates.try_recv().unwrap();
        assert_eq!(completed.snapshot.view_state(), AgentViewState::Completed);

        let late = ingest_request(
            &runtime.state,
            sequenced_request(&scope, 2, "agent_start", Value::Null),
        )
        .unwrap();
        assert!(matches!(
            late,
            HookIngestOutcome::Sequenced {
                acknowledgement: AgentHookAcknowledgement {
                    accepted_sequence: 2,
                    next_sequence: 3,
                },
                pending: false,
            }
        ));
        assert!(updates.try_recv().is_err());
        let current = &runtime.snapshots_after(&[])[0];
        assert_eq!(current.sequence, completed.sequence);
        assert_eq!(current.snapshot.view_state(), AgentViewState::Completed);
    }

    #[test]
    fn terminal_exit_before_first_hook_is_tombstoned_until_new_generation() {
        let runtime = HostAgentHookRuntime::start(7).unwrap();
        let mut terminal = spec("pre-hook-exit", "pre-hook-exit");
        let session_id = terminal.session_id.clone();
        let scope = runtime.secure_terminal_environment(&mut terminal);
        let mut updates = runtime.subscribe();

        runtime.terminal_exited(&session_id, Some(0));
        let late = ingest_request(
            &runtime.state,
            sequenced_request(&scope, 1, "agent_start", Value::Null),
        )
        .unwrap();
        assert!(matches!(
            late,
            HookIngestOutcome::Sequenced {
                acknowledgement: AgentHookAcknowledgement {
                    accepted_sequence: 1,
                    next_sequence: 2,
                },
                pending: false,
            }
        ));
        assert!(updates.try_recv().is_err());
        assert_eq!(
            runtime.snapshots_after(&[])[0].snapshot.view_state(),
            AgentViewState::Completed
        );

        let mut restarted = spec("pre-hook-exit", "pre-hook-exit");
        let restarted_scope = runtime.secure_terminal_environment(&mut restarted);
        assert!(restarted_scope.generation > scope.generation);
        assert!(
            runtime.snapshots_after(&[]).is_empty(),
            "a new terminal incarnation must not inherit the prior Agent record"
        );
        ingest_request(
            &runtime.state,
            sequenced_request(&restarted_scope, 1, "agent_start", Value::Null),
        )
        .unwrap();
        assert_eq!(
            runtime.snapshots_after(&[])[0].snapshot.view_state(),
            AgentViewState::Working
        );
    }

    #[test]
    fn stalled_hook_connection_does_not_block_another_delivery() {
        let runtime = HostAgentHookRuntime::start(7).unwrap();
        let mut terminal = spec("concurrent", "concurrent");
        runtime.secure_terminal_environment(&mut terminal);
        let endpoint = runtime.endpoint.strip_prefix("http://").unwrap();
        let stalled = TcpStream::connect(endpoint).unwrap();
        thread::sleep(Duration::from_millis(25));

        let scope = terminal
            .environment
            .iter()
            .find(|(name, _)| name == ENVIRONMENT_VARIABLES[2])
            .map(|(_, value)| value)
            .unwrap();
        let token = terminal
            .environment
            .iter()
            .find(|(name, _)| name == ENVIRONMENT_VARIABLES[1])
            .map(|(_, value)| value)
            .unwrap();
        let body = serde_json::to_vec(&AgentHookDelivery {
            protocol: AGENT_HOOK_DELIVERY_PROTOCOL,
            stream_id: "network-stream".to_string(),
            sequence: 1,
            event: "agent_start".to_string(),
            payload: Value::Null,
        })
        .unwrap();
        let mut delivered = TcpStream::connect(endpoint).unwrap();
        delivered
            .set_read_timeout(Some(Duration::from_millis(500)))
            .unwrap();
        write!(
            delivered,
            "POST /hook/omp HTTP/1.1\r\nHost: {endpoint}\r\nContent-Type: application/json\r\n{TOKEN_HEADER}: {token}\r\n{SCOPE_HEADER}: {scope}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            body.len()
        )
        .unwrap();
        delivered.write_all(&body).unwrap();
        delivered.flush().unwrap();
        let mut response = String::new();
        delivered.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK"), "{response}");
        assert!(response.contains("\"acceptedSequence\":1"), "{response}");
        drop(stalled);
    }
}
