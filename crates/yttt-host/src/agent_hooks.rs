use std::{
    collections::{BTreeMap, HashMap},
    io::{self, Read as _, Write as _},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
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
    AGENT_ACTIVITY_STALE_AFTER_MILLIS, AgentExitReason, AgentInstanceId, AgentProcessExit,
    AgentProvider, AgentReducer,
};
use yttt_agent_providers::builtin_providers;
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::{
    agent::{AgentHookScope, AgentSnapshotCursor, AgentSnapshotUpdate},
    terminal::TerminalSpawnSpec,
};

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(1);
const ACCEPT_IDLE: Duration = Duration::from_millis(10);
const EVENT_CAPACITY: usize = 64;
const MAX_AGENT_RECORDS: usize = 256;
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
}

struct AgentState {
    host_epoch: u64,
    secret: Arc<[u8; 32]>,
    providers: HashMap<String, Arc<dyn AgentProvider>>,
    bindings: Mutex<HashMap<AgentHookScope, TerminalSessionId>>,
    records: Mutex<HashMap<AgentAddress, AgentRecord>>,
    events: broadcast::Sender<AgentSnapshotUpdate>,
}

pub struct HostAgentHookRuntime {
    endpoint: Arc<str>,
    state: Arc<AgentState>,
    next_resource_epoch: AtomicU64,
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
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
            events,
        });
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_state = state.clone();
        let thread_shutdown = shutdown.clone();
        let accept_thread = thread::Builder::new()
            .name("yttt-host-agent-hooks".to_string())
            .spawn(move || accept_loop(listener, thread_state, thread_shutdown))?;
        Ok(Arc::new(Self {
            endpoint,
            state,
            next_resource_epoch: AtomicU64::new(1),
            shutdown,
            accept_thread: Some(accept_thread),
        }))
    }

    pub fn secure_terminal_environment(&self, spec: &mut TerminalSpawnSpec) -> AgentHookScope {
        spec.environment
            .retain(|(name, _)| !ENVIRONMENT_VARIABLES.contains(&name.as_str()));
        spec.removed_environment
            .retain(|name| !ENVIRONMENT_VARIABLES.contains(&name.as_str()));

        let scope = AgentHookScope {
            project_id: spec.project_id.to_string(),
            tab_id: spec.tab_id.to_string(),
            pane_id: spec.pane_id.to_string(),
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
            .bindings
            .lock()
            .retain(|candidate, _| AgentAddress::from(candidate) != address);
        self.state
            .bindings
            .lock()
            .insert(scope.clone(), spec.session_id.clone());
        scope
    }

    pub fn cancel_terminal(&self, scope: &AgentHookScope) {
        self.state.bindings.lock().remove(scope);
    }

    pub fn terminal_exited(&self, session_id: &TerminalSessionId, code: Option<i32>) {
        let now = now_millis();
        let mut updates = Vec::new();
        {
            let mut records = self.state.records.lock();
            for record in records
                .values_mut()
                .filter(|record| &record.terminal_session_id == session_id)
            {
                let exit = AgentProcessExit {
                    code,
                    reason: if code.unwrap_or_default() == 0 {
                        AgentExitReason::Completed
                    } else {
                        AgentExitReason::Failed
                    },
                };
                if record
                    .reducer
                    .process_exited(record.scope.generation, exit, now)
                {
                    updates.push(snapshot_update(self.state.host_epoch, record));
                }
            }
        }
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

fn accept_loop(listener: TcpListener, state: Arc<AgentState>, shutdown: Arc<AtomicBool>) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => handle_connection(stream, &state),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_IDLE);
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(mut stream: TcpStream, state: &AgentState) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));
    let request = read_http_request(&mut stream)
        .and_then(|request| decode_request(request, &state.secret))
        .and_then(|request| ingest_request(state, request));
    let status = match request {
        Ok(()) => 204,
        Err(HttpRequestError::Unauthorized) => 403,
        Err(HttpRequestError::NotFound) => 404,
        Err(HttpRequestError::Invalid) => 400,
    };
    let reason = match status {
        204 => "No Content",
        400 => "Bad Request",
        403 => "Forbidden",
        _ => "Not Found",
    };
    let _ = write!(
        stream,
        "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    );
    let _ = stream.flush();
}

struct DecodedHookRequest {
    scope: AgentHookScope,
    source: String,
    event: String,
    payload: Value,
}

fn ingest_request(state: &AgentState, request: DecodedHookRequest) -> Result<(), HttpRequestError> {
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
    let normalized = provider
        .normalize_hook(yttt_agent_core::ProviderHookEvent {
            name: &request.event,
            payload: &request.payload,
        })
        .map_err(|_| HttpRequestError::Invalid)?;
    let address = AgentAddress::from(&request.scope);
    let now = now_millis();
    let update = {
        let mut records = state.records.lock();
        if records
            .get(&address)
            .is_some_and(|record| record.scope.generation > request.scope.generation)
        {
            return Err(HttpRequestError::Unauthorized);
        }
        if !records
            .get(&address)
            .is_some_and(|record| record.scope == request.scope)
        {
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
                AgentReducer::new(AgentInstanceId::random(), provider.descriptor().id, now);
            reducer.process_starting(request.scope.generation, now);
            reducer.process_started(request.scope.generation, now);
            records.insert(
                address.clone(),
                AgentRecord {
                    scope: request.scope.clone(),
                    terminal_session_id,
                    sequence: 0,
                    reducer,
                },
            );
        }
        let record = records
            .get_mut(&address)
            .expect("Agent record was inserted");
        let mut changed = false;
        for event in normalized {
            changed |= record.reducer.apply(request.scope.generation, event, now);
        }
        changed.then(|| snapshot_update(state.host_epoch, record))
    };
    if let Some(update) = update {
        let _ = state.events.send(update);
    }
    Ok(())
}

struct HttpRequest {
    path: String,
    headers: BTreeMap<String, String>,
    body: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum HttpRequestError {
    Invalid,
    Unauthorized,
    NotFound,
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
    let (event, payload) = provider_event(raw)?;
    Ok(DecodedHookRequest {
        scope,
        source: source.to_string(),
        event,
        payload,
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
    use yttt_core::model::ids::{PaneId, ProjectId, TabId};
    use yttt_protocol::terminal::{TerminalExecutionSpec, TerminalGeometry};

    fn spec(session: &str, pane: &str) -> TerminalSpawnSpec {
        TerminalSpawnSpec {
            session_id: TerminalSessionId::new(session),
            project_id: ProjectId::new("project"),
            tab_id: TabId::new("tab"),
            pane_id: PaneId::new(pane),
            cwd: "/tmp".to_string(),
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
}
