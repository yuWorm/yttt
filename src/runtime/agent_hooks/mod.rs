use std::{
    collections::BTreeMap,
    fmt,
    io::{self, Read as _, Write as _},
    net::{TcpListener, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};
use uuid::Uuid;

use crate::{config::default_layout::BuiltinAgent, runtime::agent_manager::AgentPaneAddress};

pub mod installer;
pub mod providers;

const MAX_HEADER_BYTES: usize = 32 * 1024;
const MAX_BODY_BYTES: usize = 1024 * 1024;
const MAX_PENDING_EVENTS: usize = 64;
const CLIENT_TIMEOUT: Duration = Duration::from_secs(1);
const ACCEPT_IDLE: Duration = Duration::from_millis(10);
const TOKEN_HEADER: &str = "x-yttt-agent-hook-token";
const SCOPE_HEADER: &str = "x-yttt-agent-hook-scope";

#[derive(Clone)]
pub struct AgentHookClient {
    endpoint: Arc<str>,
    secret: Arc<[u8; 32]>,
}

impl fmt::Debug for AgentHookClient {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AgentHookClient")
            .field("endpoint", &self.endpoint)
            .field("secret", &"[redacted]")
            .finish()
    }
}

impl AgentHookClient {
    pub fn environment(
        &self,
        address: &AgentPaneAddress,
        generation: u64,
    ) -> BTreeMap<String, String> {
        let scope = WireScope {
            project_id: address.project_id.clone(),
            tab_id: address.tab_id.clone(),
            pane_id: address.pane_id.clone(),
            generation,
        };
        let scope = serde_json::to_vec(&scope)
            .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
            .unwrap_or_default();
        let token = scope_token(&self.secret, &scope);
        BTreeMap::from([
            (
                "YTTT_AGENT_HOOK_ENDPOINT".to_string(),
                self.endpoint.to_string(),
            ),
            ("YTTT_AGENT_HOOK_TOKEN".to_string(), token),
            ("YTTT_AGENT_HOOK_SCOPE".to_string(), scope),
        ])
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentHookRequest {
    pub address: AgentPaneAddress,
    pub generation: u64,
    pub source: BuiltinAgent,
    pub event: String,
    pub payload: Value,
}

pub struct AgentHookServer {
    client: AgentHookClient,
    receiver: Receiver<AgentHookRequest>,
    shutdown: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl AgentHookServer {
    pub fn start() -> io::Result<Self> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        listener.set_nonblocking(true)?;
        let port = listener.local_addr()?.port();
        let first = Uuid::new_v4();
        let second = Uuid::new_v4();
        let mut secret = [0_u8; 32];
        secret[..16].copy_from_slice(first.as_bytes());
        secret[16..].copy_from_slice(second.as_bytes());
        let secret = Arc::new(secret);
        let endpoint: Arc<str> = format!("http://127.0.0.1:{port}").into();
        let (sender, receiver) = mpsc::sync_channel(MAX_PENDING_EVENTS);
        let shutdown = Arc::new(AtomicBool::new(false));
        let thread_shutdown = shutdown.clone();
        let thread_secret = secret.clone();
        let accept_thread = thread::Builder::new()
            .name("yttt-agent-hook-listener".to_string())
            .spawn(move || accept_loop(listener, thread_secret, sender, thread_shutdown))?;

        Ok(Self {
            client: AgentHookClient { endpoint, secret },
            receiver,
            shutdown,
            accept_thread: Some(accept_thread),
        })
    }

    pub fn client(&self) -> AgentHookClient {
        self.client.clone()
    }

    pub fn drain(&self) -> Vec<AgentHookRequest> {
        self.receiver.try_iter().take(MAX_PENDING_EVENTS).collect()
    }
}

impl Drop for AgentHookServer {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Release);
        if let Some(thread) = self.accept_thread.take() {
            let _ = thread.join();
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WireScope {
    project_id: String,
    tab_id: String,
    pane_id: String,
    generation: u64,
}

fn accept_loop(
    listener: TcpListener,
    secret: Arc<[u8; 32]>,
    sender: SyncSender<AgentHookRequest>,
    shutdown: Arc<AtomicBool>,
) {
    while !shutdown.load(Ordering::Acquire) {
        match listener.accept() {
            Ok((stream, _)) => handle_connection(stream, &secret, &sender),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(ACCEPT_IDLE);
            }
            Err(_) => break,
        }
    }
}

fn handle_connection(
    mut stream: TcpStream,
    secret: &[u8; 32],
    sender: &SyncSender<AgentHookRequest>,
) {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(CLIENT_TIMEOUT));
    let _ = stream.set_write_timeout(Some(CLIENT_TIMEOUT));
    let request =
        read_http_request(&mut stream).and_then(|request| decode_hook_request(request, secret));
    let status = match request {
        Ok(request) => {
            let _ = sender.try_send(request);
            204
        }
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

struct HttpRequest {
    method: String,
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
    let method = request_line
        .next()
        .filter(|method| *method == "POST")
        .ok_or(HttpRequestError::NotFound)?
        .to_string();
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

    while bytes.len() - header_end < content_length {
        let count = stream
            .read(&mut chunk)
            .map_err(|_| HttpRequestError::Invalid)?;
        if count == 0 {
            return Err(HttpRequestError::Invalid);
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() - header_end > MAX_BODY_BYTES {
            return Err(HttpRequestError::Invalid);
        }
    }
    let body = bytes[header_end..header_end + content_length].to_vec();
    Ok(HttpRequest {
        method,
        path,
        headers,
        body,
    })
}

fn decode_hook_request(
    request: HttpRequest,
    secret: &[u8; 32],
) -> Result<AgentHookRequest, HttpRequestError> {
    if request.method != "POST" {
        return Err(HttpRequestError::NotFound);
    }
    let source = match request.path.as_str() {
        "/hook/codex" => BuiltinAgent::Codex,
        "/hook/claude" => BuiltinAgent::Claude,
        "/hook/opencode" => BuiltinAgent::OpenCode,
        "/hook/pi" => BuiltinAgent::Pi,
        "/hook/omp" => BuiltinAgent::OhMyPi,
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
    let scope: WireScope =
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
    Ok(AgentHookRequest {
        address: AgentPaneAddress::new(&scope.project_id, &scope.tab_id, &scope.pane_id),
        generation: scope.generation,
        source,
        event,
        payload,
    })
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

fn find_bytes(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|candidate| candidate == needle)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpStream;
    #[test]
    fn scope_token_cannot_be_reused_for_another_pane() {
        let server = AgentHookServer::start().unwrap();
        let first = server
            .client()
            .environment(&AgentPaneAddress::new("project", "tab", "first"), 1);
        let second = server
            .client()
            .environment(&AgentPaneAddress::new("project", "tab", "second"), 1);
        assert_ne!(
            first["YTTT_AGENT_HOOK_TOKEN"],
            second["YTTT_AGENT_HOOK_TOKEN"]
        );
        let request = HttpRequest {
            method: "POST".to_string(),
            path: "/hook/codex".to_string(),
            headers: BTreeMap::from([
                (
                    TOKEN_HEADER.to_string(),
                    first["YTTT_AGENT_HOOK_TOKEN"].clone(),
                ),
                (
                    SCOPE_HEADER.to_string(),
                    second["YTTT_AGENT_HOOK_SCOPE"].clone(),
                ),
            ]),
            body: br#"{"hook_event_name":"SessionStart"}"#.to_vec(),
        };
        assert_eq!(
            decode_hook_request(request, &server.client.secret),
            Err(HttpRequestError::Unauthorized)
        );
    }

    #[test]
    fn authenticated_request_reaches_the_bounded_queue() {
        let server = AgentHookServer::start().unwrap();
        let address = AgentPaneAddress::new("project", "tab", "pane");
        let env = server.client().environment(&address, 7);
        let endpoint = env["YTTT_AGENT_HOOK_ENDPOINT"]
            .strip_prefix("http://")
            .unwrap();
        let body = br#"{"hook_event_name":"UserPromptSubmit","prompt":"ship it"}"#;
        let mut stream = TcpStream::connect(endpoint).unwrap();
        write!(
            stream,
            "POST /hook/claude HTTP/1.1\r\nHost: {endpoint}\r\nContent-Length: {}\r\n{TOKEN_HEADER}: {}\r\n{SCOPE_HEADER}: {}\r\n\r\n",
            body.len(),
            env["YTTT_AGENT_HOOK_TOKEN"],
            env["YTTT_AGENT_HOOK_SCOPE"]
        )
        .unwrap();
        stream.write_all(body).unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 204"));

        let mut events = Vec::new();
        for _ in 0..50 {
            events = server.drain();
            if !events.is_empty() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].address, address);
        assert_eq!(events[0].generation, 7);
        assert_eq!(events[0].source, BuiltinAgent::Claude);
        assert_eq!(events[0].event, "UserPromptSubmit");
        assert_eq!(events[0].payload["prompt"], "ship it");
    }

    #[test]
    fn invalid_token_is_rejected_without_queueing() {
        let server = AgentHookServer::start().unwrap();
        let endpoint = server.client.endpoint.strip_prefix("http://").unwrap();
        let mut stream = TcpStream::connect(endpoint).unwrap();
        write!(
            stream,
            "POST /hook/codex HTTP/1.1\r\nHost: {endpoint}\r\nContent-Length: 2\r\n{TOKEN_HEADER}: bad\r\n{SCOPE_HEADER}: bad\r\n\r\n{{}}"
        )
        .unwrap();
        stream.flush().unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 403"), "{response:?}");
        assert!(server.drain().is_empty());
    }
}
