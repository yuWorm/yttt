use std::{
    io::{BufRead, BufReader, Read, Write as _},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::Path,
    process::{Child, ChildStdin, ChildStdout, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

use super::SessionOperation;
use crate::sessions::{AgentSessionRoots, SessionProvider};

const CODEX_TIMEOUT: Duration = Duration::from_secs(15);
const OPENCODE_SERVER_TIMEOUT: Duration = Duration::from_secs(20);
const OPENCODE_READY_TIMEOUT: Duration = Duration::from_secs(8);
const HTTP_TIMEOUT: Duration = Duration::from_secs(3);
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);
const MAX_PROTOCOL_LINE_BYTES: usize = 1024 * 1024;
const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;
const MAX_CAPTURED_STDERR_BYTES: usize = 64 * 1024;

/// Performs mutations that only the provider's own native interface can safely
/// carry out. The parent module has already validated the user input and the
/// discovered-session identity before dispatching here.
pub(super) fn mutate(
    provider: SessionProvider,
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
    operation: &SessionOperation,
) -> Result<(), String> {
    match provider {
        SessionProvider::Codex => mutate_codex(session_id, project_path, roots, operation),
        SessionProvider::OpenCode => mutate_opencode(session_id, project_path, roots, operation),
        _ => Err(format!(
            "{} has no native session-management adapter",
            provider.id()
        )),
    }
}

fn mutate_codex(
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
    operation: &SessionOperation,
) -> Result<(), String> {
    with_codex_server(project_path, roots, |server| {
        server.initialize()?;
        let thread = server.read_thread(session_id)?;
        validate_codex_thread(&thread, session_id, project_path, roots)?;

        match operation {
            SessionOperation::Rename { title } => {
                server.request(
                    3,
                    "thread/name/set",
                    json!({ "threadId": session_id, "name": title }),
                )?;
                let updated = server.read_thread_with_request_id(session_id, 4)?;
                validate_codex_thread(&updated, session_id, project_path, roots)?;
                if updated
                    .get("thread")
                    .and_then(|thread| thread.get("name"))
                    .and_then(Value::as_str)
                    != Some(title.as_str())
                {
                    return Err("Codex did not persist the requested session title".to_string());
                }
            }
            SessionOperation::Delete => {
                // `thread/archive` only moves history aside. `thread/delete` is the
                // hard-delete API and its acknowledgement is verified below.
                server.request(3, "thread/delete", json!({ "threadId": session_id }))?;
                match server.request(4, "thread/read", json!({ "threadId": session_id })) {
                    Ok(_) => {
                        return Err(
                            "Codex acknowledged deletion but the session is still readable"
                                .to_string(),
                        );
                    }
                    Err(error) if is_not_found_error(&error) => {}
                    // Some Codex versions report a deleted thread as unloaded.
                    // Accept that only when its previously validated rollout is gone.
                    Err(error)
                        if error.ends_with(&format!("thread not loaded: {session_id}"))
                            && thread
                                .get("thread")
                                .and_then(|thread| thread.get("path"))
                                .and_then(Value::as_str)
                                .is_some_and(|path| {
                                    matches!(Path::new(path).try_exists(), Ok(false))
                                }) => {}
                    Err(error) => {
                        return Err(format!(
                            "could not verify Codex session deletion after acknowledgement: {error}"
                        ));
                    }
                }
            }
        }
        Ok(())
    })
}

fn validate_codex_thread(
    thread: &Value,
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
) -> Result<(), String> {
    let thread = thread
        .get("thread")
        .ok_or_else(|| "Codex returned a malformed thread/read response".to_string())?;
    if thread.get("id").and_then(Value::as_str) != Some(session_id) {
        return Err("Codex returned a different session than the requested ID".to_string());
    }

    let cwd = thread
        .get("cwd")
        .and_then(Value::as_str)
        .ok_or_else(|| "Codex did not report the session working directory".to_string())?;
    ensure_same_project(Path::new(cwd), project_path, "Codex")?;

    // `path` is intentionally optional in Codex's app-server schema. When a
    // version exposes it, it provides a second store-containment check; the
    // configured CODEX_HOME remains authoritative for versions that do not.
    if let Some(session_path) = thread.get("path").and_then(Value::as_str) {
        ensure_path_within(Path::new(session_path), &roots.codex, "Codex session path")?;
    }

    let status = thread
        .get("status")
        .ok_or_else(|| "Codex did not report the session status".to_string())?;
    let kind = status
        .as_str()
        .or_else(|| status.get("type").and_then(Value::as_str));
    if matches!(kind, Some("idle" | "notLoaded")) {
        Ok(())
    } else {
        Err(format!(
            "Codex session is live or unavailable for safe mutation ({})",
            status_description(status)
        ))
    }
}

fn with_codex_server<T>(
    project_path: &Path,
    roots: &AgentSessionRoots,
    operation: impl FnOnce(&mut CodexServer) -> Result<T, String>,
) -> Result<T, String> {
    let mut server = CodexServer::start(project_path, roots)?;
    let result = operation(&mut server);
    let shutdown = server.shutdown();
    match (result, shutdown) {
        (Ok(value), Ok(_)) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Ok(diagnostics)) => Err(with_provider_diagnostics(error, diagnostics)),
        (Err(error), Err(_)) => Err(error),
    }
}

struct CodexServer {
    child: Arc<Mutex<Child>>,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    stderr: Option<thread::JoinHandle<Vec<u8>>>,
    deadline_done: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
    watchdog: Option<thread::JoinHandle<()>>,
}

impl CodexServer {
    fn start(project_path: &Path, roots: &AgentSessionRoots) -> Result<Self, String> {
        let codex_home = roots
            .codex
            .canonicalize()
            .map_err(|error| format!("could not resolve configured Codex session root: {error}"))?;
        if !codex_home.is_dir() {
            return Err("configured Codex session root is not a directory".to_string());
        }
        let mut command = Command::new("codex");
        command
            .args(["app-server", "--stdio"])
            // Do not let an isolated profile resolve the caller's regular Codex
            // home. App-server reads and writes exactly the store that discovery
            // selected.
            .env("CODEX_HOME", &codex_home)
            .current_dir(project_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_provider_process(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("could not start Codex app-server: {error}"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| "could not open Codex app-server standard input".to_string())?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "could not open Codex app-server standard output".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "could not open Codex app-server standard error".to_string())?;
        let child = Arc::new(Mutex::new(child));
        let deadline_done = Arc::new(AtomicBool::new(false));
        let timed_out = Arc::new(AtomicBool::new(false));
        let watchdog = spawn_timeout_watchdog(
            Arc::clone(&child),
            Arc::clone(&deadline_done),
            Arc::clone(&timed_out),
            CODEX_TIMEOUT,
        );
        Ok(Self {
            child,
            stdin: Some(stdin),
            stdout: BufReader::new(stdout),
            stderr: Some(spawn_output_drain(stderr)),
            deadline_done,
            timed_out,
            watchdog: Some(watchdog),
        })
    }

    fn initialize(&mut self) -> Result<(), String> {
        self.request(
            1,
            "initialize",
            json!({
                "clientInfo": {
                    "name": "yttt",
                    "title": "yttt session management",
                    "version": env!("CARGO_PKG_VERSION"),
                },
            }),
        )?;
        self.notification("initialized")
    }

    fn read_thread(&mut self, session_id: &str) -> Result<Value, String> {
        self.read_thread_with_request_id(session_id, 2)
    }

    fn read_thread_with_request_id(
        &mut self,
        session_id: &str,
        request_id: u64,
    ) -> Result<Value, String> {
        self.request(request_id, "thread/read", json!({ "threadId": session_id }))
    }

    fn notification(&mut self, method: &str) -> Result<(), String> {
        self.write_message(&json!({ "method": method }))
    }

    fn request(&mut self, request_id: u64, method: &str, params: Value) -> Result<Value, String> {
        self.write_message(&json!({ "id": request_id, "method": method, "params": params }))?;
        loop {
            let Some(line) = read_protocol_line(&mut self.stdout)? else {
                return Err(
                    self.protocol_stream_error("Codex app-server closed its protocol stream")
                );
            };
            let mut message: Value = serde_json::from_slice(&line).map_err(|error| {
                format!("Codex app-server emitted invalid protocol JSON: {error}")
            })?;
            if message.get("id").and_then(Value::as_u64) != Some(request_id) {
                continue;
            }
            if let Some(error) = message.get("error") {
                return Err(format!(
                    "Codex app-server {method} failed: {}",
                    rpc_error_description(error)
                ));
            }
            return message
                .as_object_mut()
                .and_then(|message| message.remove("result"))
                .ok_or_else(|| format!("Codex app-server {method} returned no result"));
        }
    }

    fn write_message(&mut self, message: &Value) -> Result<(), String> {
        let stdin = self
            .stdin
            .as_mut()
            .ok_or_else(|| "Codex app-server input is already closed".to_string())?;
        serde_json::to_writer(&mut *stdin, message)
            .map_err(|error| format!("could not encode Codex app-server request: {error}"))?;
        stdin
            .write_all(b"\n")
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("could not send Codex app-server request: {error}"))
    }

    fn protocol_stream_error(&self, prefix: &str) -> String {
        if self.timed_out.load(Ordering::SeqCst) {
            format!(
                "{prefix}: timed out after {} seconds",
                CODEX_TIMEOUT.as_secs()
            )
        } else {
            prefix.to_string()
        }
    }

    fn shutdown(&mut self) -> Result<String, String> {
        if self.watchdog.is_none() {
            return Ok(String::new());
        }
        self.deadline_done.store(true, Ordering::SeqCst);
        drop(self.stdin.take());
        let mut child = self
            .child
            .lock()
            .map_err(|_| "Codex app-server process lock was poisoned".to_string())?;
        let exited = wait_for_child(&mut child, SHUTDOWN_GRACE)?;
        // Descendants can retain the output pipes after the server exits.
        force_stop_provider(&mut child, "Codex app-server")?;
        if !exited {
            child
                .wait()
                .map_err(|error| format!("could not reap temporary Codex app-server: {error}"))?;
        }
        drop(child);
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        let diagnostics = self
            .stderr
            .take()
            .and_then(|stderr| stderr.join().ok())
            .map(|output| String::from_utf8_lossy(&output).trim().to_string())
            .unwrap_or_default();
        Ok(diagnostics)
    }
}

impl Drop for CodexServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

fn mutate_opencode(
    session_id: &str,
    project_path: &Path,
    roots: &AgentSessionRoots,
    operation: &SessionOperation,
) -> Result<(), String> {
    // OpenCode's 1.18 native store is selected by its XDG environment rather
    // than AgentSessionRoots. Refuse profiles with explicit agent roots so an
    // isolated profile cannot accidentally mutate the user's ambient store.
    ensure_opencode_uses_native_profile(roots)?;

    with_opencode_server(project_path, |server| {
        server.wait_until_ready()?;
        let session = server.get_session(session_id)?;
        validate_opencode_session(&session, session_id, project_path)?;
        server.ensure_idle(session_id)?;

        match operation {
            SessionOperation::Rename { title } => {
                let updated = server.json_request(
                    "PATCH",
                    &session_path(session_id),
                    Some(json!({ "title": title })),
                )?;
                validate_opencode_session(&updated, session_id, project_path)?;
                if updated.get("title").and_then(Value::as_str) != Some(title.as_str()) {
                    return Err("OpenCode did not persist the requested session title".to_string());
                }
            }
            SessionOperation::Delete => {
                let response = server.raw_request("DELETE", &session_path(session_id), None)?;
                if !(200..300).contains(&response.status) {
                    return Err(format!(
                        "OpenCode DELETE {} failed with HTTP {}{}",
                        session_path(session_id),
                        response.status,
                        response_error_suffix(&response.body)
                    ));
                }
                if !response.body.is_empty()
                    && serde_json::from_slice::<Value>(&response.body)
                        .ok()
                        .and_then(|value| value.as_bool())
                        != Some(true)
                {
                    return Err("OpenCode did not confirm deletion of the session".to_string());
                }
                let verification = server.raw_request("GET", &session_path(session_id), None)?;
                if verification.status != 404 {
                    return Err(format!(
                        "OpenCode acknowledged deletion but the session is still accessible (HTTP {})",
                        verification.status
                    ));
                }
            }
        }
        Ok(())
    })
}

fn ensure_opencode_uses_native_profile(roots: &AgentSessionRoots) -> Result<(), String> {
    let native = AgentSessionRoots::native().ok_or_else(|| {
        "OpenCode native mutation needs a host profile with a configured home directory".to_string()
    })?;
    if roots.codex != native.codex
        || roots.claude != native.claude
        || roots.grok != native.grok
        || roots.pi != native.pi
        || roots.omp != native.omp
    {
        return Err(
            "OpenCode native mutation is unavailable for an isolated agent-session profile"
                .to_string(),
        );
    }
    Ok(())
}

fn validate_opencode_session(
    session: &Value,
    session_id: &str,
    project_path: &Path,
) -> Result<(), String> {
    if session.get("id").and_then(Value::as_str) != Some(session_id) {
        return Err("OpenCode returned a different session than the requested ID".to_string());
    }
    let directory = session
        .get("directory")
        .and_then(Value::as_str)
        .ok_or_else(|| "OpenCode did not report the session project directory".to_string())?;
    ensure_same_project(Path::new(directory), project_path, "OpenCode")
}

fn with_opencode_server<T>(
    project_path: &Path,
    operation: impl FnOnce(&mut OpenCodeServer) -> Result<T, String>,
) -> Result<T, String> {
    let mut server = OpenCodeServer::start(project_path)?;
    let result = operation(&mut server);
    let shutdown = server.shutdown();
    match (result, shutdown) {
        (Ok(value), Ok(_)) => Ok(value),
        (Ok(_), Err(error)) => Err(error),
        (Err(error), Ok(diagnostics)) => Err(with_provider_diagnostics(error, diagnostics)),
        (Err(error), Err(_)) => Err(error),
    }
}

struct OpenCodeServer {
    address: SocketAddr,
    basic_authorization: String,
    child: Arc<Mutex<Child>>,
    stdout: Option<thread::JoinHandle<Vec<u8>>>,
    stderr: Option<thread::JoinHandle<Vec<u8>>>,
    deadline_done: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
    watchdog: Option<thread::JoinHandle<()>>,
}

impl OpenCodeServer {
    fn start(project_path: &Path) -> Result<Self, String> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .map_err(|error| format!("could not reserve a loopback port for OpenCode: {error}"))?;
        let address = listener
            .local_addr()
            .map_err(|error| format!("could not inspect OpenCode loopback port: {error}"))?;
        drop(listener);

        let password = local_server_password();
        let port = address.port().to_string();
        let mut command = Command::new("opencode");
        command
            .args([
                "serve",
                "--pure",
                "--hostname",
                "127.0.0.1",
                "--port",
                &port,
            ])
            .env("OPENCODE_SERVER_USERNAME", "opencode")
            .env("OPENCODE_SERVER_PASSWORD", &password)
            .current_dir(project_path)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        configure_provider_process(&mut command);
        let mut child = command
            .spawn()
            .map_err(|error| format!("could not start OpenCode server: {error}"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| "could not capture OpenCode server output".to_string())?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| "could not capture OpenCode server diagnostics".to_string())?;
        let child = Arc::new(Mutex::new(child));
        let deadline_done = Arc::new(AtomicBool::new(false));
        let timed_out = Arc::new(AtomicBool::new(false));
        let watchdog = spawn_timeout_watchdog(
            Arc::clone(&child),
            Arc::clone(&deadline_done),
            Arc::clone(&timed_out),
            OPENCODE_SERVER_TIMEOUT,
        );
        Ok(Self {
            address,
            basic_authorization: basic_authorization(&password),
            child,
            stdout: Some(spawn_output_drain(stdout)),
            stderr: Some(spawn_output_drain(stderr)),
            deadline_done,
            timed_out,
            watchdog: Some(watchdog),
        })
    }

    fn wait_until_ready(&mut self) -> Result<(), String> {
        let deadline = Instant::now() + OPENCODE_READY_TIMEOUT;
        let mut last_error = None;
        while Instant::now() < deadline {
            if self.timed_out.load(Ordering::SeqCst) {
                return Err(format!(
                    "OpenCode server timed out after {} seconds",
                    OPENCODE_SERVER_TIMEOUT.as_secs()
                ));
            }
            if let Some(status) = self
                .child
                .lock()
                .map_err(|_| "OpenCode server process lock was poisoned".to_string())?
                .try_wait()
                .map_err(|error| format!("could not inspect OpenCode server: {error}"))?
            {
                return Err(format!(
                    "OpenCode server exited before becoming ready ({status})"
                ));
            }
            match self.raw_request("GET", "/global/health", None) {
                Ok(response) if response.status == 200 => return Ok(()),
                Ok(response) => {
                    last_error = Some(format!(
                        "OpenCode health endpoint returned HTTP {}",
                        response.status
                    ));
                }
                Err(error) => last_error = Some(error),
            }
            thread::sleep(Duration::from_millis(100));
        }
        Err(format!(
            "OpenCode server did not become ready within {} seconds{}",
            OPENCODE_READY_TIMEOUT.as_secs(),
            last_error
                .map(|error| format!(": {error}"))
                .unwrap_or_default()
        ))
    }

    fn get_session(&self, session_id: &str) -> Result<Value, String> {
        self.json_request("GET", &session_path(session_id), None)
    }

    fn ensure_idle(&self, session_id: &str) -> Result<(), String> {
        let statuses = self.json_request("GET", "/session/status", None)?;
        let Some(status) = statuses.get(session_id) else {
            return Ok(());
        };
        if status.get("type").and_then(Value::as_str) == Some("idle") {
            return Ok(());
        }
        Err(format!(
            "OpenCode session is live or unavailable for safe mutation ({})",
            status_description(status)
        ))
    }

    fn json_request(&self, method: &str, path: &str, body: Option<Value>) -> Result<Value, String> {
        let response = self.raw_request(method, path, body)?;
        if !(200..300).contains(&response.status) {
            return Err(format!(
                "OpenCode {method} {path} failed with HTTP {}{}",
                response.status,
                response_error_suffix(&response.body)
            ));
        }
        serde_json::from_slice(&response.body)
            .map_err(|error| format!("OpenCode {method} {path} returned invalid JSON: {error}"))
    }

    fn raw_request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> Result<HttpResponse, String> {
        let mut stream = TcpStream::connect_timeout(&self.address, Duration::from_millis(250))
            .map_err(|error| format!("could not connect to local OpenCode server: {error}"))?;
        stream
            .set_read_timeout(Some(HTTP_TIMEOUT))
            .and_then(|()| stream.set_write_timeout(Some(HTTP_TIMEOUT)))
            .map_err(|error| format!("could not configure OpenCode HTTP timeout: {error}"))?;
        let body = match body {
            Some(body) => serde_json::to_vec(&body)
                .map_err(|error| format!("could not encode OpenCode request: {error}"))?,
            None => Vec::new(),
        };
        let mut request = format!(
            "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: {}\r\nAccept: application/json\r\nConnection: close\r\n",
            self.address.port(),
            self.basic_authorization,
        )
        .into_bytes();
        if !body.is_empty() {
            request.extend_from_slice(
                format!(
                    "Content-Type: application/json\r\nContent-Length: {}\r\n",
                    body.len()
                )
                .as_bytes(),
            );
        }
        request.extend_from_slice(b"\r\n");
        request.extend_from_slice(&body);
        stream
            .write_all(&request)
            .and_then(|()| stream.flush())
            .map_err(|error| format!("could not send OpenCode HTTP request: {error}"))?;
        read_http_response(&mut stream)
    }

    fn shutdown(&mut self) -> Result<String, String> {
        if self.watchdog.is_none() {
            return Ok(String::new());
        }
        self.deadline_done.store(true, Ordering::SeqCst);
        let mut child = self
            .child
            .lock()
            .map_err(|_| "OpenCode server process lock was poisoned".to_string())?;
        let exited = wait_for_child(&mut child, SHUTDOWN_GRACE)?;
        force_stop_provider(&mut child, "OpenCode server")?;
        if !exited {
            child
                .wait()
                .map_err(|error| format!("could not reap temporary OpenCode server: {error}"))?;
        }
        drop(child);
        if let Some(watchdog) = self.watchdog.take() {
            let _ = watchdog.join();
        }
        if let Some(stdout) = self.stdout.take() {
            let _ = stdout.join();
        }
        let diagnostics = self
            .stderr
            .take()
            .and_then(|stderr| stderr.join().ok())
            .map(|output| String::from_utf8_lossy(&output).trim().to_string())
            .unwrap_or_default();
        Ok(diagnostics)
    }
}

impl Drop for OpenCodeServer {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

struct HttpResponse {
    status: u16,
    body: Vec<u8>,
}

fn read_http_response(stream: &mut impl Read) -> Result<HttpResponse, String> {
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    read_http_line(&mut reader, &mut line)?;
    let status = std::str::from_utf8(&line)
        .ok()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or_else(|| "OpenCode HTTP response had an invalid status line".to_string())?;
    let mut content_length = None;
    let mut chunked = false;
    let mut header_bytes = line.len();
    loop {
        read_http_line(&mut reader, &mut line)?;
        header_bytes += line.len() + 2;
        if header_bytes > 64 * 1024 {
            return Err("OpenCode HTTP headers exceeded the safety limit".into());
        }
        if line.is_empty() {
            break;
        }
        let header = std::str::from_utf8(&line)
            .map_err(|_| "OpenCode HTTP header was not UTF-8".to_string())?;
        let (name, value) = header
            .split_once(':')
            .ok_or_else(|| "OpenCode returned an invalid HTTP header".to_string())?;
        if name.eq_ignore_ascii_case("content-length") {
            let size = value
                .trim()
                .parse::<usize>()
                .map_err(|_| "OpenCode returned an invalid content length".to_string())?;
            if size > MAX_HTTP_RESPONSE_BYTES || content_length.is_some_and(|old| old != size) {
                return Err("OpenCode returned an unsafe content length".into());
            }
            content_length = Some(size);
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            if !value.trim().eq_ignore_ascii_case("chunked") {
                return Err("OpenCode returned an unsupported transfer encoding".into());
            }
            chunked = true;
        }
    }
    let body = if status == 204 || status == 304 {
        Vec::new()
    } else if chunked {
        decode_chunked_body(&mut reader)?
    } else if let Some(size) = content_length {
        let mut body = vec![0; size];
        reader
            .read_exact(&mut body)
            .map_err(|error| format!("could not read OpenCode response body: {error}"))?;
        body
    } else {
        let mut body = Vec::new();
        reader
            .take(MAX_HTTP_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut body)
            .map_err(|error| format!("could not read OpenCode response body: {error}"))?;
        if body.len() > MAX_HTTP_RESPONSE_BYTES {
            return Err("OpenCode HTTP response exceeded the safety limit".into());
        }
        body
    };
    Ok(HttpResponse { status, body })
}

fn decode_chunked_body(mut input: impl BufRead) -> Result<Vec<u8>, String> {
    let mut decoded = Vec::new();
    let mut line = Vec::new();
    loop {
        read_http_line(&mut input, &mut line)?;
        let size = std::str::from_utf8(&line)
            .ok()
            .and_then(|line| line.split(';').next())
            .and_then(|value| usize::from_str_radix(value.trim(), 16).ok())
            .ok_or_else(|| "OpenCode HTTP response had an invalid chunk size".to_string())?;
        if size == 0 {
            let mut trailer_bytes = 0;
            loop {
                read_http_line(&mut input, &mut line)?;
                trailer_bytes += line.len() + 2;
                if trailer_bytes > 64 * 1024 {
                    return Err("OpenCode HTTP trailers exceeded the safety limit".into());
                }
                if line.is_empty() {
                    return Ok(decoded);
                }
            }
        }
        if size > MAX_HTTP_RESPONSE_BYTES - decoded.len() {
            return Err("OpenCode HTTP response exceeded the safety limit".into());
        }
        let offset = decoded.len();
        decoded.resize(offset + size, 0);
        input
            .read_exact(&mut decoded[offset..])
            .map_err(|error| format!("could not read OpenCode response chunk: {error}"))?;
        let mut delimiter = [0; 2];
        input
            .read_exact(&mut delimiter)
            .map_err(|error| format!("could not read OpenCode chunk delimiter: {error}"))?;
        if delimiter != *b"\r\n" {
            return Err("OpenCode HTTP response ended inside a chunk".into());
        }
    }
}

fn read_http_line(reader: &mut impl BufRead, line: &mut Vec<u8>) -> Result<(), String> {
    line.clear();
    reader
        .take(64 * 1024)
        .read_until(b'\n', line)
        .map_err(|error| format!("could not read OpenCode HTTP headers: {error}"))?;
    if !line.ends_with(b"\r\n") {
        return Err("OpenCode returned an incomplete or oversized HTTP line".into());
    }
    line.truncate(line.len() - 2);
    Ok(())
}

fn session_path(session_id: &str) -> String {
    format!("/session/{}", percent_encode_path_segment(session_id))
}

fn percent_encode_path_segment(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~') {
            encoded.push(char::from(byte));
        } else {
            const HEX: &[u8; 16] = b"0123456789ABCDEF";
            encoded.push('%');
            encoded.push(char::from(HEX[(byte >> 4) as usize]));
            encoded.push(char::from(HEX[(byte & 0x0f) as usize]));
        }
    }
    encoded
}

fn local_server_password() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}

fn basic_authorization(password: &str) -> String {
    format!(
        "Basic {}",
        base64_encode(format!("opencode:{password}").as_bytes())
    )
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut encoded = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or_default();
        let third = chunk.get(2).copied().unwrap_or_default();
        encoded.push(char::from(TABLE[(first >> 2) as usize]));
        encoded.push(char::from(
            TABLE[((first & 0b11) << 4 | second >> 4) as usize],
        ));
        encoded.push(if chunk.len() > 1 {
            char::from(TABLE[((second & 0b1111) << 2 | third >> 6) as usize])
        } else {
            '='
        });
        encoded.push(if chunk.len() > 2 {
            char::from(TABLE[(third & 0b11_1111) as usize])
        } else {
            '='
        });
    }
    encoded
}

fn ensure_same_project(actual: &Path, expected: &Path, provider: &str) -> Result<(), String> {
    if !super::belongs_to_project(actual, expected) {
        return Err(format!("{provider} session belongs to a different project"));
    }
    Ok(())
}

fn ensure_path_within(path: &Path, root: &Path, label: &str) -> Result<(), String> {
    let path = path
        .canonicalize()
        .map_err(|error| format!("could not resolve {label}: {error}"))?;
    let root = root
        .canonicalize()
        .map_err(|error| format!("could not resolve configured session root: {error}"))?;
    if !path.starts_with(&root) {
        return Err(format!("{label} is outside the configured session root"));
    }
    Ok(())
}

fn read_protocol_line(reader: &mut impl BufRead) -> Result<Option<Vec<u8>>, String> {
    let mut line = Vec::with_capacity(512);
    loop {
        let available = reader
            .fill_buf()
            .map_err(|error| format!("could not read Codex app-server protocol: {error}"))?;
        if available.is_empty() {
            return if line.is_empty() {
                Ok(None)
            } else {
                Err("Codex app-server ended a protocol message without a newline".to_string())
            };
        }
        let newline = available.iter().position(|byte| *byte == b'\n');
        let count = newline.map_or(available.len(), |index| index + 1);
        if line.len() + count > MAX_PROTOCOL_LINE_BYTES {
            return Err("Codex app-server protocol message exceeded the safety limit".to_string());
        }
        line.extend_from_slice(&available[..count]);
        reader.consume(count);
        if newline.is_some() {
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            return Ok(Some(line));
        }
    }
}

fn spawn_output_drain(mut output: impl Read + Send + 'static) -> thread::JoinHandle<Vec<u8>> {
    thread::spawn(move || {
        let mut captured = Vec::new();
        let mut buffer = [0_u8; 8192];
        loop {
            match output.read(&mut buffer) {
                Ok(0) | Err(_) => return captured,
                Ok(read) => {
                    let remaining = MAX_CAPTURED_STDERR_BYTES.saturating_sub(captured.len());
                    captured.extend_from_slice(&buffer[..read.min(remaining)]);
                }
            }
        }
    })
}

fn spawn_timeout_watchdog(
    child: Arc<Mutex<Child>>,
    done: Arc<AtomicBool>,
    timed_out: Arc<AtomicBool>,
    timeout: Duration,
) -> thread::JoinHandle<()> {
    thread::spawn(move || {
        let deadline = Instant::now() + timeout;
        while !done.load(Ordering::SeqCst) {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                timed_out.store(true, Ordering::SeqCst);
                if let Ok(mut child) = child.lock() {
                    let _ = force_stop_provider(&mut child, "provider process");
                }
                return;
            }
            thread::sleep(remaining.min(Duration::from_millis(50)));
        }
    })
}
fn configure_provider_process(command: &mut Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt as _;

        command.process_group(0);
    }
    #[cfg(not(unix))]
    {
        let _ = command;
    }
}

fn force_stop_provider(child: &mut Child, label: &str) -> Result<(), String> {
    #[cfg(unix)]
    {
        let process_group = format!("-{}", child.id());
        let status = Command::new("/bin/kill")
            .args(["-KILL", process_group.as_str()])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|error| format!("could not stop temporary {label}: {error}"))?;
        if status.success()
            || child
                .try_wait()
                .map_err(|error| format!("could not inspect temporary {label}: {error}"))?
                .is_some()
        {
            return Ok(());
        }
        return Err(format!("could not stop temporary {label} process group"));
    }
    #[cfg(not(unix))]
    {
        child
            .kill()
            .map_err(|error| format!("could not stop temporary {label}: {error}"))
    }
}

fn wait_for_child(child: &mut Child, grace: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + grace;
    loop {
        if child
            .try_wait()
            .map_err(|error| format!("could not inspect temporary provider process: {error}"))?
            .is_some()
        {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(25));
    }
}

fn rpc_error_description(error: &Value) -> String {
    let code = error
        .get("code")
        .and_then(Value::as_i64)
        .map(|code| format!("code {code}: "))
        .unwrap_or_default();
    let message = error
        .get("message")
        .and_then(Value::as_str)
        .unwrap_or("unknown error");
    format!("{code}{message}")
}

fn with_provider_diagnostics(error: String, diagnostics: String) -> String {
    if diagnostics.is_empty() {
        error
    } else {
        format!("{error}: {diagnostics}")
    }
}

fn is_not_found_error(error: &str) -> bool {
    let error = error.to_ascii_lowercase();
    error.contains("not found") || error.contains("not_found") || error.contains("does not exist")
}

fn status_description(status: &Value) -> String {
    serde_json::to_string(status).unwrap_or_else(|_| "unknown status".to_string())
}

fn response_error_suffix(body: &[u8]) -> String {
    if body.is_empty() {
        return String::new();
    }
    let text = String::from_utf8_lossy(body);
    let text = text.trim();
    if text.is_empty() {
        String::new()
    } else {
        format!(": {text}")
    }
}

#[cfg(test)]
mod tests {
    use super::{base64_encode, percent_encode_path_segment};

    #[test]
    fn encodes_open_code_session_path_segments() {
        assert_eq!(percent_encode_path_segment("ses/a b?"), "ses%2Fa%20b%3F");
    }

    #[test]
    fn encodes_basic_auth_without_a_dependency() {
        assert_eq!(
            base64_encode(b"opencode:password"),
            "b3BlbmNvZGU6cGFzc3dvcmQ="
        );
    }

    #[test]
    fn rejects_overflowing_or_truncated_http_chunks() {
        for response in [b"ffffffffffffffff\r\n".as_slice(), b"4\r\nab\r\n"] {
            assert!(super::decode_chunked_body(response).is_err());
        }
        assert_eq!(
            super::decode_chunked_body(b"2\r\n{}\r\n0\r\n\r\n".as_slice()).unwrap(),
            b"{}"
        );
    }

    #[test]
    fn reads_framed_http_response_without_waiting_for_connection_close() {
        struct KeepAlive<'a>(&'a [u8]);
        impl std::io::Read for KeepAlive<'_> {
            fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
                if self.0.is_empty() {
                    return Err(std::io::ErrorKind::TimedOut.into());
                }
                std::io::Read::read(&mut self.0, bytes)
            }
        }
        for wire in [
            b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}".as_slice(),
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n",
        ] {
            let response = super::read_http_response(&mut KeepAlive(wire)).unwrap();
            assert_eq!(response.status, 200);
            assert_eq!(response.body, b"{}");
        }
    }
}
