#![forbid(unsafe_code)]
mod agent_hooks;
mod audit;
pub mod diagnostics;
mod lifecycle;
mod project;
pub mod runtime;
mod ssh_runtime;
pub mod terminal;
mod terminal_data;
pub use terminal_data::SharedTerminalUpdate;

use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    future::Future,
    io::{self, Read as _, Seek as _, SeekFrom, Write as _},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use crate::{
    agent_hooks::HostAgentHookRuntime,
    audit::{AuditEntry, AuditResult, HostAuditLog},
    diagnostics::{
        DEFAULT_DIAGNOSTICS_LOG_BYTES, DIAGNOSTICS_SCHEMA_VERSION, DiagnosticsClock,
        DiagnosticsSink, HostDiagnosticsSnapshot, ProcessDiagnosticsSampler,
        RotatingJsonlDiagnosticsSink, SystemDiagnosticsClock, diagnostics_log_path,
    },
    lifecycle::HostLifecycle,
    project::{HostProjectError, HostProjectRuntime},
    runtime::{AttachmentResyncReason, HostRuntime, HostRuntimeError, TerminalControlOutcome},
    ssh_runtime::HostSshRuntime,
    terminal::{HostTerminalEvent, HostedTerminalError},
    terminal_data::TerminalDataWriter,
};
use fs2::FileExt as _;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::{
    io::AsyncReadExt as _,
    sync::{broadcast, watch},
};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId, TerminalSessionId};
use yttt_protocol::{
    BuildIdentity, ClientRequest, ControlMessage, FailureCode, HostEvent, HostLifecycleStatus,
    HostResponse, LIFECYCLE_PROTOCOL_VERSION, LifecycleMessage, LifecycleRequest,
    LifecycleResponse, LifecycleResponseEnvelope, ProtocolFailure, ProtocolRange,
    RESOURCE_PROTOCOL_VERSION, Request, ResourceCatalog, Response, ServerEvent,
    TerminalTerminationResult,
    terminal::{
        AttachTerminal, TerminalLeaseMode, TerminalStreamUpdate, TerminalViewportAnchor,
        TerminalViewportRead, TerminationMode,
    },
};
use yttt_transport::{
    AuthToken, HostIdentity, TransportListener, TransportStream, receive_control,
    receive_lifecycle, send_control, send_lifecycle, server_handshake,
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostLifetime {
    DesktopOwned,
    #[default]
    Independent,
}

impl HostLifetime {
    pub const fn as_arg(self) -> &'static str {
        match self {
            Self::DesktopOwned => "desktop_owned",
            Self::Independent => "independent",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostBootstrap {
    pub profile_id: ProfileId,
    pub runtime_root: PathBuf,
    pub auth_token_file: PathBuf,
    pub ssh_host_keys_file: PathBuf,
    pub credential_namespace: String,
    pub build: BuildIdentity,
    pub lifetime: HostLifetime,
}

impl HostBootstrap {
    pub fn lock_file(&self) -> PathBuf {
        self.runtime_root.join("host.lock")
    }

    pub fn pid_file(&self) -> PathBuf {
        self.runtime_root.join("host.pid")
    }

    pub fn ready_file(&self) -> PathBuf {
        self.runtime_root.join("host-ready.json")
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReadyMetadata {
    pub profile_id: ProfileId,
    pub host_id: HostId,
    pub host_epoch: u64,
    pub pid: u32,
    #[serde(default)]
    pub lifetime: HostLifetime,
    pub build: BuildIdentity,
    pub resource_protocol: u16,
    pub lifecycle_protocol: u16,
    pub executable: PathBuf,
    pub started_millis: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum HostError {
    #[error("another Host owns the profile lock")]
    AlreadyRunning,
    #[error("invalid 32-byte Host authentication token")]
    InvalidAuthToken,
    #[error("Host bootstrap file permissions are not user-only")]
    InsecureBootstrapFile,
    #[error("Host I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("Host transport failed: {0}")]
    Transport(#[from] yttt_transport::TransportError),
    #[error("Host metadata failed: {0}")]
    Metadata(#[from] serde_json::Error),
    #[error("Host SSH runtime failed: {0}")]
    Ssh(String),
}

/// `bind` runs only after the single-instance guard is held. Binding first would let a
/// second Host claim the endpoint before losing the lock, unlinking the live socket on
/// its way out.
pub async fn run<L, B, Fut, E>(bootstrap: HostBootstrap, bind: B) -> Result<(), HostError>
where
    L: TransportListener,
    B: FnOnce() -> Fut,
    Fut: Future<Output = Result<L, E>>,
    E: Into<yttt_transport::TransportError>,
{
    fs::create_dir_all(&bootstrap.runtime_root)?;
    secure_runtime_root(&bootstrap.runtime_root)?;
    let guard = HostInstanceGuard::acquire(&bootstrap)?;
    let listener = bind()
        .await
        .map_err(|error| HostError::Transport(error.into()))?;
    let token = read_auth_token(&bootstrap.auth_token_file)?;
    let host_epoch = next_host_epoch(&bootstrap.runtime_root)?;
    let host_id = HostId::new(format!("host-{}-{host_epoch}", std::process::id()));
    let ready = ReadyMetadata {
        profile_id: bootstrap.profile_id.clone(),
        host_id: host_id.clone(),
        host_epoch,
        pid: std::process::id(),
        lifetime: bootstrap.lifetime,
        build: bootstrap.build.clone(),
        resource_protocol: RESOURCE_PROTOCOL_VERSION,
        lifecycle_protocol: LIFECYCLE_PROTOCOL_VERSION,
        executable: std::env::current_exe()?,
        started_millis: now_millis(),
    };

    let identity = Arc::new(HostIdentity {
        resource_supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        lifecycle_supported: ProtocolRange::exact(LIFECYCLE_PROTOCOL_VERSION),
        build: bootstrap.build,
        profile_id: bootstrap.profile_id.clone(),
        host_id,
        host_epoch,
        connection_sequence: 1,
    });
    let token = Arc::new(token);
    let next_connection = Arc::new(AtomicU64::new(1));
    let next_host_sequence = Arc::new(AtomicU64::new(1));
    let request_journals = Arc::new(Mutex::new(HashMap::new()));
    let audit = Arc::new(HostAuditLog::new());
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let runtime = HostRuntime::new();
    let lifecycle = Arc::new(HostLifecycle::new(stop_tx.clone(), bootstrap.lifetime));
    let projects = Arc::new(HostProjectRuntime::new_with_epoch(host_epoch));
    let ssh = HostSshRuntime::start(
        bootstrap.ssh_host_keys_file.clone(),
        bootstrap.credential_namespace.clone(),
    )
    .map_err(HostError::Ssh)?;
    tokio::spawn(
        lifecycle
            .clone()
            .run(runtime.clone(), ssh.clone(), projects.clone()),
    );
    let diagnostics_sink: Arc<dyn DiagnosticsSink> = Arc::new(RotatingJsonlDiagnosticsSink::new(
        diagnostics_log_path(&bootstrap.runtime_root),
        DEFAULT_DIAGNOSTICS_LOG_BYTES,
    ));
    let diagnostics_clock: Arc<dyn DiagnosticsClock> = Arc::new(SystemDiagnosticsClock);
    let diagnostics_runtime = runtime.clone();
    let diagnostics_lifecycle = lifecycle.clone();
    let diagnostics_sink_for_task = diagnostics_sink.clone();
    let diagnostics_clock_for_task = diagnostics_clock.clone();
    let mut diagnostics_stop = stop_rx.clone();
    let diagnostics_task = tokio::spawn(async move {
        let mut sampler = ProcessDiagnosticsSampler::default();
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {}
                changed = diagnostics_stop.changed() => {
                    if changed.is_err() || *diagnostics_stop.borrow() {
                        let snapshot = collect_host_diagnostics(
                            diagnostics_clock_for_task.as_ref(),
                            host_epoch,
                            &diagnostics_runtime,
                            &diagnostics_lifecycle,
                            &mut sampler,
                        );
                        let _ = diagnostics_sink_for_task.record(&snapshot);
                        break;
                    }
                    continue;
                }
            }
            let snapshot = collect_host_diagnostics(
                diagnostics_clock_for_task.as_ref(),
                host_epoch,
                &diagnostics_runtime,
                &diagnostics_lifecycle,
                &mut sampler,
            );
            let _ = diagnostics_sink_for_task.record(&snapshot);
        }
    });
    let agent_hooks = HostAgentHookRuntime::start(host_epoch)?;
    let mut agent_terminal_events = runtime.subscribe();
    let terminal_agent_hooks = agent_hooks.clone();
    let terminal_runtime = runtime.clone();
    tokio::spawn(async move {
        loop {
            match agent_terminal_events.recv().await {
                Ok(HostTerminalEvent::Exited {
                    session_id, code, ..
                }) => terminal_agent_hooks.terminal_exited(&session_id, code),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    for placement in terminal_runtime.placements() {
                        if let Some(viewport) = placement.viewport
                            && let yttt_protocol::terminal::TerminalProcessState::Exited { code } =
                                viewport.process_state
                        {
                            terminal_agent_hooks.terminal_exited(&placement.session_id, code);
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Ok(_) => {}
            }
        }
    });
    guard.publish_ready(&ready)?;

    let mut exit_reaper = tokio::time::interval(Duration::from_secs(60));
    exit_reaper.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
            }
            _ = exit_reaper.tick() => {
                let now = now_millis();
                let reaped_exits = runtime.reap_expired_exits(now);
                let reaped_control = runtime.reap_pending_control(now);
                if reaped_exits != 0 || reaped_control != 0 {
                    lifecycle.resource_changed();
                }
                agent_hooks.decay_stale_activity();
            }
            accepted = listener.accept() => {
                let stream = accepted?;
                let mut connection_identity = (*identity).clone();
                connection_identity.connection_sequence =
                    next_connection.fetch_add(1, Ordering::Relaxed);
                let token = token.clone();
                let context = ConnectionContext {
                    identity: connection_identity,
                    profile_id: bootstrap.profile_id.clone(),
                    host_sequence: next_host_sequence.clone(),
                    ssh: ssh.clone(),
                    agent_hooks: agent_hooks.clone(),
                    projects: projects.clone(),
                    request_journals: request_journals.clone(),
                    runtime: runtime.clone(),
                    lifecycle: lifecycle.clone(),
                    audit: audit.clone(),
                    stop: stop_rx.clone(),
                };
                tokio::spawn(async move {
                    let _ = serve_connection(stream, token, context).await;
                });
            }
        }
    }

    drop(listener);
    let _ = diagnostics_task.await;
    drop(guard);
    Ok(())
}

fn collect_host_diagnostics(
    clock: &dyn DiagnosticsClock,
    host_epoch: u64,
    runtime: &HostRuntime,
    lifecycle: &HostLifecycle,
    sampler: &mut ProcessDiagnosticsSampler,
) -> HostDiagnosticsSnapshot {
    let sessions = runtime.session_count();
    let attachments = runtime.attachment_count();
    let process = sampler.sample();
    HostDiagnosticsSnapshot {
        schema_version: DIAGNOSTICS_SCHEMA_VERSION,
        captured_at_millis: clock.now_millis(),
        host_epoch,
        sessions,
        clients: lifecycle.client_count(),
        attachments,
        rss_bytes: process.rss_bytes,
        thread_count: process.thread_count,
        idle_cpu_percent: (sessions == 0 && attachments == 0)
            .then_some(process.cpu_percent)
            .flatten(),
        terminals: runtime.terminal_diagnostics(),
        queues: runtime.attachment_queue_diagnostics(),
        attachment_resyncs: runtime.attachment_resync_diagnostics(),
    }
}

struct ConnectionContext {
    identity: HostIdentity,
    profile_id: ProfileId,
    host_sequence: Arc<AtomicU64>,
    ssh: Arc<HostSshRuntime>,
    agent_hooks: Arc<HostAgentHookRuntime>,
    projects: Arc<HostProjectRuntime>,
    request_journals:
        Arc<Mutex<HashMap<ClientInstanceId, Arc<tokio::sync::Mutex<RequestJournal>>>>>,
    runtime: Arc<HostRuntime>,
    lifecycle: Arc<HostLifecycle>,
    audit: Arc<HostAuditLog>,
    stop: watch::Receiver<bool>,
}

#[derive(Clone, Copy, Debug)]
struct TerminalAttachment {
    mode: TerminalLeaseMode,
    lease_epoch: u64,
    display_offset: u64,
    unseen_output: u64,
    search_generation: u64,
    last_client_sequence: u64,
    pending_scroll_resync: bool,
    lagged_events: u64,
}

impl TerminalAttachment {
    fn new(mode: TerminalLeaseMode, lease_epoch: u64) -> Self {
        Self {
            mode,
            lease_epoch,
            display_offset: 0,
            unseen_output: 0,
            search_generation: 0,
            last_client_sequence: 0,
            pending_scroll_resync: false,
            lagged_events: 0,
        }
    }

    fn note_lagged(&mut self, skipped: u64) {
        let skipped = skipped.max(1);
        self.lagged_events = self.lagged_events.saturating_add(skipped);
        if self.display_offset != 0 {
            self.pending_scroll_resync = true;
            self.unseen_output = self.unseen_output.saturating_add(skipped);
        }
    }

    fn take_scroll_resync(&mut self) -> bool {
        let pending = self.pending_scroll_resync;
        self.pending_scroll_resync = false;
        pending
    }
}

const MAX_REQUEST_JOURNAL_ENTRIES: usize = 256;
const MAX_REQUEST_JOURNAL_BYTES: usize = 4 * 1024 * 1024;

struct DigestWriter(Sha256);

impl io::Write for DigestWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.update(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct CountingWriter(usize);

impl io::Write for CountingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self.0.saturating_add(bytes.len());
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[derive(Clone)]
struct RequestJournalEntry {
    request_id: u64,
    fingerprint: [u8; 32],
    response: HostResponse,
    encoded_bytes: usize,
}

#[derive(Default)]
struct RequestJournal {
    entries: VecDeque<RequestJournalEntry>,
    encoded_bytes: usize,
}

enum JournalLookup {
    Miss,
    Replay(Box<HostResponse>),
    Conflict,
}

impl RequestJournal {
    fn lookup(&self, request_id: u64, fingerprint: &[u8; 32]) -> JournalLookup {
        let Some(entry) = self
            .entries
            .iter()
            .find(|entry| entry.request_id == request_id)
        else {
            return JournalLookup::Miss;
        };
        if entry.fingerprint == *fingerprint {
            JournalLookup::Replay(Box::new(entry.response.clone()))
        } else {
            JournalLookup::Conflict
        }
    }

    fn insert(&mut self, fingerprint: [u8; 32], response: HostResponse) {
        let mut counter = CountingWriter::default();
        if serde_json::to_writer(&mut counter, &response).is_err()
            || counter.0 > MAX_REQUEST_JOURNAL_BYTES
        {
            return;
        }
        while self.entries.len() >= MAX_REQUEST_JOURNAL_ENTRIES
            || self.encoded_bytes.saturating_add(counter.0) > MAX_REQUEST_JOURNAL_BYTES
        {
            let Some(removed) = self.entries.pop_front() else {
                break;
            };
            self.encoded_bytes = self.encoded_bytes.saturating_sub(removed.encoded_bytes);
        }
        self.encoded_bytes = self.encoded_bytes.saturating_add(counter.0);
        self.entries.push_back(RequestJournalEntry {
            request_id: response.request_id,
            fingerprint,
            response,
            encoded_bytes: counter.0,
        });
    }
}

fn request_is_journalable(request: &Request) -> bool {
    // Credential and SSH connect messages carry secrets and must never enter
    // the replayable request journal, even when a later remote profile exists.
    !matches!(
        request,
        Request::SshConnect(_) | Request::CredentialAnswer { .. }
    )
}

fn authorize_local_capability(request: &Request) -> Result<(), ProtocolFailure> {
    match request.required_capability() {
        None | Some(_) => Ok(()),
    }
}

fn record_audit(
    audit: &HostAuditLog,
    request_id: u64,
    client_id: &ClientInstanceId,
    actor_device_id: Option<String>,
    request: &Request,
    result: AuditResult,
) {
    let Some(action) = request.audit_action() else {
        return;
    };
    audit.record(AuditEntry {
        request_id,
        actor_client_id: client_id.to_string(),
        actor_device_id,
        action: action.to_string(),
        resource: request.audit_resource(),
        result,
    });
}

fn request_fingerprint(request: &Request) -> [u8; 32] {
    let mut writer = DigestWriter(Sha256::new());
    match request {
        Request::TerminalInput(input) => {
            writer.0.update(b"terminal-input");
            writer.0.update(input.session_id.as_str().as_bytes());
            writer.0.update(input.context.host_epoch.to_le_bytes());
            writer.0.update(input.context.session_epoch.to_le_bytes());
            writer.0.update(input.context.lease_epoch.to_le_bytes());
            writer.0.update(input.context.geometry_epoch.to_le_bytes());
            writer.0.update(input.context.client_sequence.to_le_bytes());
        }
        _ => {
            serde_json::to_writer(&mut writer, request)
                .expect("serializing a protocol request for journaling must succeed");
        }
    }
    writer.0.finalize().into()
}

async fn serve_connection(
    mut stream: TransportStream,
    token: Arc<AuthToken>,
    context: ConnectionContext,
) -> Result<(), ()> {
    let ConnectionContext {
        identity,
        profile_id: _,
        host_sequence,
        ssh,
        agent_hooks,
        projects,
        request_journals,
        runtime,
        lifecycle,
        audit: _,
        stop,
    } = &context;
    let authenticated = server_handshake(&mut stream, identity, token.as_ref())
        .await
        .map_err(|_| ())?;
    if authenticated.channel == yttt_protocol::ConnectionChannel::DesktopOwner {
        if !lifecycle.desktop_owner_connected() {
            return Err(());
        }
        let result = serve_desktop_owner_connection(stream, stop.clone()).await;
        lifecycle.desktop_owner_disconnected();
        lifecycle.resource_changed();
        return result;
    }
    lifecycle.client_connected();
    if authenticated.channel == yttt_protocol::ConnectionChannel::Lifecycle {
        let result =
            serve_lifecycle_connection(stream, authenticated.can_force_stop, &context).await;
        lifecycle.client_disconnected();
        lifecycle.resource_changed();
        return result;
    }
    let client_id = authenticated.client_instance_id;
    if authenticated.channel == yttt_protocol::ConnectionChannel::TerminalData {
        let session_id = authenticated.terminal_session_id.ok_or(())?;
        let result = serve_terminal_data_connection(
            stream,
            client_id,
            session_id,
            runtime,
            host_sequence,
            stop.clone(),
        )
        .await;
        lifecycle.client_disconnected();
        lifecycle.resource_changed();
        return result;
    }
    let request_journal = {
        let mut journals = request_journals.lock();
        journals
            .entry(client_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(RequestJournal::default())))
            .clone()
    };
    let mut ssh_events = ssh.subscribe();
    let mut agent_hook_events = agent_hooks.subscribe();
    let mut project_events = projects.subscribe();
    let mut runtime_events = runtime.subscribe();
    let mut subscriptions = HashMap::<_, TerminalAttachment>::new();
    let mut stop = stop.clone();
    let result = async {
    loop {
        tokio::select! {
            message = receive_control(&mut stream) => {
                let message = message.map_err(|_| ())?;
                let request = match message {
                    ControlMessage::Request(request) => request,
                    ControlMessage::TerminalInput(input) => {
                        let response = handle_request(
                            ClientRequest {
                                request_id: 0,
                                actor_device_id: Some(client_id.to_string()),
                                lease_epoch: None,
                                body: Request::TerminalInput(input),
                            },
                            &context,
                            &client_id,
                            &mut subscriptions,
                            host_sequence.fetch_add(1, Ordering::Relaxed),
                        )
                        .await;
                        if let Err(error) = response.result {
                            eprintln!("one-way terminal input rejected: {error:?}");
                        }
                        continue;
                    }
                    ControlMessage::Response(_) | ControlMessage::Event(_) => return Err(()),
                };
                let subscription = match &request.body {
                    Request::SpawnTerminal(spec) => Some((
                        spec.session_id.clone(),
                        TerminalLeaseMode::Interactive,
                        true,
                    )),
                    Request::AttachTerminal(attach) => {
                        Some((attach.session_id.clone(), attach.mode, true))
                    }
                    Request::AcquireTerminalLease { session_id, mode } => {
                        Some((session_id.clone(), *mode, true))
                    }
                    Request::ReleaseTerminalLease { session_id }
                    | Request::ReleaseTerminalControl { session_id } => {
                        Some((session_id.clone(), TerminalLeaseMode::Observer, true))
                    }
                    Request::RequestTerminalControl { session_id } => {
                        Some((session_id.clone(), TerminalLeaseMode::Observer, false))
                    }
                    Request::RequestCheckpoint { session_id, .. } => Some((
                        session_id.clone(),
                        TerminalLeaseMode::Observer,
                        false,
                    )),
                    _ => None,
                };
                let unsubscription = match &request.body {
                    Request::DetachTerminal { session_id }
                    | Request::TerminateTerminal { session_id, .. }
                    | Request::AcknowledgeTerminalExit {
                        session_id,
                        ..
                    } => Some(session_id.clone()),
                    _ => None,
                };
                let (response, apply_effects) = if request_is_journalable(&request.body) {
                    let fingerprint = request_fingerprint(&request.body);
                    let mut journal = request_journal.lock().await;
                    match journal.lookup(request.request_id, &fingerprint) {
                        JournalLookup::Replay(response) => (*response, false),
                        JournalLookup::Conflict => (
                            HostResponse {
                                request_id: request.request_id,
                                result: Err(ProtocolFailure::new(
                                    FailureCode::InvalidRequest,
                                    "request ID was already used for a different request",
                                    false,
                                )),
                            },
                            false,
                        ),
                        JournalLookup::Miss => {
                            let response = handle_request(
                                request,
                                &context,
                                &client_id,
                                &mut subscriptions,
                                host_sequence.fetch_add(1, Ordering::Relaxed),
                            )
                            .await;
                            journal.insert(fingerprint, response.clone());
                            (response, true)
                        }
                    }
                } else {
                    (
                        handle_request(
                            request,
                            &context,
                            &client_id,
                            &mut subscriptions,
                            host_sequence.fetch_add(1, Ordering::Relaxed),
                        )
                        .await,
                        true,
                    )
                };
                if apply_effects {
                    if let Ok(result) = &response.result {
                    if let Some((session_id, mode, replace_mode)) = subscription {
                        let granted = match result {
                            Response::TerminalSpawned { lease, .. }
                            | Response::TerminalAttached { lease, .. }
                            | Response::TerminalLease(lease) => Some(lease.clone()),
                            _ => None,
                        };
                        let lease_epoch = granted.as_ref().map_or_else(
                            || {
                                subscriptions
                                    .get(&session_id)
                                    .map_or(0, |attachment| attachment.lease_epoch)
                            },
                            |lease| lease.lease_epoch,
                        );
                        runtime.register_attachment(&session_id, &client_id);
                        let attachment = subscriptions.entry(session_id).or_insert_with(|| {
                            TerminalAttachment::new(
                                granted.as_ref().map_or(mode, |lease| lease.mode),
                                lease_epoch,
                            )
                        });
                        if let Some(lease) = granted {
                            attachment.mode = lease.mode;
                            attachment.lease_epoch = lease.lease_epoch;
                            attachment.last_client_sequence = 0;
                        } else if replace_mode {
                            attachment.mode = mode;
                            attachment.lease_epoch = lease_epoch;
                            attachment.last_client_sequence = 0;
                        }
                    }
                    match result {
                        Response::TerminalScrolled(read) | Response::TerminalViewport(read) => {
                            if let Some(attachment) =
                                subscriptions.get_mut(&read.viewport.session_id)
                            {
                                attachment.display_offset = read.viewport.display_offset;
                                runtime.update_attachment_display_offset(
                                    &read.viewport.session_id,
                                    &client_id,
                                    read.viewport.display_offset,
                                );
                                if attachment.display_offset == 0 {
                                    attachment.unseen_output = 0;
                                    if attachment.take_scroll_resync() {
                                        runtime.mark_attachment_resync(
                                            &read.viewport.session_id,
                                            &client_id,
                                            AttachmentResyncReason::ReturnToBottom,
                                        );
                                    }
                                }
                            }
                        }
                        Response::TerminalSearch(results) => {
                            if let Some(attachment) =
                                subscriptions.get_mut(&results.session_id)
                            {
                                attachment.search_generation = results.generation;
                            }
                        }
                        Response::TerminalsTerminated { results } => {
                            for result in results.iter().filter(|result| result.result.is_ok()) {
                                subscriptions.remove(&result.session_id);
                                runtime.release_attachment(&result.session_id, &client_id);
                            }
                        }
                        _ => {}
                    }
                    if let Some(session_id) = unsubscription {
                        subscriptions.remove(&session_id);
                        runtime.release_attachment(&session_id, &client_id);
                    }
                    }
                    lifecycle.resource_changed();
                }
                send_control(&mut stream, &ControlMessage::Response(response))
                    .await
                    .map_err(|_| ())?;
            }
            event = runtime_events.recv() => {
                let server_event = match event {
                    Ok(HostTerminalEvent::Update { session_id, .. }) => {
                        if let Some(attachment) = subscriptions.get_mut(&session_id)
                            && attachment.display_offset != 0
                        {
                            attachment.unseen_output =
                                attachment.unseen_output.saturating_add(1);
                        }
                        None
                    }
                    Ok(HostTerminalEvent::Exited {
                        session_id,
                        session_epoch,
                        code,
                        final_sequence,
                    }) if subscriptions.contains_key(&session_id) => {
                        Some(ServerEvent::TerminalExit {
                            session_id,
                            session_epoch,
                            code,
                            final_sequence,
                        })
                    }
                    Ok(HostTerminalEvent::LeaseRevoked {
                        session_id,
                        previous_owner,
                    }) if previous_owner == client_id => {
                        if let Some(attachment) = subscriptions.get_mut(&session_id) {
                            attachment.mode = TerminalLeaseMode::Observer;
                        }
                        Some(ServerEvent::TerminalLeaseRevoked {
                            session_id,
                            previous_owner,
                        })
                    }
                    Ok(HostTerminalEvent::LeaseReleased {
                        session_id,
                        previous_owner,
                    }) if previous_owner == client_id => {
                        if let Some(attachment) = subscriptions.get_mut(&session_id) {
                            attachment.mode = TerminalLeaseMode::Observer;
                        }
                        Some(ServerEvent::TerminalLeaseReleased {
                            session_id,
                            previous_owner,
                        })
                    }
                    Ok(HostTerminalEvent::LeaseExpired {
                        session_id,
                        previous_owner,
                    }) if previous_owner == client_id => {
                        if let Some(attachment) = subscriptions.get_mut(&session_id) {
                            attachment.mode = TerminalLeaseMode::Observer;
                        }
                        Some(ServerEvent::TerminalLeaseExpired {
                            session_id,
                            previous_owner,
                        })
                    }
                    Ok(HostTerminalEvent::ControlRequested {
                        session_id,
                        holder,
                        requester,
                    }) if holder == client_id => Some(ServerEvent::TerminalControlRequested {
                        session_id,
                        requester,
                    }),
                    Ok(HostTerminalEvent::ControlGranted { session_id, lease })
                        if lease.owner == client_id =>
                    {
                        if let Some(attachment) = subscriptions.get_mut(&session_id) {
                            attachment.mode = TerminalLeaseMode::Interactive;
                            attachment.lease_epoch = lease.lease_epoch;
                            attachment.last_client_sequence = 0;
                        }
                        Some(ServerEvent::TerminalControlGranted { lease })
                    }
                    Ok(HostTerminalEvent::ControlDenied {
                        session_id,
                        requester,
                        reason,
                    }) if requester == client_id => Some(ServerEvent::TerminalControlDenied {
                        session_id,
                        requester,
                        reason,
                    }),
                    Ok(HostTerminalEvent::TitleChanged { .. } | HostTerminalEvent::Bell { .. }) => {
                        None
                    }
                    Ok(_) => None,
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        for (session_id, attachment) in subscriptions.iter_mut() {
                            let scrolled = attachment.display_offset != 0;
                            attachment.note_lagged(skipped);
                            runtime.note_attachment_lag(
                                session_id,
                                &client_id,
                                skipped,
                                scrolled,
                            );
                        }
                        None
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                };
                if let Some(body) = server_event {
                    send_control(
                        &mut stream,
                        &ControlMessage::Event(HostEvent {
                            host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                            body,
                        }),
                    )
                    .await
                    .map_err(|_| ())?;
                }
            }
            event = ssh_events.recv() => {
                let body = match event {
                    Ok(event) => match event.for_client(&client_id) {
                        Some(body) => body.clone(),
                        None => continue,
                    },
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                };
                send_control(
                    &mut stream,
                    &ControlMessage::Event(HostEvent {
                        host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                        body,
                    }),
                )
                .await
                .map_err(|_| ())?;
            }
            event = project_events.recv() => {
                match event {
                    Ok(change) => {
                        send_control(
                            &mut stream,
                            &ControlMessage::Event(HostEvent {
                                host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                                body: ServerEvent::ProjectChanged(change),
                            }),
                        )
                        .await
                        .map_err(|_| ())?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for change in projects.resync_changes() {
                            send_control(
                                &mut stream,
                                &ControlMessage::Event(HostEvent {
                                    host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                                    body: ServerEvent::ProjectChanged(change),
                                }),
                            )
                            .await
                            .map_err(|_| ())?;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                }
            }
            event = agent_hook_events.recv() => {
                let updates = match event {
                    Ok(update) => vec![update],
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        agent_hooks.snapshots_after(&[])
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                };
                for update in updates {
                    send_control(
                        &mut stream,
                        &ControlMessage::Event(HostEvent {
                            host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                            body: ServerEvent::AgentSnapshot(Box::new(update)),
                        }),
                    )
                    .await
                    .map_err(|_| ())?;
                }
            }
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    let _ = send_control(
                        &mut stream,
                        &ControlMessage::Event(HostEvent {
                            host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                            body: ServerEvent::HostStopping,
                        }),
                    )
                    .await;
                    return Ok(());
                }
            }
        }
    }
    }.await;
    runtime.release_client(&client_id);
    ssh.abandon_challenges(&client_id);
    lifecycle.client_disconnected();
    lifecycle.resource_changed();
    result
}
async fn serve_desktop_owner_connection(
    mut stream: TransportStream,
    mut stop: watch::Receiver<bool>,
) -> Result<(), ()> {
    let mut byte = [0_u8; 1];
    tokio::select! {
        read = stream.read(&mut byte) => match read {
            Ok(0) => Ok(()),
            Ok(_) | Err(_) => Err(()),
        },
        changed = stop.changed() => {
            if changed.is_err() || *stop.borrow() {
                Ok(())
            } else {
                Err(())
            }
        }
    }
}

async fn serve_lifecycle_connection(
    mut stream: TransportStream,
    can_force_stop: bool,
    context: &ConnectionContext,
) -> Result<(), ()> {
    let ConnectionContext {
        identity,
        runtime,
        lifecycle,
        ssh,
        projects,
        agent_hooks,
        stop,
        ..
    } = context;
    let mut stop = stop.clone();
    loop {
        tokio::select! {
            message = receive_lifecycle(&mut stream) => {
                let LifecycleMessage::Request(request) = message.map_err(|_| ())? else {
                    return Err(());
                };
                let result = match request.body {
                    LifecycleRequest::Probe => LifecycleResponse::Pong,
                    LifecycleRequest::Status => {
                        let ssh_connections = ssh.connections();
                        let project_ids = projects.projects();
                        let blockers = lifecycle.blockers(
                            runtime,
                            ssh_connections.clone(),
                            project_ids.clone(),
                        );
                        let terminal_count = blockers
                            .iter()
                            .filter(|blocker| {
                                matches!(
                                    blocker,
                                    yttt_protocol::HostBlocker::RunningTerminal(_)
                                        | yttt_protocol::HostBlocker::ExitedTerminalAwaitingAck { .. }
                                )
                            })
                            .count()
                            .min(u32::MAX as usize) as u32;
                        LifecycleResponse::Status(HostLifecycleStatus {
                            lifecycle_protocol: LIFECYCLE_PROTOCOL_VERSION,
                            resource_protocol: RESOURCE_PROTOCOL_VERSION,
                            build: identity.build.clone(),
                            state: lifecycle.state(),
                            terminal_count,
                            client_count: lifecycle
                                .client_count()
                                .saturating_sub(1)
                                .min(u32::MAX as usize) as u32,
                            project_count: project_ids.len().min(u32::MAX as usize) as u32,
                            ssh_connection_count: ssh_connections.len().min(u32::MAX as usize)
                                as u32,
                            agent_count: agent_hooks
                                .active_agent_count()
                                .min(u32::MAX as usize) as u32,
                            blockers,
                        })
                    }
                    LifecycleRequest::StopIfIdle => {
                        match lifecycle.stop_if_idle(
                            runtime,
                            ssh.connections(),
                            projects.projects(),
                        ) {
                            Ok(()) => LifecycleResponse::Stopping,
                            Err(blockers) => LifecycleResponse::Busy { blockers },
                        }
                    }
                    LifecycleRequest::BeginDrain => {
                        lifecycle.begin_drain();
                        LifecycleResponse::Draining
                    }
                    LifecycleRequest::ForceStop if can_force_stop => {
                        lifecycle.force_stop();
                        LifecycleResponse::Draining
                    }
                    LifecycleRequest::ForceStop => LifecycleResponse::PermissionDenied,
                };
                send_lifecycle(
                    &mut stream,
                    &LifecycleMessage::Response(LifecycleResponseEnvelope {
                        request_id: request.request_id,
                        result,
                    }),
                )
                .await
                .map_err(|_| ())?;
            }
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
        }
    }
}

async fn serve_terminal_data_connection(
    stream: TransportStream,
    client_id: ClientInstanceId,
    session_id: TerminalSessionId,
    runtime: &Arc<HostRuntime>,
    host_sequence: &Arc<AtomicU64>,
    mut stop: watch::Receiver<bool>,
) -> Result<(), ()> {
    let terminal = runtime.terminal(&session_id).ok_or(())?;
    let mut attachment = runtime
        .attachment_receiver(&session_id, &client_id)
        .ok_or(())?;
    let mut attachment_state = *attachment.borrow();
    if !attachment_state.attached {
        return Err(());
    }
    let mut terminal_events = terminal.subscribe();
    let output = TerminalDataWriter::new(stream, runtime.new_attachment_queue_diagnostics());
    let mut output_failure = output.subscribe_failure();
    let checkpoint = terminal.control_checkpoint().ok_or(())?;
    output
        .enqueue(
            host_sequence,
            TerminalStreamUpdate::Snapshot(checkpoint.viewport),
        )
        .map_err(|_| ())?;
    if let Some(after_sequence) = attachment_state.pending_raw_after {
        runtime.set_pending_raw_after(&session_id, &client_id, None);
        match terminal.raw_replay_chunks(after_sequence) {
            Ok(chunks) => {
                for bytes in chunks {
                    output
                        .enqueue_paced(
                            host_sequence,
                            TerminalStreamUpdate::RawTail {
                                session_id: session_id.clone(),
                                session_epoch: terminal.session_epoch(),
                                sequence: terminal
                                    .latest_viewport()
                                    .map_or(0, |viewport| viewport.sequence),
                                bytes,
                            },
                        )
                        .await
                        .map_err(|_| ())?;
                }
            }
            Err(HostedTerminalError::ResyncRequired {
                available_from_sequence,
                ..
            }) => {
                output
                    .enqueue(
                        host_sequence,
                        TerminalStreamUpdate::ResyncRequired {
                            session_id: session_id.clone(),
                            available_from_sequence,
                        },
                    )
                    .map_err(|_| ())?;
            }
            Err(_) => return Err(()),
        }
    }
    if terminal.is_exited() {
        let checkpoint = terminal.control_checkpoint().ok_or(())?;
        output
            .enqueue(
                host_sequence,
                TerminalStreamUpdate::Snapshot(checkpoint.viewport),
            )
            .map_err(|_| ())?;
        drain_terminal_data(&output).await?;
        return Ok(());
    }
    loop {
        tokio::select! {
            event = terminal_events.recv() => {
                match event {
                    Ok(HostTerminalEvent::Update {
                        session_id: update_session_id,
                        update,
                    }) if update_session_id == session_id
                        && attachment_state.display_offset == 0 =>
                    {
                        output
                            .enqueue_shared(host_sequence, update.as_ref())
                            .map_err(|_| ())?;
                    }
                    Ok(HostTerminalEvent::Exited {
                        session_id: exited_session_id,
                        ..
                    }) if exited_session_id == session_id => {
                        let checkpoint = terminal.control_checkpoint().ok_or(())?;
                        output
                            .enqueue(
                                host_sequence,
                                TerminalStreamUpdate::Snapshot(checkpoint.viewport),
                            )
                            .map_err(|_| ())?;
                        drain_terminal_data(&output).await?;
                        return Ok(());
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped))
                        if attachment_state.display_offset == 0 =>
                    {
                        runtime.note_attachment_lag(&session_id, &client_id, skipped, false);
                        let available_from_sequence = terminal
                            .latest_viewport()
                            .map_or(0, |viewport| viewport.sequence);
                        output
                            .enqueue(
                                host_sequence,
                                TerminalStreamUpdate::ResyncRequired {
                                    session_id: session_id.clone(),
                                    available_from_sequence,
                                },
                            )
                            .map_err(|_| ())?;
                    }
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        runtime.note_attachment_lag(&session_id, &client_id, skipped, true);
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                }
            }
            changed = attachment.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
                let previous_offset = attachment_state.display_offset;
                attachment_state = *attachment.borrow();
                if !attachment_state.attached {
                    return Ok(());
                }
                if previous_offset != 0 && attachment_state.display_offset == 0 {
                    runtime.mark_attachment_resync(
                        &session_id,
                        &client_id,
                        AttachmentResyncReason::ReturnToBottom,
                    );
                    let checkpoint = terminal.control_checkpoint().ok_or(())?;
                    output
                        .enqueue(
                            host_sequence,
                            TerminalStreamUpdate::Snapshot(checkpoint.viewport),
                        )
                        .map_err(|_| ())?;
                }
            }
            _ = output_failure.changed() => return Err(()),
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    return Ok(());
                }
            }
        }
    }
}

async fn drain_terminal_data(output: &TerminalDataWriter) -> Result<(), ()> {
    tokio::time::timeout(Duration::from_secs(1), output.drain())
        .await
        .map_err(|_| ())?
}
async fn handle_request(
    request: ClientRequest,
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
    attachments: &mut HashMap<yttt_core::model::ids::TerminalSessionId, TerminalAttachment>,
    host_sequence: u64,
) -> HostResponse {
    let ConnectionContext {
        identity,
        profile_id,
        ssh,
        agent_hooks,
        projects,
        runtime,
        lifecycle,
        audit,
        ..
    } = context;
    let ClientRequest {
        request_id,
        actor_device_id,
        body,
        ..
    } = request;
    if let Err(failure) = authorize_local_capability(&body) {
        record_audit(
            audit,
            request_id,
            client_id,
            actor_device_id,
            &body,
            AuditResult::Denied,
        );
        return HostResponse {
            request_id,
            result: Err(failure),
        };
    }
    let audit_action = body
        .audit_action()
        .map(|action| (action, body.audit_resource(), actor_device_id.clone()));
    let _resource_admission = if request_creates_resource(&body) {
        let Some(admission) = lifecycle.admit_resource() else {
            return HostResponse {
                request_id,
                result: Err(host_stopping_failure()),
            };
        };
        Some(admission)
    } else {
        None
    };
    let result = match body {
        Request::Ping { sent_millis } => Ok(Response::Pong {
            sent_millis,
            host_millis: now_millis(),
        }),
        Request::ListResources => Ok(Response::Resources(ResourceCatalog {
            profile_id: profile_id.clone(),
            host_id: identity.host_id.clone(),
            host_epoch: identity.host_epoch,
            revision: host_sequence,
            terminals: runtime.placements(),
            ssh_connections: ssh.connections(),
            projects: projects.projects(),
        })),
        Request::SpawnTerminal(mut spec) => {
            let scope = agent_hooks.secure_terminal_environment(&mut spec);
            let local_cwd = projects
                .local_root(&spec.project_id)
                .ok()
                .map(|root| spec.cwd.join_under(&root));
            match runtime.spawn_with_transport(spec, Some(ssh.transport()), local_cwd) {
                Ok(terminal) => runtime
                    .acquire_lease(
                        terminal.session_id(),
                        client_id,
                        TerminalLeaseMode::Interactive,
                    )
                    .map(|lease| Response::TerminalSpawned {
                        lease,
                        session_epoch: terminal.session_epoch(),
                    })
                    .map_err(runtime_failure),
                Err(error) => {
                    agent_hooks.cancel_terminal(&scope);
                    Err(runtime_failure(error))
                }
            }
        }
        Request::AttachTerminal(attach) => {
            if attach.mode == TerminalLeaseMode::Observer
                && attachments
                    .get(&attach.session_id)
                    .is_some_and(|attachment| attachment.mode == TerminalLeaseMode::Interactive)
            {
                runtime.release_lease(&attach.session_id, client_id);
            }
            attach_terminal(runtime, client_id, attach).map_err(runtime_failure)
        }
        Request::DetachTerminal { session_id } => {
            if attachments
                .get(&session_id)
                .is_some_and(|attachment| attachment.mode == TerminalLeaseMode::Interactive)
            {
                runtime.release_lease(&session_id, client_id);
            }
            Ok(Response::TerminalDetached)
        }
        Request::ReleaseTerminalLease { session_id } => (|| {
            require_interactive_attachment(attachments, &session_id)?;
            runtime.release_lease(&session_id, client_id);
            Ok(Response::TerminalDetached)
        })()
        .map_err(runtime_failure),
        Request::AcquireTerminalLease { session_id, mode } => runtime
            .acquire_lease(&session_id, client_id, mode)
            .map(Response::TerminalLease)
            .map_err(runtime_failure),
        Request::RequestTerminalControl { session_id } => runtime
            .request_terminal_control(&session_id, client_id)
            .map(|outcome| match outcome {
                TerminalControlOutcome::Granted(lease) => Response::TerminalLease(lease),
                TerminalControlOutcome::Pending { holder } => {
                    Response::TerminalControlPending { session_id, holder }
                }
            })
            .map_err(runtime_failure),
        Request::ReleaseTerminalControl { session_id } => (|| {
            require_interactive_attachment(attachments, &session_id)?;
            runtime.release_control(&session_id, client_id)?;
            Ok(Response::Applied)
        })()
        .map_err(runtime_failure),
        Request::TerminalInput(input) => (|| {
            let terminal = runtime
                .terminal(&input.session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(input.session_id.clone()))?;
            validate_terminal_mutation(
                identity.host_epoch,
                attachments,
                &terminal,
                &input.session_id,
                input.context,
                true,
                true,
            )?;
            runtime.validate_lease(&input.session_id, client_id)?;
            terminal.input(input.bytes)?;
            accept_terminal_mutation(
                attachments,
                &input.session_id,
                input.context.client_sequence,
            );
            Ok(Response::TerminalInputAccepted {
                client_sequence: input.context.client_sequence,
            })
        })()
        .map_err(runtime_failure),
        Request::ResizeTerminal(request) => (|| {
            let terminal = runtime
                .terminal(&request.session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(request.session_id.clone()))?;
            validate_terminal_mutation(
                identity.host_epoch,
                attachments,
                &terminal,
                &request.session_id,
                request.context,
                true,
                false,
            )?;
            runtime.validate_lease(&request.session_id, client_id)?;
            terminal.resize(request.geometry, request.context.geometry_epoch)?;
            accept_terminal_mutation(
                attachments,
                &request.session_id,
                request.context.client_sequence,
            );
            Ok(Response::TerminalResized {
                geometry_epoch: request.context.geometry_epoch,
            })
        })()
        .map_err(runtime_failure),
        Request::ScrollTerminal(request) => (|| {
            let terminal = runtime
                .terminal(&request.session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(request.session_id.clone()))?;
            validate_terminal_mutation(
                identity.host_epoch,
                attachments,
                &terminal,
                &request.session_id,
                request.context,
                false,
                true,
            )?;
            let unseen_output = attachments
                .get(&request.session_id)
                .map_or(0, |attachment| attachment.unseen_output);
            let current = terminal
                .latest_viewport()
                .ok_or_else(|| HostRuntimeError::NotFound(request.session_id.clone()))?;
            let viewport = terminal.read_viewport(
                current.scrollback_epoch,
                TerminalViewportAnchor::DisplayOffset(request.display_offset),
            )?;
            accept_terminal_mutation(
                attachments,
                &request.session_id,
                request.context.client_sequence,
            );
            Ok(Response::TerminalScrolled(terminal_viewport_read(
                &terminal,
                viewport,
                unseen_output,
            )))
        })()
        .map_err(runtime_failure),
        Request::ReadTerminalViewport(request) => (|| {
            require_attachment(attachments, &request.session_id)?;
            let terminal = runtime
                .terminal(&request.session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(request.session_id.clone()))?;
            validate_terminal_session_epoch(&terminal, request.session_epoch)?;
            let viewport = terminal.read_viewport(request.scrollback_epoch, request.anchor)?;
            let attachment = attachments
                .get_mut(&request.session_id)
                .expect("attachment was required");
            if attachment.take_scroll_resync() {
                runtime.mark_attachment_resync(
                    &request.session_id,
                    client_id,
                    AttachmentResyncReason::ViewportRead,
                );
            }
            Ok(Response::TerminalViewport(terminal_viewport_read(
                &terminal,
                viewport,
                attachment.unseen_output,
            )))
        })()
        .map_err(runtime_failure),
        Request::SearchTerminal(request) => (|| {
            let attachment = require_attachment(attachments, &request.session_id)?;
            if request.generation < attachment.search_generation {
                return Err(HostRuntimeError::StaleSearchGeneration {
                    session_id: request.session_id,
                    received: request.generation,
                    current: attachment.search_generation,
                });
            }
            let terminal = runtime
                .terminal(&request.session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(request.session_id.clone()))?;
            validate_terminal_session_epoch(&terminal, request.session_epoch)?;
            terminal
                .search(&request)
                .map(Response::TerminalSearch)
                .map_err(HostRuntimeError::from)
        })()
        .map_err(runtime_failure),
        Request::SetTerminalQueryPalette(request) => (|| {
            let terminal = runtime
                .terminal(&request.session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(request.session_id.clone()))?;
            validate_terminal_mutation(
                identity.host_epoch,
                attachments,
                &terminal,
                &request.session_id,
                request.context,
                true,
                true,
            )?;
            runtime.validate_lease(&request.session_id, client_id)?;
            terminal.set_query_palette(request.colors, request.revision);
            accept_terminal_mutation(
                attachments,
                &request.session_id,
                request.context.client_sequence,
            );
            Ok(Response::TerminalPaletteAccepted {
                revision: request.revision,
            })
        })()
        .map_err(runtime_failure),
        Request::RequestCheckpoint {
            session_id,
            after_sequence,
        } => (|| {
            let terminal = runtime
                .terminal(&session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
            if let Some(after_sequence) = after_sequence {
                terminal.raw_replay_chunks(after_sequence)?;
                runtime.set_pending_raw_after(&session_id, client_id, Some(after_sequence));
            }
            terminal
                .control_checkpoint()
                .map(Response::TerminalCheckpoint)
                .ok_or(HostRuntimeError::NotFound(session_id))
        })()
        .map_err(runtime_failure),
        Request::AcknowledgeTerminalExit {
            session_id,
            session_epoch,
            final_sequence,
        } => runtime
            .acknowledge_terminal_exit(&session_id, session_epoch, final_sequence)
            .map(|()| Response::TerminalExitAcknowledged)
            .map_err(runtime_failure),
        Request::TerminateTerminal { session_id, mode } => match mode {
            TerminationMode::Detach => {
                if attachments
                    .get(&session_id)
                    .is_some_and(|attachment| attachment.mode == TerminalLeaseMode::Interactive)
                {
                    runtime.release_lease(&session_id, client_id);
                }
                Ok(Response::TerminalDetached)
            }
            TerminationMode::Terminate | TerminationMode::TerminateMany => (|| {
                require_interactive_attachment(attachments, &session_id)?;
                runtime.validate_lease(&session_id, client_id)?;
                runtime
                    .terminate(&session_id)
                    .map(Response::TerminalTerminated)
            })()
            .map_err(runtime_failure),
        },
        Request::TerminateMany { requests } => {
            let results = requests
                .into_iter()
                .map(|request| {
                    let session_id = request.session_id;
                    let result = (|| {
                        require_interactive_attachment(attachments, &session_id)?;
                        runtime.validate_lease(&session_id, client_id)?;
                        let terminated = runtime.terminate(&session_id)?;
                        runtime.acknowledge_terminal_exit(
                            &session_id,
                            terminated.session_epoch,
                            terminated.final_sequence,
                        )?;
                        Ok(terminated)
                    })()
                    .map_err(runtime_failure);
                    TerminalTerminationResult {
                        request_id: request.request_id,
                        session_id,
                        result,
                    }
                })
                .collect();
            Ok(Response::TerminalsTerminated { results })
        }
        Request::SshConnect(spec) => ssh
            .connect(spec, client_id.clone())
            .await
            .map_err(ssh_failure),
        Request::SshDisconnect { connection_id } => {
            ssh.disconnect(connection_id).await.map_err(ssh_failure)
        }
        Request::CredentialAnswer {
            challenge_id,
            answer,
        } => ssh.answer_credential(challenge_id, answer, client_id),
        Request::DeleteSshCredential { credential_id } => {
            ssh.delete_credential(credential_id).map_err(ssh_failure)
        }
        Request::RemoteFile(request) => {
            let ssh = ssh.clone();
            let projects = projects.clone();
            tokio::task::spawn_blocking(move || ssh.remote_file(&projects, request))
                .await
                .map_err(|error| ssh_failure(error.to_string()))
                .and_then(|result| result.map_err(project_failure))
        }
        Request::RemoteCommand(request) => {
            let ssh = ssh.clone();
            let projects = projects.clone();
            tokio::task::spawn_blocking(move || ssh.remote_command(&projects, request))
                .await
                .map_err(|error| ssh_failure(error.to_string()))
                .and_then(|result| result.map_err(project_failure))
        }
        Request::Project(request) => {
            let projects = projects.clone();
            tokio::task::spawn_blocking(move || projects.handle(request))
                .await
                .map_err(|error| {
                    ProtocolFailure::new(FailureCode::Internal, error.to_string(), false)
                })
                .and_then(|result| result.map(Response::Project).map_err(project_failure))
        }
        Request::ReadAgentSnapshots { acknowledged } => Ok(Response::AgentSnapshots(
            agent_hooks.snapshots_after(&acknowledged),
        )),
    };
    if let Some((action, resource, actor_device_id)) = audit_action {
        audit.record(AuditEntry {
            request_id,
            actor_client_id: client_id.to_string(),
            actor_device_id,
            action: action.to_string(),
            resource,
            result: if result.is_ok() {
                AuditResult::Allowed
            } else {
                AuditResult::Denied
            },
        });
    }
    HostResponse { request_id, result }
}

fn require_attachment<'a>(
    attachments: &'a HashMap<TerminalSessionId, TerminalAttachment>,
    session_id: &TerminalSessionId,
) -> Result<&'a TerminalAttachment, HostRuntimeError> {
    attachments
        .get(session_id)
        .ok_or_else(|| HostRuntimeError::LeaseRequired(session_id.clone()))
}

fn require_interactive_attachment(
    attachments: &HashMap<TerminalSessionId, TerminalAttachment>,
    session_id: &TerminalSessionId,
) -> Result<(), HostRuntimeError> {
    let attachment = require_attachment(attachments, session_id)?;
    if attachment.mode == TerminalLeaseMode::Interactive {
        Ok(())
    } else {
        Err(HostRuntimeError::LeaseRequired(session_id.clone()))
    }
}

fn validate_terminal_mutation(
    host_epoch: u64,
    attachments: &mut HashMap<TerminalSessionId, TerminalAttachment>,
    terminal: &crate::terminal::HostedTerminal,
    session_id: &TerminalSessionId,
    context: yttt_protocol::terminal::TerminalMutationContext,
    requires_interactive: bool,
    require_current_geometry: bool,
) -> Result<(), HostRuntimeError> {
    if context.host_epoch != host_epoch {
        return Err(HostRuntimeError::StaleHostEpoch {
            expected: host_epoch,
            actual: context.host_epoch,
        });
    }
    validate_terminal_session_epoch(terminal, context.session_epoch)?;
    let attachment = attachments
        .get(session_id)
        .ok_or_else(|| HostRuntimeError::LeaseRequired(session_id.clone()))?;
    if requires_interactive && attachment.mode != TerminalLeaseMode::Interactive {
        return Err(HostRuntimeError::LeaseRequired(session_id.clone()));
    }
    if context.lease_epoch != attachment.lease_epoch {
        return Err(HostRuntimeError::StaleLeaseEpoch {
            session_id: session_id.clone(),
            expected: attachment.lease_epoch,
            actual: context.lease_epoch,
        });
    }
    if context.client_sequence <= attachment.last_client_sequence {
        return Err(HostRuntimeError::StaleClientSequence {
            session_id: session_id.clone(),
            last_accepted: attachment.last_client_sequence,
            received: context.client_sequence,
        });
    }
    if require_current_geometry {
        let current = terminal
            .latest_viewport()
            .map_or(context.geometry_epoch, |viewport| viewport.geometry_epoch);
        if context.geometry_epoch != current {
            return Err(HostRuntimeError::Terminal(
                HostedTerminalError::StaleGeometry {
                    received: context.geometry_epoch,
                    current,
                },
            ));
        }
    }
    Ok(())
}

fn accept_terminal_mutation(
    attachments: &mut HashMap<TerminalSessionId, TerminalAttachment>,
    session_id: &TerminalSessionId,
    client_sequence: u64,
) {
    if let Some(attachment) = attachments.get_mut(session_id) {
        attachment.last_client_sequence = client_sequence;
    }
}

fn validate_terminal_session_epoch(
    terminal: &crate::terminal::HostedTerminal,
    received: u64,
) -> Result<(), HostRuntimeError> {
    if terminal.session_epoch() == received {
        Ok(())
    } else {
        Err(HostRuntimeError::StaleSessionEpoch {
            session_id: terminal.session_id().clone(),
            expected: terminal.session_epoch(),
            actual: received,
        })
    }
}

fn terminal_viewport_read(
    terminal: &crate::terminal::HostedTerminal,
    viewport: yttt_protocol::terminal::SemanticViewport,
    unseen_output: u64,
) -> TerminalViewportRead {
    let canonical = terminal.latest_viewport();
    TerminalViewportRead {
        bottom_line_id: canonical
            .as_ref()
            .and_then(|viewport| viewport.rows.last())
            .map(|row| row.line_id),
        checkpoint_sequence: canonical
            .as_ref()
            .map_or(viewport.sequence, |viewport| viewport.sequence),
        viewport,
        unseen_output,
    }
}

fn attach_terminal(
    runtime: &Arc<HostRuntime>,
    client_id: &ClientInstanceId,
    attach: AttachTerminal,
) -> Result<Response, HostRuntimeError> {
    let terminal = runtime
        .terminal(&attach.session_id)
        .ok_or_else(|| HostRuntimeError::NotFound(attach.session_id.clone()))?;
    if let Some(known_epoch) = attach.known_session_epoch
        && known_epoch != terminal.session_epoch()
    {
        return Err(HostRuntimeError::StaleSessionEpoch {
            session_id: attach.session_id,
            expected: terminal.session_epoch(),
            actual: known_epoch,
        });
    }
    let lease = match runtime.acquire_lease(terminal.session_id(), client_id, attach.mode) {
        Ok(lease) => lease,
        Err(HostRuntimeError::LeaseConflict { .. })
            if attach.mode == TerminalLeaseMode::Interactive =>
        {
            runtime.acquire_lease(
                terminal.session_id(),
                client_id,
                TerminalLeaseMode::Observer,
            )?
        }
        Err(error) => return Err(error),
    };
    if lease.mode == TerminalLeaseMode::Interactive {
        terminal.set_query_palette(attach.query_palette, attach.palette_revision);
        let current_geometry_epoch = terminal
            .latest_viewport()
            .map_or(0, |viewport| viewport.geometry_epoch);
        if attach.geometry_epoch > current_geometry_epoch {
            terminal.resize(attach.geometry, attach.geometry_epoch)?;
        }
    }
    let checkpoint = terminal
        .control_checkpoint()
        .ok_or_else(|| HostRuntimeError::NotFound(terminal.session_id().clone()))?;
    Ok(Response::TerminalAttached { lease, checkpoint })
}

fn request_creates_resource(request: &Request) -> bool {
    matches!(
        request,
        Request::SpawnTerminal(_)
            | Request::SshConnect(_)
            | Request::Project(
                yttt_protocol::project::ProjectRequest::Register { .. }
                    | yttt_protocol::project::ProjectRequest::RegisterSsh { .. }
            )
    )
}

fn host_stopping_failure() -> ProtocolFailure {
    ProtocolFailure::new(
        FailureCode::HostStopping,
        "Host is draining and no longer accepts new resources",
        false,
    )
}

fn runtime_failure(error: HostRuntimeError) -> ProtocolFailure {
    let code = match &error {
        HostRuntimeError::AlreadyExists(_) => FailureCode::AlreadyExists,
        HostRuntimeError::AddressConflict { .. } => FailureCode::AddressConflict,
        HostRuntimeError::NotFound(_) => FailureCode::NotFound,
        HostRuntimeError::LeaseConflict { .. }
        | HostRuntimeError::ControlRequestConflict { .. }
        | HostRuntimeError::TerminalStillRunning(_) => FailureCode::Conflict,
        HostRuntimeError::LeaseRequired(_) => FailureCode::PermissionDenied,
        HostRuntimeError::StaleHostEpoch { .. }
        | HostRuntimeError::StaleSessionEpoch { .. }
        | HostRuntimeError::StaleLeaseEpoch { .. }
        | HostRuntimeError::Terminal(
            HostedTerminalError::StaleGeometry { .. } | HostedTerminalError::StaleScrollback { .. },
        ) => FailureCode::StaleEpoch,
        HostRuntimeError::StaleClientSequence { .. }
        | HostRuntimeError::StaleFinalSequence { .. }
        | HostRuntimeError::StaleSearchGeneration { .. }
        | HostRuntimeError::Terminal(HostedTerminalError::UnknownLineId(_)) => {
            FailureCode::StaleSequence
        }
        HostRuntimeError::Terminal(HostedTerminalError::ResyncRequired { .. }) => {
            FailureCode::ResyncRequired
        }
        HostRuntimeError::Terminal(HostedTerminalError::Backpressure) => FailureCode::Backpressure,
        HostRuntimeError::Terminal(HostedTerminalError::Stopped) => FailureCode::TransportClosed,
        HostRuntimeError::Terminal(
            HostedTerminalError::InvalidGeometry | HostedTerminalError::InvalidSearch,
        ) => FailureCode::InvalidRequest,
        HostRuntimeError::Terminal(
            HostedTerminalError::UnsupportedExecution
            | HostedTerminalError::Io(_)
            | HostedTerminalError::Pty(_),
        ) => FailureCode::Internal,
    };
    ProtocolFailure::new(
        code,
        error.to_string(),
        matches!(code, FailureCode::ResyncRequired),
    )
}

fn ssh_failure(error: String) -> ProtocolFailure {
    ProtocolFailure::new(FailureCode::TransportClosed, error, false)
}

fn project_failure(error: HostProjectError) -> ProtocolFailure {
    ProtocolFailure::new(error.code, error.to_string(), false)
}

pub fn read_ready_metadata(path: &Path) -> Result<ReadyMetadata, HostError> {
    let bytes = fs::read(path)?;
    Ok(serde_json::from_slice(&bytes)?)
}
pub fn profile_lock_is_held(runtime_root: &Path) -> Result<bool, HostError> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let lock = options.open(runtime_root.join("host.lock"))?;
    match lock.try_lock_exclusive() {
        Ok(()) => {
            lock.unlock()?;
            Ok(false)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(true),
        Err(error) => Err(error.into()),
    }
}

fn read_auth_token(path: &Path) -> Result<AuthToken, HostError> {
    validate_user_only_file(path)?;
    let mut file = File::open(path)?;
    let mut bytes = [0_u8; 32];
    file.read_exact(&mut bytes)
        .map_err(|_| HostError::InvalidAuthToken)?;
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing)? != 0 {
        return Err(HostError::InvalidAuthToken);
    }
    Ok(AuthToken::from_bytes(bytes))
}

struct HostInstanceGuard {
    lock: File,
    pid_file: PathBuf,
    ready_file: PathBuf,
}

impl HostInstanceGuard {
    fn acquire(bootstrap: &HostBootstrap) -> Result<Self, HostError> {
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut lock = options.open(bootstrap.lock_file())?;
        lock.try_lock_exclusive()
            .map_err(|_| HostError::AlreadyRunning)?;
        let _ = fs::remove_file(bootstrap.ready_file());
        lock.set_len(0)?;
        lock.seek(SeekFrom::Start(0))?;
        writeln!(lock, "{}", std::process::id())?;
        lock.sync_all()?;
        atomic_write(
            &bootstrap.pid_file(),
            std::process::id().to_string().as_bytes(),
        )?;
        Ok(Self {
            lock,
            pid_file: bootstrap.pid_file(),
            ready_file: bootstrap.ready_file(),
        })
    }

    fn publish_ready(&self, ready: &ReadyMetadata) -> Result<(), HostError> {
        atomic_write(&self.ready_file, &serde_json::to_vec(ready)?)?;
        Ok(())
    }
}

impl Drop for HostInstanceGuard {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.ready_file);
        let _ = fs::remove_file(&self.pid_file);
        let _ = self.lock.unlock();
    }
}

fn next_host_epoch(runtime_root: &Path) -> Result<u64, HostError> {
    let path = runtime_root.join("host-epoch");
    let previous = fs::read_to_string(&path)
        .ok()
        .and_then(|value| value.trim().parse::<u64>().ok())
        .unwrap_or(0);
    let next = previous.saturating_add(1).max(1);
    atomic_write(&path, next.to_string().as_bytes())?;
    Ok(next)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let temporary = path.with_extension(format!("tmp-{}", std::process::id()));
    let mut options = OpenOptions::new();
    options.create(true).truncate(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    fs::rename(temporary, path)?;
    Ok(())
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(unix)]
fn secure_runtime_root(path: &Path) -> Result<(), HostError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.uid() != rustix::process::geteuid().as_raw() {
        return Err(HostError::InsecureBootstrapFile);
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}

#[cfg(windows)]
fn secure_runtime_root(path: &Path) -> Result<(), HostError> {
    if !fs::metadata(path)?.is_dir() {
        return Err(HostError::InsecureBootstrapFile);
    }
    Ok(())
}

#[cfg(unix)]
fn validate_user_only_file(path: &Path) -> Result<(), HostError> {
    use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.permissions().mode() & 0o077 != 0
    {
        return Err(HostError::InsecureBootstrapFile);
    }
    Ok(())
}

#[cfg(windows)]
fn validate_user_only_file(path: &Path) -> Result<(), HostError> {
    if !fs::metadata(path)?.is_file() {
        return Err(HostError::InsecureBootstrapFile);
    }
    Ok(())
}
#[cfg(test)]
mod request_journal_tests {
    use super::*;
    use yttt_core::model::ids::{ProjectId, TerminalSessionId};
    use yttt_protocol::{
        ssh::{CredentialAnswer, SensitiveBytes},
        terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalSpawnSpec},
    };

    fn response(request_id: u64) -> HostResponse {
        HostResponse {
            request_id,
            result: Ok(Response::Applied),
        }
    }

    #[test]
    fn journal_evicts_old_entries_at_its_hard_entry_limit() {
        let mut journal = RequestJournal::default();
        for request_id in 0..=MAX_REQUEST_JOURNAL_ENTRIES as u64 {
            journal.insert([request_id as u8; 32], response(request_id));
        }
        assert_eq!(journal.entries.len(), MAX_REQUEST_JOURNAL_ENTRIES);
        assert!(matches!(journal.lookup(0, &[0; 32]), JournalLookup::Miss));
        assert!(matches!(
            journal.lookup(1, &[1; 32]),
            JournalLookup::Replay(_)
        ));
        assert!(journal.encoded_bytes <= MAX_REQUEST_JOURNAL_BYTES);
    }

    #[test]
    fn secret_bearing_requests_are_never_journalable() {
        let sensitive = Request::CredentialAnswer {
            challenge_id: 1,
            answer: CredentialAnswer::Secret(SensitiveBytes::new(b"secret".to_vec())),
        };
        let ssh = Request::SshConnect(yttt_protocol::ssh::SshConnectSpec {
            connection_id: "secret".to_string(),
            endpoint: yttt_protocol::ssh::SshEndpoint {
                host: "localhost".to_string(),
                port: 22,
                username: "user".to_string(),
            },
            authentication: yttt_protocol::ssh::SshAuthentication::Password {
                secret: SensitiveBytes::new(b"secret".to_vec()),
                save_as: None,
            },
            reconnect: false,
        });
        let non_secret = Request::SpawnTerminal(TerminalSpawnSpec {
            session_id: TerminalSessionId::new("safe"),
            project_id: ProjectId::new("project"),
            cwd: yttt_protocol::ProjectRelativePath::root(),
            execution: TerminalExecutionSpec::Shell {
                program: "/bin/sh".to_string(),
                args: Vec::new(),
                initial_command: None,
            },
            geometry: TerminalGeometry {
                cols: 80,
                rows: 24,
                cell_width: 0,
                cell_height: 0,
            },
            geometry_epoch: 1,
            query_palette: Vec::new(),
            palette_revision: 1,
            scrollback_limit: 100,
            environment: Vec::new(),
            removed_environment: Vec::new(),
        });
        assert!(!request_is_journalable(&sensitive));
        assert!(!request_is_journalable(&ssh));
        assert!(request_is_journalable(&non_secret));
    }

    #[test]
    fn mutating_requests_must_pass_the_local_capability_checkpoint() {
        let mutate = Request::Project(yttt_protocol::project::ProjectRequest::SaveFile {
            project_id: ProjectId::new("notes"),
            relative_path: yttt_protocol::ProjectRelativePath::from_utf8("notes.txt").unwrap(),
            text: "x".to_string(),
            mode: yttt_protocol::project::ProjectSaveMode::Force,
        });
        assert_eq!(
            mutate.required_capability(),
            Some(yttt_protocol::Capability::ProjectMutate)
        );
        assert!(authorize_local_capability(&mutate).is_ok());
        assert!(mutate.audit_action().is_some());
    }
}

#[cfg(test)]
mod attachment_resync_tests {
    use super::*;
    use yttt_protocol::terminal::TerminalLeaseMode;

    #[test]
    fn scrolled_lag_counts_skipped_events_and_marks_resync() {
        let mut attachment = TerminalAttachment::new(TerminalLeaseMode::Observer, 1);
        attachment.display_offset = 4;
        attachment.note_lagged(12);
        assert_eq!(attachment.unseen_output, 12);
        assert_eq!(attachment.lagged_events, 12);
        assert!(attachment.pending_scroll_resync);
        assert!(attachment.take_scroll_resync());
        assert!(!attachment.pending_scroll_resync);
    }

    #[test]
    fn follow_mode_lag_does_not_inflate_unseen_output() {
        let mut attachment = TerminalAttachment::new(TerminalLeaseMode::Interactive, 1);
        attachment.note_lagged(8);
        assert_eq!(attachment.unseen_output, 0);
        assert_eq!(attachment.lagged_events, 8);
        assert!(!attachment.pending_scroll_resync);
    }
}
