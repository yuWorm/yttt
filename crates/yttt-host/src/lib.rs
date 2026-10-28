#![forbid(unsafe_code)]
mod agent_hooks;
mod agent_processes;
mod audit;
mod control;
mod device_settings;
pub mod diagnostics;
mod lifecycle;
mod project;
mod remote_access;
pub mod runtime;
mod ssh_runtime;
pub mod terminal;
mod terminal_data;
pub use terminal_data::SharedTerminalUpdate;
mod token_usage;
mod workspace;
mod workspace_backups;
mod workspace_objects;
pub use workspace::WorkspaceService;

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
    terminal_data::{TerminalStream, TerminalStreamContext},
};
use fs2::FileExt as _;
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use tokio::{
    io::AsyncReadExt as _,
    sync::{broadcast, mpsc, watch},
};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId, TerminalSessionId};
use yttt_protocol::{
    BuildIdentity, ClientRequest, ControlMessage, FailureCode, HostEvent, HostLifecycleStatus,
    HostResponse, LIFECYCLE_PROTOCOL_VERSION, LifecycleMessage, LifecycleRequest,
    LifecycleResponse, LifecycleResponseEnvelope, ProtocolFailure, ProtocolRange,
    RESOURCE_PROTOCOL_VERSION, Request, ResourceCatalog, Response, ServerEvent,
    TerminalTerminationResult,
    terminal::{
        AttachTerminal, TerminalLeaseMode, TerminalSpawnSpec, TerminalViewportAnchor,
        TerminalViewportRead, TerminatedTerminal, TerminationMode,
    },
    workspace::WorkspaceProjectConfig,
};
use yttt_transport::{
    AuthToken, HostIdentity, SessionReader, SessionWriter, TransportListener, TransportStream,
    receive_lifecycle, send_lifecycle, server_handshake,
};

const HOST_DIAGNOSTICS_INTERVAL: Duration = Duration::from_secs(5);

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
    pub state_root: PathBuf,
    pub config_root: PathBuf,
    pub project_config: WorkspaceProjectConfig,
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

/// A private SSH-forwarded work endpoint. Its credential never grants local administration.
pub struct RemoteWorkListener {
    listener: Box<dyn TransportListener>,
    token: Arc<AuthToken>,
}

impl RemoteWorkListener {
    pub fn new(listener: impl TransportListener, token: AuthToken) -> Self {
        Self {
            listener: Box::new(listener),
            token: Arc::new(token),
        }
    }
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
    run_with_remote_work(bootstrap, || async move {
        bind().await.map(|listener| (listener, None))
    })
    .await
}

/// Binds both endpoints under the same profile lock before publishing readiness.
pub async fn run_with_remote_work<L, B, Fut, E>(
    bootstrap: HostBootstrap,
    bind: B,
) -> Result<(), HostError>
where
    L: TransportListener,
    B: FnOnce() -> Fut,
    Fut: Future<Output = Result<(L, Option<RemoteWorkListener>), E>>,
    E: Into<yttt_transport::TransportError>,
{
    fs::create_dir_all(&bootstrap.runtime_root)?;
    secure_runtime_root(&bootstrap.runtime_root)?;
    let guard = HostInstanceGuard::acquire(&bootstrap)?;
    let workspaces = Arc::new(WorkspaceService::new(
        &bootstrap.state_root,
        &bootstrap.config_root,
        bootstrap.project_config.clone(),
    )?);
    let device_settings = Arc::new(Mutex::new(device_settings::DeviceSettingsStore::load(
        &bootstrap.state_root,
        &bootstrap.config_root,
    )?));
    let (listener, work_listener) = bind()
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
        environment_id: workspaces.environment_id().to_string(),
        credential_generation: 0,
        ingress: yttt_transport::IngressKind::LocalAdmin,
        host_id,
        host_epoch,
        connection_sequence: 1,
    });
    let token = Arc::new(token);
    let next_connection = Arc::new(AtomicU64::new(1));
    let next_host_sequence = Arc::new(AtomicU64::new(1));
    let request_journals = Arc::new(Mutex::new(HashMap::new()));
    let client_attachments = Arc::new(Mutex::new(HashMap::new()));
    let start_attempts = Arc::new(Mutex::new(StartAttemptRegistry::default()));
    let sessions = Arc::new(Mutex::new(HashMap::new()));
    let mutation_gate = Arc::new(tokio::sync::RwLock::new(()));
    let (profile_changes, _) = watch::channel(0_u64);
    let audit = Arc::new(HostAuditLog::new());
    let (stop_tx, mut stop_rx) = watch::channel(false);
    let runtime = HostRuntime::new();
    let lifecycle = Arc::new(HostLifecycle::new(stop_tx.clone(), bootstrap.lifetime));
    let projects = Arc::new(HostProjectRuntime::new_with_epoch(host_epoch));
    projects.sync_workspaces(workspaces.referenced_projects());
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
        let mut interval = tokio::time::interval(HOST_DIAGNOSTICS_INTERVAL);
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
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
    let agent_hooks = HostAgentHookRuntime::start_in(host_epoch, &bootstrap.state_root)?;
    tokio::spawn(agent_processes::run(
        runtime.clone(),
        agent_hooks.clone(),
        stop_rx.clone(),
    ));
    let mut agent_terminal_events = runtime.subscribe();
    let terminal_agent_hooks = agent_hooks.clone();
    let terminal_runtime = runtime.clone();
    tokio::spawn(async move {
        loop {
            match agent_terminal_events.recv().await {
                Ok(HostTerminalEvent::Exited {
                    session_id,
                    session_epoch,
                    code,
                    ..
                }) => terminal_agent_hooks.terminal_exited(&session_id, session_epoch, code),
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    for placement in terminal_runtime.placements() {
                        if let yttt_protocol::terminal::TerminalProcessState::Exited { code } =
                            placement.process_state
                        {
                            terminal_agent_hooks.terminal_exited(
                                &placement.session_id,
                                placement.session_epoch,
                                code,
                            );
                        }
                    }
                }
                Err(broadcast::error::RecvError::Closed) => break,
                Ok(_) => {}
            }
        }
    });
    let (remote_access, remote_commands) = remote_access::RemoteAccessHandle::channel();
    let connection_context = ConnectionContext {
        identity: (*identity).clone(),
        profile_id: bootstrap.profile_id.clone(),
        host_sequence: next_host_sequence.clone(),
        ssh: ssh.clone(),
        agent_hooks: agent_hooks.clone(),
        projects: projects.clone(),
        workspaces: workspaces.clone(),
        device_settings: device_settings.clone(),
        remote_access,
        network_admission: None,
        handshake_permit: None,
        handshake_deadline: None,
        mutation_gate: mutation_gate.clone(),
        request_journals: request_journals.clone(),
        start_attempts: start_attempts.clone(),
        client_attachments: client_attachments.clone(),
        sessions: sessions.clone(),
        profile_changes: profile_changes.clone(),
        runtime: runtime.clone(),
        lifecycle: lifecycle.clone(),
        audit: audit.clone(),
        stop: stop_rx.clone(),
    };
    let remote_task = tokio::spawn(remote_access::run(
        connection_context.clone(),
        remote_commands,
        next_connection.clone(),
    ));
    let work_task = work_listener.map(|work| {
        tokio::spawn(serve_remote_work(
            work,
            connection_context.clone(),
            next_connection.clone(),
        ))
    });
    guard.publish_ready(&ready)?;

    let mut exit_reaper = tokio::time::interval(Duration::from_secs(60));
    exit_reaper.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut profile_deadline_changes = profile_changes.subscribe();
    loop {
        tokio::select! {
            changed = stop_rx.changed() => {
                if changed.is_err() || *stop_rx.borrow() {
                    break;
                }
            }
            changed = profile_deadline_changes.changed() => {
                if changed.is_err() {
                    break;
                }
            }
            _ = wait_for_preparation_deadline(
                workspaces.control.preparation_deadline_millis()
            ) => {
                if workspaces.control.expire_preparation() {
                    profile_changes.send_modify(|revision| *revision = revision.wrapping_add(1));
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
                prune_inactive_sessions(&connection_context);
            }
            accepted = listener.accept() => {
                let stream = accepted?;
                let mut context = connection_context.clone();
                context.identity.connection_sequence = next_connection.fetch_add(1, Ordering::Relaxed);
                let token = token.clone();
                tokio::spawn(async move {
                    let _ = serve_connection(stream, token, context).await;
                });
            }
        }
    }

    drop(listener);
    let _ = remote_task.await;
    if let Some(task) = work_task {
        let _ = task.await;
    }
    let _ = diagnostics_task.await;
    drop(guard);
    Ok(())
}

async fn serve_remote_work(
    work: RemoteWorkListener,
    mut context: ConnectionContext,
    sequence: Arc<AtomicU64>,
) {
    context.identity.ingress = yttt_transport::IngressKind::SshWork;
    let mut stop = context.stop.clone();
    let mut tasks = tokio::task::JoinSet::new();
    loop {
        tokio::select! {
            changed = stop.changed() => { if changed.is_err() || *stop.borrow() { break; } }
            _ = tasks.join_next(), if !tasks.is_empty() => {}
            accepted = work.listener.accept(), if tasks.len() < 256 => {
                let Ok(stream) = accepted else { break };
                let mut context = context.clone();
                context.identity.connection_sequence = sequence.fetch_add(1, Ordering::Relaxed);
                let token = work.token.clone();
                tasks.spawn(async move { let _ = serve_connection(stream, token, context).await; });
            }
        }
    }
    drop(work);
    while tasks.join_next().await.is_some() {}
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

type SharedTerminalAttachments =
    Arc<tokio::sync::Mutex<HashMap<TerminalSessionId, TerminalAttachment>>>;

#[derive(Clone)]
struct ConnectionContext {
    identity: HostIdentity,
    profile_id: ProfileId,
    host_sequence: Arc<AtomicU64>,
    ssh: Arc<HostSshRuntime>,
    agent_hooks: Arc<HostAgentHookRuntime>,
    projects: Arc<HostProjectRuntime>,
    workspaces: Arc<WorkspaceService>,
    device_settings: Arc<Mutex<device_settings::DeviceSettingsStore>>,
    remote_access: remote_access::RemoteAccessHandle,
    network_admission: Option<Arc<remote_access::NetworkAdmission>>,
    handshake_permit: Option<Arc<Mutex<Option<tokio::sync::OwnedSemaphorePermit>>>>,
    handshake_deadline: Option<tokio::time::Instant>,
    mutation_gate: Arc<tokio::sync::RwLock<()>>,
    request_journals:
        Arc<Mutex<HashMap<ClientInstanceId, Arc<tokio::sync::Mutex<RequestJournal>>>>>,
    start_attempts: Arc<Mutex<StartAttemptRegistry>>,
    client_attachments: Arc<Mutex<HashMap<ClientInstanceId, SharedTerminalAttachments>>>,
    sessions: Arc<Mutex<HashMap<ClientInstanceId, SessionBinding>>>,
    profile_changes: watch::Sender<u64>,
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

// Records retain only bounded start IDs and fixed-size address digests. The terminal
// runtime owns the potentially large original spawn specification while it is live.
const MAX_REQUEST_JOURNAL_ENTRIES: usize = 256;
const MAX_START_ATTEMPTS: usize = 4_096;
// Bounds retained start-ID bytes to at most one MiB across the registry.
const MAX_START_ID_BYTES: usize = 256;

#[derive(Default)]
struct StartAttemptRegistry {
    records: HashMap<String, StartAttemptRecord>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct StartAttemptIdentity([u8; 32]);

impl StartAttemptIdentity {
    // This is computed before secure_terminal_environment mutates the spawn spec.
    fn from_spec(spec: &TerminalSpawnSpec) -> Self {
        let mut writer = DigestWriter(Sha256::new());
        serde_json::to_writer(
            &mut writer,
            &(
                &spec.session_id,
                &spec.project_id,
                &spec.cwd,
                &spec.execution,
            ),
        )
        .expect("terminal start address must serialize");
        Self(writer.0.finalize().into())
    }
}

struct StartAttemptRecord {
    host_epoch: u64,
    identity: StartAttemptIdentity,
    outcome: StartAttemptOutcome,
}

#[derive(Clone, Copy)]
enum StartAttemptOutcome {
    Starting,
    Spawned { session_epoch: u64 },
    Failed(FailureCode),
}
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
    !recovery_request(request)
        && !matches!(
            request,
            Request::SshConnect(_)
                | Request::CredentialAnswer { .. }
                | Request::ProfileControl(_)
                | Request::RemoteAccess(_)
                | Request::TerminateMany { .. }
        )
}

fn authorize_ingress(
    request: &Request,
    ingress: yttt_transport::IngressKind,
) -> Result<(), ProtocolFailure> {
    if ingress != yttt_transport::IngressKind::LocalAdmin
        && matches!(
            request,
            Request::DeleteSshCredential { .. }
                | Request::ReadDeviceSettings
                | Request::SetLoginStartupConsent { .. }
                | Request::RemoteAccess(_)
                | Request::ProfileControl(
                    yttt_protocol::session::ProfileControlRequest::ReacquireIfUnowned { .. }
                )
        )
    {
        return Err(ProtocolFailure::new(
            FailureCode::PermissionDenied,
            "device administration requires local IPC",
            false,
        ));
    }
    Ok(())
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

const MAX_CLIENT_SESSIONS: usize = 64;
const MAX_IDLE_CLIENT_SESSIONS: usize = 16;
const CLIENT_SESSION_RETENTION: Duration = Duration::from_secs(300);

struct SessionBinding {
    authentication: yttt_transport::AuthenticatedSession,
    /// Closes the current Session connection when a newer one replaces it.
    connection: Option<watch::Sender<bool>>,
    generation: u64,
    /// Held by a connection until it has released the Client's resources.
    slot: Arc<tokio::sync::Mutex<()>>,
    last_disconnected: std::time::Instant,
}

fn expired_sessions(
    sessions: &HashMap<ClientInstanceId, SessionBinding>,
    now: std::time::Instant,
) -> Vec<ClientInstanceId> {
    let mut idle = sessions
        .iter()
        .filter(|(_, binding)| binding.connection.is_none())
        .map(|(id, binding)| (id.clone(), binding.last_disconnected))
        .collect::<Vec<_>>();
    idle.sort_by_key(|(_, disconnected)| *disconnected);
    let excess = idle.len().saturating_sub(MAX_IDLE_CLIENT_SESSIONS);
    idle.into_iter()
        .enumerate()
        .filter_map(|(index, (id, disconnected))| {
            (index < excess
                || now.saturating_duration_since(disconnected) >= CLIENT_SESSION_RETENTION)
                .then_some(id)
        })
        .collect()
}

fn prune_inactive_sessions(context: &ConnectionContext) {
    let mut sessions = context.sessions.lock();
    let expired = expired_sessions(&sessions, std::time::Instant::now());
    if expired.is_empty() {
        return;
    }
    let mut journals = context.request_journals.lock();
    let mut attachments = context.client_attachments.lock();
    for id in expired {
        sessions.remove(&id);
        journals.remove(&id);
        attachments.remove(&id);
    }
}

struct SessionConnectionGuard {
    sessions: Arc<Mutex<HashMap<ClientInstanceId, SessionBinding>>>,
    client_id: ClientInstanceId,
    generation: u64,
}

impl Drop for SessionConnectionGuard {
    fn drop(&mut self) {
        if let Some(binding) = self.sessions.lock().get_mut(&self.client_id)
            && binding.generation == self.generation
        {
            binding.connection = None;
            binding.last_disconnected = std::time::Instant::now();
        }
    }
}

async fn serve_connection(
    mut stream: TransportStream,
    token: Arc<AuthToken>,
    mut context: ConnectionContext,
) -> Result<(), ()> {
    let mut handshake_stop = context.stop.clone();
    if *handshake_stop.borrow() {
        return Err(());
    }
    let deadline = context
        .handshake_deadline
        .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(5));
    let authenticated = tokio::select! {
        _ = handshake_stop.changed() => return Err(()),
        result = tokio::time::timeout_at(deadline, server_handshake(&mut stream, &context.identity, token.as_ref())) =>
            result.map_err(|_| ())?.map_err(|_| ())?,
    };
    if let Some(permit) = context.handshake_permit.take() {
        permit.lock().take();
    }
    let _network_session = if let Some(admission) = &context.network_admission {
        let session = admission.admit(&authenticated.client_instance_id)?;
        context.stop = session.stop.clone();
        Some(session)
    } else {
        None
    };
    prune_inactive_sessions(&context);
    match authenticated.channel {
        yttt_protocol::ConnectionChannel::DesktopOwner => {
            if !context.lifecycle.desktop_owner_connected() {
                return Err(());
            }
            let result = serve_desktop_owner_connection(stream, context.stop.clone()).await;
            context.lifecycle.desktop_owner_disconnected();
            context.lifecycle.resource_changed();
            result
        }
        yttt_protocol::ConnectionChannel::Lifecycle => {
            context.lifecycle.client_connected();
            let result =
                serve_lifecycle_connection(stream, authenticated.can_force_stop, &context).await;
            context.lifecycle.client_disconnected();
            result
        }
        yttt_protocol::ConnectionChannel::Session => {
            serve_client_session(stream, authenticated, &context).await
        }
    }
}

/// Binds the Session connection to its Client session, replacing an earlier
/// connection of the same session, and releases the Client's resources when
/// the connection ends.
async fn serve_client_session(
    stream: TransportStream,
    authenticated: yttt_transport::AuthenticatedClient,
    context: &ConnectionContext,
) -> Result<(), ()> {
    let client_id = authenticated.client_instance_id;
    let (replace, replaced) = watch::channel(false);
    let (slot, generation) = {
        let mut sessions = context.sessions.lock();
        if let Some(binding) = sessions.get(&client_id) {
            if binding.authentication != authenticated.session {
                return Err(());
            }
        } else {
            if sessions.len() >= MAX_CLIENT_SESSIONS {
                return Err(());
            }
            sessions.insert(
                client_id.clone(),
                SessionBinding {
                    authentication: authenticated.session.clone(),
                    connection: None,
                    generation: 0,
                    slot: Arc::new(tokio::sync::Mutex::new(())),
                    last_disconnected: std::time::Instant::now(),
                },
            );
        }
        let binding = sessions.get_mut(&client_id).expect("registered session");
        if let Some(previous) = binding.connection.replace(replace) {
            previous.send_replace(true);
        }
        binding.generation += 1;
        (binding.slot.clone(), binding.generation)
    };
    let _connection = SessionConnectionGuard {
        sessions: context.sessions.clone(),
        client_id: client_id.clone(),
        generation,
    };
    // The replaced connection finishes releasing the Client's resources first,
    // so it cannot release what this connection registers.
    let _slot = slot.lock_owned().await;
    if *replaced.borrow() {
        return Ok(());
    }
    context.lifecycle.client_connected();
    let attachments = shared_terminal_attachments(context, &client_id);
    let request_journal = {
        let mut journals = context.request_journals.lock();
        journals
            .entry(client_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(RequestJournal::default())))
            .clone()
    };
    let result = serve_session_connection(
        stream,
        client_id.clone(),
        attachments.clone(),
        request_journal,
        replaced,
        context,
    )
    .await;
    {
        let _fence = context.mutation_gate.write().await;
        context.workspaces.control.disconnect(&client_id);
        context.runtime.release_client(&client_id);
        context.projects.disconnect(&client_id);
        context
            .profile_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    context.ssh.abandon_challenges(&client_id);
    attachments.lock().await.clear();
    {
        let mut all_attachments = context.client_attachments.lock();
        if all_attachments
            .get(&client_id)
            .is_some_and(|current| Arc::ptr_eq(current, &attachments))
        {
            all_attachments.remove(&client_id);
        }
    }
    context.lifecycle.client_disconnected();
    context.lifecycle.resource_changed();
    result
}

fn shared_terminal_attachments(
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
) -> SharedTerminalAttachments {
    context
        .client_attachments
        .lock()
        .entry(client_id.clone())
        .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(HashMap::new())))
        .clone()
}

fn request_uses_terminal_attachments(request: &Request) -> bool {
    matches!(
        request,
        Request::SpawnTerminal { .. }
            | Request::AttachTerminal(_)
            | Request::DetachTerminal { .. }
            | Request::AcquireTerminalLease { .. }
            | Request::ReleaseTerminalLease { .. }
            | Request::TerminalInput(_)
            | Request::ResizeTerminal(_)
            | Request::ScrollTerminal(_)
            | Request::ReadTerminalViewport(_)
            | Request::SearchTerminal(_)
            | Request::SetTerminalQueryPalette(_)
            | Request::RequestCheckpoint { .. }
            | Request::AcknowledgeTerminalExit { .. }
            | Request::TerminateTerminal { .. }
            | Request::TerminateMany { .. }
            | Request::RequestTerminalControl { .. }
            | Request::ReleaseTerminalControl { .. }
    )
}

fn interactive_lane_request(request: &Request) -> bool {
    matches!(
        request,
        Request::TerminalInput(_) | Request::ResizeTerminal(_) | Request::ScrollTerminal(_)
    )
}
fn request_changes_resource_catalog(request: &Request) -> bool {
    match request {
        Request::SpawnTerminal { .. }
        | Request::AttachTerminal(_)
        | Request::DetachTerminal { .. }
        | Request::AcquireTerminalLease { .. }
        | Request::ReleaseTerminalLease { .. }
        | Request::AcknowledgeTerminalExit { .. }
        | Request::TerminateTerminal { .. }
        | Request::TerminateMany { .. }
        | Request::SshConnect(_)
        | Request::SshDisconnect { .. }
        | Request::RequestTerminalControl { .. }
        | Request::ReleaseTerminalControl { .. } => true,
        Request::Project(request) => matches!(
            request,
            yttt_protocol::project::ProjectRequest::Register { .. }
                | yttt_protocol::project::ProjectRequest::RegisterSsh { .. }
                | yttt_protocol::project::ProjectRequest::Close { .. }
        ),
        _ => false,
    }
}

fn recovery_request(request: &Request) -> bool {
    matches!(
        request,
        Request::Ping { .. }
            | Request::ListResources
            | Request::RequestCheckpoint { .. }
            | Request::ReadAgentSnapshots { .. }
            | Request::SetTerminalVisibility { .. }
    )
}

/// Aborts a connection's helper task when the connection ends.
struct AbortOnDrop(tokio::task::JoinHandle<()>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// How long a stopping Host waits for a connection to write `HostStopping`.
const STOPPING_FLUSH_TIMEOUT: Duration = Duration::from_secs(1);

/// Serves one authenticated Client over its single Session connection:
/// requests and responses, terminal input, state events, and terminal frames.
async fn serve_session_connection(
    stream: TransportStream,
    client_id: ClientInstanceId,
    attachments: SharedTerminalAttachments,
    request_journal: Arc<tokio::sync::Mutex<RequestJournal>>,
    mut replaced: watch::Receiver<bool>,
    context: &ConnectionContext,
) -> Result<(), ()> {
    const MAX_QUEUED_REQUESTS: usize = 64;
    let (reader, writer) = tokio::io::split(stream);
    let mut reader = SessionReader::new(reader);
    let (writer, _writer_task) = SessionWriter::spawn(writer);
    let mut streams = TerminalStreams {
        context: TerminalStreamContext {
            writer: writer.clone(),
            runtime: context.runtime.clone(),
            host_sequence: context.host_sequence.clone(),
            client_id: client_id.clone(),
        },
        streams: HashMap::new(),
    };
    // Each opened workspace keeps at most one unsent layout on this connection,
    // so a slow Client skips straight to the latest one.
    let _layouts = context.workspaces.subscribe_layouts(client_id.clone(), {
        let writer = writer.clone();
        let host_sequence = context.host_sequence.clone();
        Box::new(move |update| {
            let message = ControlMessage::Event(HostEvent {
                host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                body: ServerEvent::WorkspaceChanged(Box::new(update.clone())),
            });
            let key = format!("workspace-layout:{}", update.workspace_id.as_str());
            writer.send_reliable_latest(&key, &message).is_ok()
        })
    });
    let _state_events = AbortOnDrop(tokio::spawn({
        let events = forward_state_events(
            writer.clone(),
            client_id.clone(),
            attachments.clone(),
            context.clone(),
        );
        let writer = writer.clone();
        async move {
            if events.await.is_err() {
                writer.close();
            }
        }
    }));
    let (interactive, interactive_requests) = mpsc::channel(MAX_QUEUED_INTERACTIVE_REQUESTS);
    let _interactive_worker = AbortOnDrop(tokio::spawn(run_interactive_requests(
        interactive_requests,
        writer.clone(),
        client_id.clone(),
        attachments.clone(),
        context.clone(),
    )));
    let mut stop = context.stop.clone();
    let mut queued = VecDeque::<ClientRequest>::new();
    let mut running = tokio::task::JoinSet::new();
    loop {
        let incoming = reader.receive();
        tokio::pin!(incoming);
        loop {
            if running.is_empty()
                && let Some(request) = queued.pop_front()
            {
                let context = context.clone();
                let client_id = client_id.clone();
                let attachments = attachments.clone();
                let journal = request_journal.clone();
                running.spawn(async move {
                    let session_id = attached_session(&request.body).cloned();
                    let uses_attachments = request_uses_terminal_attachments(&request.body);
                    let (response, changed) = dispatch_control_request(
                        request,
                        &client_id,
                        &attachments,
                        &journal,
                        &context,
                    )
                    .await;
                    let attached = session_id.filter(|_| response.result.is_ok());
                    (response, changed, uses_attachments, attached)
                });
            }
            tokio::select! {
                message = &mut incoming, if queued.len() < MAX_QUEUED_REQUESTS => {
                    let message = match message {
                        Ok(message) => message,
                        Err(error) => {
                            let request_id = error.unsupported_request_id().ok_or(())?;
                            writer
                                .send_reliable(&ControlMessage::Response(HostResponse::unsupported(request_id)))
                                .map_err(|_| ())?;
                            break;
                        }
                    };
                    match message {
                        ControlMessage::TerminalInput { input, control } => {
                            let request = ClientRequest {
                                request_id: 0,
                                actor_device_id: Some(client_id.to_string()),
                                lease_epoch: None,
                                control,
                                body: Request::TerminalInput(input),
                            };
                            interactive
                                .try_send(InteractiveRequest::Input(request))
                                .map_err(|_| ())?;
                        }
                        ControlMessage::Request(request) => {
                            if let Request::Ping { sent_millis } = request.body {
                                writer
                                    .send_interactive(&ControlMessage::Response(HostResponse {
                                        request_id: request.request_id,
                                        result: Ok(Response::Pong {
                                            sent_millis,
                                            host_millis: now_millis(),
                                        }),
                                    }))
                                    .map_err(|_| ())?;
                            } else if interactive_lane_request(&request.body) {
                                interactive
                                    .try_send(InteractiveRequest::Request(request))
                                    .map_err(|_| ())?;
                            } else if recovery_request(&request.body) {
                                let uses_attachments =
                                    request_uses_terminal_attachments(&request.body);
                                let (response, changed) = dispatch_control_request(
                                    request,
                                    &client_id,
                                    &attachments,
                                    &request_journal,
                                    context,
                                )
                                .await;
                                writer
                                    .send_reliable(&ControlMessage::Response(response))
                                    .map_err(|_| ())?;
                                if uses_attachments {
                                    streams.reconcile(&attachments, None).await;
                                }
                                if changed {
                                    context.lifecycle.resource_changed();
                                }
                            } else {
                                queued.push_back(request);
                            }
                        }
                        ControlMessage::Response(_)
                        | ControlMessage::Event(_)
                        | ControlMessage::Chunk(_) => return Err(()),
                    }
                    break;
                }
                finished = running.join_next(), if !running.is_empty() => {
                    let (response, changed, uses_attachments, attached) =
                        finished.ok_or(())?.map_err(|_| ())?;
                    writer
                        .send_reliable(&ControlMessage::Response(response))
                        .map_err(|_| ())?;
                    // Streams start after the response that attached them is queued.
                    if uses_attachments {
                        streams.reconcile(&attachments, attached.as_ref()).await;
                    }
                    if changed {
                        context.lifecycle.resource_changed();
                    }
                }
                () = writer.closed() => return Err(()),
                changed = replaced.changed() => {
                    if changed.is_err() || *replaced.borrow() {
                        return Ok(());
                    }
                }
                changed = stop.changed() => {
                    if changed.is_err() || *stop.borrow() {
                        let _ = send_host_state_event(&writer, context, ServerEvent::HostStopping);
                        let _ = tokio::time::timeout(STOPPING_FLUSH_TIMEOUT, writer.flush()).await;
                        return Ok(());
                    }
                }
            }
        }
    }
}

/// The terminal a successful request attaches anew; its stream restarts with a Snapshot.
fn attached_session(request: &Request) -> Option<&TerminalSessionId> {
    match request {
        Request::SpawnTerminal { spec, .. } => Some(&spec.session_id),
        Request::AttachTerminal(attach) => Some(&attach.session_id),
        _ => None,
    }
}

/// The terminal streams of one Session connection: one per terminal attachment.
struct TerminalStreams {
    context: TerminalStreamContext,
    streams: HashMap<TerminalSessionId, TerminalStream>,
}

impl TerminalStreams {
    /// Matches the streams to the connection's attachments. A stream that
    /// finished, for example after its terminal exited, is not restarted
    /// unless `attached` names it.
    async fn reconcile(
        &mut self,
        attachments: &SharedTerminalAttachments,
        attached: Option<&TerminalSessionId>,
    ) {
        let attachments = attachments.lock().await;
        self.streams
            .retain(|session_id, _| attachments.contains_key(session_id));
        if let Some(session_id) = attached.filter(|id| attachments.contains_key(*id)) {
            self.streams.insert(
                session_id.clone(),
                TerminalStream::start(&self.context, session_id.clone()),
            );
        }
        for session_id in attachments.keys() {
            if !self.streams.contains_key(session_id) {
                self.streams.insert(
                    session_id.clone(),
                    TerminalStream::start(&self.context, session_id.clone()),
                );
            }
        }
    }
}

async fn dispatch_control_request(
    request: ClientRequest,
    client_id: &ClientInstanceId,
    attachments: &SharedTerminalAttachments,
    request_journal: &Arc<tokio::sync::Mutex<RequestJournal>>,
    context: &ConnectionContext,
) -> (HostResponse, bool) {
    let resource_changed = request_changes_resource_catalog(&request.body);
    let response = if request_uses_terminal_attachments(&request.body) {
        let mut attachments = attachments.lock().await;
        process_control_request(
            request,
            context,
            client_id,
            &mut attachments,
            request_journal,
        )
        .await
    } else {
        process_control_request(
            request,
            context,
            client_id,
            &mut HashMap::new(),
            request_journal,
        )
        .await
    };
    let changed = resource_changed && response.result.is_ok();
    (response, changed)
}

async fn process_control_request(
    request: ClientRequest,
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
    subscriptions: &mut HashMap<TerminalSessionId, TerminalAttachment>,
    request_journal: &Arc<tokio::sync::Mutex<RequestJournal>>,
) -> HostResponse {
    let mutation_guard = match admit_mutation(&request, context, client_id).await {
        Ok(guard) => guard,
        Err(failure) => {
            return HostResponse {
                request_id: request.request_id,
                result: Err(failure),
            };
        }
    };
    if let Err(failure) =
        authorize_terminal_termination(&request.body, context, client_id, subscriptions)
    {
        return HostResponse {
            request_id: request.request_id,
            result: Err(failure),
        };
    }
    let subscription = match &request.body {
        Request::SpawnTerminal { spec, .. } => Some((
            spec.session_id.clone(),
            TerminalLeaseMode::Interactive,
            true,
        )),
        Request::AttachTerminal(attach) => Some((attach.session_id.clone(), attach.mode, true)),
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
        Request::RequestCheckpoint { session_id, .. } => {
            Some((session_id.clone(), TerminalLeaseMode::Observer, false))
        }
        _ => None,
    };
    let unsubscription = match &request.body {
        Request::DetachTerminal { session_id }
        | Request::TerminateTerminal { session_id, .. }
        | Request::AcknowledgeTerminalExit { session_id, .. } => Some(session_id.clone()),
        _ => None,
    };
    let (response, apply_effects) = if request_is_journalable(&request.body) {
        let fingerprint = request_fingerprint(&request.body);
        let mut journal = request_journal.lock().await;
        match journal.lookup(request.request_id, &fingerprint) {
            JournalLookup::Replay(response) => match &request.body {
                Request::SpawnTerminal {
                    spec,
                    start_id,
                    expected_host_epoch,
                } => (
                    HostResponse {
                        request_id: request.request_id,
                        result: spawn_terminal(
                            spec.clone(),
                            start_id.clone(),
                            *expected_host_epoch,
                            context,
                            client_id,
                            false,
                        ),
                    },
                    true,
                ),
                _ => (*response, false),
            },
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
                    context,
                    client_id,
                    subscriptions,
                    context.host_sequence.fetch_add(1, Ordering::Relaxed),
                    mutation_guard.blocking.clone(),
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
                context,
                client_id,
                subscriptions,
                context.host_sequence.fetch_add(1, Ordering::Relaxed),
                mutation_guard.blocking.clone(),
            )
            .await,
            true,
        )
    };
    if apply_effects && let Ok(result) = &response.result {
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
            context.runtime.register_attachment(&session_id, client_id);
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
        apply_terminal_response_effects(result, subscriptions, client_id, &context.runtime);
        if let Response::TerminalsTerminated { results } = result {
            for result in results.iter().filter(|result| result.result.is_ok()) {
                subscriptions.remove(&result.session_id);
                context
                    .runtime
                    .release_attachment(&result.session_id, client_id);
            }
        }
        if let Some(session_id) = unsubscription {
            subscriptions.remove(&session_id);
            context.runtime.release_attachment(&session_id, client_id);
        }
    }
    response
}

fn apply_terminal_response_effects(
    result: &Response,
    subscriptions: &mut HashMap<TerminalSessionId, TerminalAttachment>,
    client_id: &ClientInstanceId,
    runtime: &HostRuntime,
) {
    match result {
        Response::TerminalScrolled(read) | Response::TerminalViewport(read) => {
            if let Some(attachment) = subscriptions.get_mut(&read.viewport.session_id) {
                attachment.display_offset = read.viewport.display_offset;
                runtime.update_attachment_display_offset(
                    &read.viewport.session_id,
                    client_id,
                    read.viewport.display_offset,
                );
                if attachment.display_offset == 0 {
                    attachment.unseen_output = 0;
                    if attachment.take_scroll_resync() {
                        runtime.mark_attachment_resync(
                            &read.viewport.session_id,
                            client_id,
                            AttachmentResyncReason::ReturnToBottom,
                        );
                    }
                }
            }
        }
        Response::TerminalSearch(results) => {
            if let Some(attachment) = subscriptions.get_mut(&results.session_id) {
                attachment.search_generation = results.generation;
            }
        }
        _ => {}
    }
}

/// A terminal mutation read from the Session connection, executed in arrival
/// order by the connection's interactive worker.
enum InteractiveRequest {
    /// One-way input; a rejection is only logged.
    Input(ClientRequest),
    /// Input, resize, or scroll that expects a response.
    Request(ClientRequest),
}

/// Requests a connection may queue for its interactive worker before it is
/// closed as unresponsive.
const MAX_QUEUED_INTERACTIVE_REQUESTS: usize = 1024;

async fn run_interactive_requests(
    mut requests: mpsc::Receiver<InteractiveRequest>,
    writer: SessionWriter,
    client_id: ClientInstanceId,
    attachments: SharedTerminalAttachments,
    context: ConnectionContext,
) {
    while let Some(request) = requests.recv().await {
        let (request, reply) = match request {
            InteractiveRequest::Input(request) => (request, false),
            InteractiveRequest::Request(request) => (request, true),
        };
        let mut attachments = attachments.lock().await;
        let admission = admit_mutation(&request, &context, &client_id).await;
        let response = match &admission {
            Ok(guard) => {
                handle_request(
                    request,
                    &context,
                    &client_id,
                    &mut attachments,
                    context.host_sequence.fetch_add(1, Ordering::Relaxed),
                    guard.blocking.clone(),
                )
                .await
            }
            Err(failure) => HostResponse {
                request_id: request.request_id,
                result: Err(failure.clone()),
            },
        };
        if let Ok(result) = &response.result {
            apply_terminal_response_effects(result, &mut attachments, &client_id, &context.runtime);
        }
        drop(attachments);
        if reply {
            if writer
                .send_interactive(&ControlMessage::Response(response))
                .is_err()
            {
                return;
            }
        } else if let Err(error) = response.result {
            eprintln!("one-way terminal input rejected: {error:?}");
        }
    }
}

fn send_host_state_event(
    writer: &SessionWriter,
    context: &ConnectionContext,
    body: ServerEvent,
) -> Result<(), ()> {
    writer
        .send_reliable(&ControlMessage::Event(HostEvent {
            host_sequence: context.host_sequence.fetch_add(1, Ordering::Relaxed),
            body,
        }))
        .map_err(|_| ())
}

/// Forwards Host state changes to one Session connection as reliable events.
/// Terminal exits travel with their terminal's stream instead.
async fn forward_state_events(
    writer: SessionWriter,
    client_id: ClientInstanceId,
    attachments: SharedTerminalAttachments,
    context: ConnectionContext,
) -> Result<(), ()> {
    let context = &context;
    let mut ssh_events = context.ssh.subscribe();
    let mut agent_hook_events = context.agent_hooks.subscribe();
    let mut token_usage_events = context.agent_hooks.subscribe_token_usage();
    let mut project_events = context.projects.subscribe();
    let mut runtime_events = context.runtime.subscribe();
    let mut resource_changes = context.lifecycle.subscribe_resource_changes();
    let mut profile_changes = context.profile_changes.subscribe();
    send_host_state_event(
        &writer,
        context,
        ServerEvent::ProfileControl(
            context
                .workspaces
                .control
                .status(context.identity.host_epoch),
        ),
    )?;
    for update in context.agent_hooks.snapshots_after(&[]) {
        send_host_state_event(
            &writer,
            context,
            ServerEvent::AgentSnapshot(Box::new(update)),
        )?;
    }
    for summary in context.agent_hooks.project_token_usage() {
        send_host_state_event(&writer, context, ServerEvent::ProjectTokenUsage(summary))?;
    }
    for change in context.projects.resync_changes() {
        send_host_state_event(&writer, context, ServerEvent::ProjectChanged(change))?;
    }
    loop {
        tokio::select! {
            changed = profile_changes.changed() => {
                changed.map_err(|_| ())?;
                send_host_state_event(&writer, context,
                    ServerEvent::ProfileControl(context.workspaces.control.status(context.identity.host_epoch)))?;
            }
            event = runtime_events.recv() => {
                let server_event = {
                    let mut subscriptions = attachments.lock().await;
                    match event {
                        Ok(HostTerminalEvent::Update { session_id, .. }) => {
                            if let Some(attachment) = subscriptions.get_mut(&session_id)
                                && attachment.display_offset != 0
                            {
                                attachment.unseen_output =
                                    attachment.unseen_output.saturating_add(1);
                            }
                            None
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
                        }) if holder == client_id => {
                            Some(ServerEvent::TerminalControlRequested {
                                session_id,
                                requester,
                            })
                        }
                        Ok(HostTerminalEvent::ControlGranted { lease, .. })
                            if lease.owner == client_id =>
                        {
                            if let Some(attachment) =
                                subscriptions.get_mut(&lease.session_id)
                            {
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
                        }) if requester == client_id => {
                            Some(ServerEvent::TerminalControlDenied {
                                session_id,
                                requester,
                                reason,
                            })
                        }
                        Ok(HostTerminalEvent::TitleChanged { .. } | HostTerminalEvent::Bell { .. }) => {
                            None
                        }
                        Ok(_) => None,
                        Err(broadcast::error::RecvError::Lagged(skipped)) => {
                            for (session_id, attachment) in subscriptions.iter_mut() {
                                let scrolled = attachment.display_offset != 0;
                                attachment.note_lagged(skipped);
                                context.runtime.note_attachment_lag(
                                    session_id,
                                    &client_id,
                                    skipped,
                                    scrolled,
                                );
                            }
                            None
                        }
                        Err(broadcast::error::RecvError::Closed) => return Err(()),
                    }
                };
                if let Some(body) = server_event {
                    send_host_state_event(&writer, context, body)?;
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
                send_host_state_event(&writer, context, body)?;
            }
            event = project_events.recv() => {
                match event {
                    Ok(change) => {
                        send_host_state_event(
                            &writer,
                            context,
                            ServerEvent::ProjectChanged(change),
                        )?;
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        for change in context.projects.resync_changes() {
                            send_host_state_event(
                                &writer,
                                context,
                                ServerEvent::ProjectChanged(change),
                            )?;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                }
            }
            event = agent_hook_events.recv() => {
                let updates = match event {
                    Ok(update) => vec![update],
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        context.agent_hooks.snapshots_after(&[])
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                };
                for update in updates {
                    send_host_state_event(
                        &writer,
                        context,
                        ServerEvent::AgentSnapshot(Box::new(update)),
                    )?;
                }
            }
            event = token_usage_events.recv() => {
                let summaries = match event {
                    Ok(summary) => vec![summary],
                    Err(broadcast::error::RecvError::Lagged(_)) => {
                        context.agent_hooks.project_token_usage()
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                };
                for summary in summaries {
                    send_host_state_event(
                        &writer,
                        context,
                        ServerEvent::ProjectTokenUsage(summary),
                    )?;
                }
            }
            changed = resource_changes.changed() => {
                if changed.is_err() {
                    return Err(());
                }
                send_host_state_event(
                    &writer,
                    context,
                    ServerEvent::ResourceCatalogChanged,
                )?;
            }
        }
    }
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

fn request_mutates(body: &Request) -> bool {
    matches!(
        body.required_capability(),
        Some(
            yttt_protocol::Capability::WorkspaceMutate
                | yttt_protocol::Capability::ProjectMutate
                | yttt_protocol::Capability::GitMutate
                | yttt_protocol::Capability::TerminalInteractive
                | yttt_protocol::Capability::SshConnect
                | yttt_protocol::Capability::RemoteCommandPrivileged
        )
    )
}

#[derive(Default)]
struct MutationAdmission {
    _direct: Option<tokio::sync::OwnedRwLockReadGuard<()>>,
    blocking: Option<Arc<tokio::sync::OwnedRwLockReadGuard<()>>>,
}

async fn admit_mutation(
    request: &ClientRequest,
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
) -> Result<MutationAdmission, ProtocolFailure> {
    authorize_ingress(&request.body, context.identity.ingress)?;
    if !request_mutates(&request.body) {
        return Ok(MutationAdmission::default());
    }
    let guard = context.mutation_gate.clone().read_owned().await;
    // Layout intents serialize in the workspace store; any Client may apply them.
    if !matches!(
        &request.body,
        Request::Workspace(yttt_protocol::workspace::WorkspaceRequest::Apply { .. })
    ) {
        let control = request.control.ok_or_else(|| {
            ProtocolFailure::new(
                FailureCode::PermissionDenied,
                "mutation has no authenticated profile control context",
                false,
            )
        })?;
        context
            .workspaces
            .control
            .authorize(client_id, control, context.identity.host_epoch)?;
    }
    // Blocking I/O retains a permit even if its awaiting connection disappears.
    // Interactive input uses the direct guard without a per-keystroke allocation.
    if matches!(
        request.body,
        Request::Project(_)
            | Request::Workspace(_)
            | Request::RemoteFile(_)
            | Request::RemoteCommand(_)
    ) {
        Ok(MutationAdmission {
            _direct: None,
            blocking: Some(Arc::new(guard)),
        })
    } else {
        Ok(MutationAdmission {
            _direct: Some(guard),
            blocking: None,
        })
    }
}

async fn handle_profile_control(
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
    request: yttt_protocol::session::ProfileControlRequest,
) -> Result<yttt_protocol::session::ControlStatus, ProtocolFailure> {
    let expired = context.workspaces.control.expire_preparation();
    let changes = !matches!(
        request,
        yttt_protocol::session::ProfileControlRequest::Status
    );
    let fence = context
        .workspaces
        .control
        .request(client_id, context.identity.host_epoch, request);
    if expired {
        context
            .profile_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    let fence = fence?;
    if changes {
        context
            .profile_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
    }
    if let Some(id) = fence {
        let _guard = context.mutation_gate.write().await;
        let result = context.workspaces.control.finish(&id, |revisions| {
            context.workspaces.verify_revisions(revisions)
        });
        context
            .profile_changes
            .send_modify(|revision| *revision = revision.wrapping_add(1));
        if let Some(previous) = result? {
            context.runtime.release_client(&previous);
        }
    }
    Ok(context
        .workspaces
        .control
        .status(context.identity.host_epoch))
}

async fn handle_request(
    request: ClientRequest,
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
    attachments: &mut HashMap<yttt_core::model::ids::TerminalSessionId, TerminalAttachment>,
    host_sequence: u64,
    mutation_guard: Option<Arc<tokio::sync::OwnedRwLockReadGuard<()>>>,
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
    if let Err(failure) = authorize_ingress(&body, context.identity.ingress) {
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
        Request::SpawnTerminal {
            spec,
            start_id,
            expected_host_epoch,
        } => spawn_terminal(
            spec,
            start_id,
            expected_host_epoch,
            context,
            client_id,
            true,
        ),
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
        // v22 clients still ask; the raw replay cursor is ignored.
        Request::RequestCheckpoint { session_id, .. } => (|| {
            let terminal = runtime
                .terminal(&session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
            let display_offset = attachments
                .get(&session_id)
                .map_or(0, |attachment| attachment.display_offset);
            terminal
                .control_checkpoint_for_offset(display_offset)
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
        Request::TerminateTerminal {
            session_id,
            host_epoch,
            session_epoch,
            mode,
        } => {
            if mode == TerminationMode::Detach {
                Err(ProtocolFailure::new(
                    FailureCode::InvalidRequest,
                    "use DetachTerminal to detach without terminating the Host terminal",
                    false,
                ))
            } else {
                terminate_terminal(
                    identity.host_epoch,
                    host_epoch,
                    runtime,
                    attachments,
                    client_id,
                    &session_id,
                    session_epoch,
                )
                .map(|terminated| {
                    Response::TerminalTerminated(terminated.unwrap_or(TerminatedTerminal {
                        session_epoch,
                        final_sequence: 0,
                    }))
                })
                .map_err(runtime_failure)
            }
        }
        Request::TerminateMany { requests } => {
            let results = requests
                .into_iter()
                .map(|request| {
                    let session_id = request.session_id;
                    let result = (|| {
                        let terminated = terminate_terminal(
                            identity.host_epoch,
                            request.host_epoch,
                            runtime,
                            attachments,
                            client_id,
                            &session_id,
                            request.session_epoch,
                        )?;
                        if let Some(terminated) = terminated {
                            runtime.acknowledge_terminal_exit(
                                &session_id,
                                terminated.session_epoch,
                                terminated.final_sequence,
                            )?;
                            Ok(terminated)
                        } else {
                            Ok(TerminatedTerminal {
                                session_epoch: request.session_epoch,
                                final_sequence: 0,
                            })
                        }
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
            tokio::task::spawn_blocking(move || {
                let _guard = mutation_guard;
                ssh.remote_file(&projects, request)
            })
            .await
            .map_err(|error| ssh_failure(error.to_string()))
            .and_then(|result| result.map_err(project_failure))
        }
        Request::RemoteCommand(request) => {
            let ssh = ssh.clone();
            let projects = projects.clone();
            tokio::task::spawn_blocking(move || {
                let _guard = mutation_guard;
                ssh.remote_command(&projects, request)
            })
            .await
            .map_err(|error| ssh_failure(error.to_string()))
            .and_then(|result| result.map_err(project_failure))
        }
        Request::Project(request) => {
            let projects = projects.clone();
            let client_id = client_id.clone();
            tokio::task::spawn_blocking(move || {
                let _guard = mutation_guard;
                projects.handle(&client_id, request)
            })
            .await
            .map_err(|error| ProtocolFailure::new(FailureCode::Internal, error.to_string(), false))
            .and_then(|result| result.map(Response::Project).map_err(project_failure))
        }
        Request::ReadAgentSnapshots { acknowledged } => {
            let (snapshots, has_more) = agent_hooks.snapshot_page(&acknowledged);
            Ok(Response::AgentSnapshots {
                snapshots,
                project_token_usage: agent_hooks.project_token_usage(),
                has_more,
            })
        }
        Request::AgentCommand(request) => agent_hooks.command(request).map(|()| Response::Applied),
        Request::SetTerminalVisibility { revision, visible } => {
            runtime.set_client_visibility(client_id, revision, visible.into_iter().collect());
            Ok(Response::Applied)
        }
        Request::Workspace(request) => {
            use yttt_protocol::workspace::{WorkspaceRequest, WorkspaceResponse};
            let workspaces = context.workspaces.clone();
            let requesting_client = client_id.clone();
            let projects = context.projects.clone();
            let runtime = context.runtime.clone();
            tokio::task::spawn_blocking(move || {
                let _guard = mutation_guard;
                let result = match request {
                    WorkspaceRequest::Apply {
                        workspace_id,
                        intent,
                        expected_revision,
                    } => {
                        let applied = workspaces.apply(workspace_id.clone(), intent, expected_revision)?;
                        // Subscribers already have the new layout queued, so a
                        // terminal's exit reaches them after its pane is gone.
                        for pane in &applied.removed_panes {
                            let session_id = pane.terminal_session_id();
                            match runtime.terminate(&session_id) {
                                Ok(_) | Err(HostRuntimeError::NotFound(_)) => {}
                                Err(error) => {
                                    eprintln!("closed pane terminal {session_id} was not terminated: {error}");
                                }
                            }
                        }
                        WorkspaceResponse::Applied {
                            workspace_id,
                            revision: applied.revision,
                        }
                    }
                    request => workspaces.handle(&requesting_client, request)?,
                };
                if matches!(
                    &result,
                    WorkspaceResponse::Committed { .. }
                        | WorkspaceResponse::Applied { .. }
                        | WorkspaceResponse::Forgotten { .. }
                ) {
                    projects.sync_workspaces(workspaces.referenced_projects());
                }
                Ok(result)
            })
            .await
            .map_err(|error| ProtocolFailure::new(FailureCode::Internal, error.to_string(), false))
            .and_then(|result| result.map(Response::Workspace))
        }
        Request::ProfileControl(request) => handle_profile_control(context, client_id, request)
            .await
            .map(Response::ProfileControl),
        Request::ReadDeviceSettings => Ok(Response::DeviceSettings(
            context.device_settings.lock().settings().clone(),
        )),
        Request::SetLoginStartupConsent { granted } => context
            .device_settings
            .lock()
            .set_startup_consent(granted)
            .map(Response::DeviceSettings)
            .map_err(|error| ProtocolFailure::new(FailureCode::Internal, error.to_string(), false)),
        Request::RemoteAccess(request) => context
            .remote_access
            .request(client_id.clone(), request)
            .await
            .map(Response::RemoteAccess),
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

fn authorize_terminal_termination(
    request: &Request,
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
    attachments: &HashMap<TerminalSessionId, TerminalAttachment>,
) -> Result<(), ProtocolFailure> {
    let Request::TerminateTerminal {
        session_id,
        host_epoch,
        session_epoch,
        ..
    } = request
    else {
        return Ok(());
    };
    if *host_epoch != context.identity.host_epoch {
        return Err(runtime_failure(HostRuntimeError::StaleHostEpoch {
            expected: context.identity.host_epoch,
            actual: *host_epoch,
        }));
    }
    let Some(terminal) = context.runtime.terminal(session_id) else {
        return Ok(());
    };
    validate_terminal_session_epoch(&terminal, *session_epoch).map_err(runtime_failure)?;
    require_interactive_attachment(attachments, session_id).map_err(runtime_failure)?;
    context
        .runtime
        .validate_lease(session_id, client_id)
        .map_err(runtime_failure)?;
    Ok(())
}

fn terminate_terminal(
    current_host_epoch: u64,
    requested_host_epoch: u64,
    runtime: &HostRuntime,
    attachments: &HashMap<TerminalSessionId, TerminalAttachment>,
    client_id: &ClientInstanceId,
    session_id: &TerminalSessionId,
    requested_session_epoch: u64,
) -> Result<Option<TerminatedTerminal>, HostRuntimeError> {
    if requested_host_epoch != current_host_epoch {
        return Err(HostRuntimeError::StaleHostEpoch {
            expected: current_host_epoch,
            actual: requested_host_epoch,
        });
    }
    let Some(terminal) = runtime.terminal(session_id) else {
        return Ok(None);
    };
    validate_terminal_session_epoch(&terminal, requested_session_epoch)?;
    require_interactive_attachment(attachments, session_id)?;
    runtime.validate_lease(session_id, client_id)?;
    terminal.terminate()?;
    let final_sequence = terminal
        .checkpoint()
        .map_or(0, |checkpoint| checkpoint.viewport.sequence);
    Ok(Some(TerminatedTerminal {
        session_epoch: terminal.session_epoch(),
        final_sequence,
    }))
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
    let display_offset = runtime
        .attachment_receiver(terminal.session_id(), client_id)
        .map_or(0, |attachment| attachment.borrow().display_offset);
    let checkpoint = terminal
        .control_checkpoint_for_offset(display_offset)
        .ok_or_else(|| HostRuntimeError::NotFound(terminal.session_id().clone()))?;
    Ok(Response::TerminalAttached { lease, checkpoint })
}

fn spawn_terminal(
    spec: TerminalSpawnSpec,
    start_id: String,
    expected_host_epoch: u64,
    context: &ConnectionContext,
    client_id: &ClientInstanceId,
    allow_new: bool,
) -> Result<Response, ProtocolFailure> {
    if expected_host_epoch != context.identity.host_epoch {
        return Err(runtime_failure(HostRuntimeError::StaleHostEpoch {
            expected: context.identity.host_epoch,
            actual: expected_host_epoch,
        }));
    }
    if start_id.is_empty() || start_id.len() > MAX_START_ID_BYTES {
        return Err(ProtocolFailure::new(
            FailureCode::InvalidRequest,
            format!("terminal start ID must contain 1..={MAX_START_ID_BYTES} bytes"),
            false,
        ));
    }

    let attempt_identity = StartAttemptIdentity::from_spec(&spec);
    let mut attempts = context.start_attempts.lock();
    if let Some(record) = attempts.records.get(&start_id) {
        if record.host_epoch != context.identity.host_epoch || record.identity != attempt_identity {
            return Err(ProtocolFailure::new(
                FailureCode::AddressConflict,
                "terminal start ID was already used for a different terminal address",
                false,
            ));
        }
        return match record.outcome {
            StartAttemptOutcome::Starting => Err(outcome_unknown_failure(&start_id)),
            StartAttemptOutcome::Failed(code) => Err(failed_start_attempt_failure(&start_id, code)),
            StartAttemptOutcome::Spawned { session_epoch } => {
                let Some(terminal) = context.runtime.terminal(&spec.session_id) else {
                    return Err(outcome_unknown_failure(&start_id));
                };
                if terminal.session_epoch() != session_epoch || terminal.is_exited() {
                    return Err(outcome_unknown_failure(&start_id));
                }
                context
                    .runtime
                    .acquire_lease(&spec.session_id, client_id, TerminalLeaseMode::Interactive)
                    .map(|lease| Response::TerminalSpawned {
                        lease,
                        session_epoch,
                    })
                    .map_err(runtime_failure)
            }
        };
    }
    if !allow_new {
        return Err(outcome_unknown_failure(&start_id));
    }
    if attempts.records.len() >= MAX_START_ATTEMPTS {
        return Err(ProtocolFailure::new(
            FailureCode::ResourceLimit,
            "Host has reached its terminal start-attempt retention limit",
            false,
        ));
    }

    attempts.records.insert(
        start_id.clone(),
        StartAttemptRecord {
            host_epoch: context.identity.host_epoch,
            identity: attempt_identity,
            outcome: StartAttemptOutcome::Starting,
        },
    );

    // A different attempt must not replace the hook credentials of a live session.
    if let Some(existing) = context.runtime.terminal(&spec.session_id) {
        let expected_fingerprint = existing.spec().address_fingerprint();
        let actual_fingerprint = spec.address_fingerprint();
        let failure = runtime_failure(if expected_fingerprint == actual_fingerprint {
            HostRuntimeError::AlreadyExists(spec.session_id)
        } else {
            HostRuntimeError::AddressConflict {
                session_id: spec.session_id,
                expected_fingerprint,
                actual_fingerprint,
            }
        });
        attempts.records.get_mut(&start_id).unwrap().outcome =
            StartAttemptOutcome::Failed(failure.code);
        return Err(failure);
    }

    let mut scope = None;
    let local_cwd = context
        .projects
        .local_root(&spec.project_id)
        .ok()
        .map(|root| spec.cwd.join_under(&root));
    let (outcome, response) = match context.runtime.spawn_with_transport(
        spec,
        Some(context.ssh.transport()),
        local_cwd,
        |spec, epoch| {
            scope = Some(context.agent_hooks.secure_terminal_environment(spec, epoch));
        },
    ) {
        Ok(terminal) => {
            let session_epoch = terminal.session_epoch();
            let response = context
                .runtime
                .acquire_lease(
                    terminal.session_id(),
                    client_id,
                    TerminalLeaseMode::Interactive,
                )
                .map(|lease| Response::TerminalSpawned {
                    lease,
                    session_epoch,
                })
                .map_err(runtime_failure);
            (StartAttemptOutcome::Spawned { session_epoch }, response)
        }
        Err(error) => {
            if let Some(scope) = scope {
                context.agent_hooks.cancel_terminal(&scope);
            }
            let failure = runtime_failure(error);
            (StartAttemptOutcome::Failed(failure.code), Err(failure))
        }
    };
    attempts
        .records
        .get_mut(&start_id)
        .expect("reserved terminal start attempt must remain recorded")
        .outcome = outcome;
    response
}

fn outcome_unknown_failure(start_id: &str) -> ProtocolFailure {
    ProtocolFailure::new(
        FailureCode::OutcomeUnknown,
        format!("terminal start attempt {start_id:?} no longer has a replayable outcome"),
        false,
    )
}

fn failed_start_attempt_failure(start_id: &str, code: FailureCode) -> ProtocolFailure {
    ProtocolFailure::new(
        code,
        format!("terminal start attempt {start_id:?} previously failed"),
        false,
    )
}

fn request_creates_resource(request: &Request) -> bool {
    matches!(
        request,
        Request::SpawnTerminal { .. }
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
        Err(error) if error.raw_os_error() == fs2::lock_contended_error().raw_os_error() => {
            Ok(true)
        }
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

async fn wait_for_preparation_deadline(deadline_millis: Option<u64>) {
    match deadline_millis {
        Some(deadline_millis) => {
            tokio::time::sleep(Duration::from_millis(
                deadline_millis.saturating_sub(now_millis()),
            ))
            .await;
        }
        None => std::future::pending::<()>().await,
    }
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
        let non_secret = Request::SpawnTerminal {
            spec: TerminalSpawnSpec {
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
            },
            start_id: "journal-safe".to_string(),
            expected_host_epoch: 1,
        };
        assert!(!request_is_journalable(&sensitive));
        assert!(!request_is_journalable(&ssh));
        assert!(request_is_journalable(&non_secret));
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

#[cfg(test)]
mod session_retention_tests {
    use super::*;

    #[test]
    fn session_churn_preserves_active_clients_and_recent_replay_window() {
        let now = std::time::Instant::now();
        let mut sessions = HashMap::new();
        for index in 0..1000 {
            sessions.insert(
                ClientInstanceId::new(index.to_string()),
                SessionBinding {
                    authentication: yttt_transport::AuthenticatedSession {
                        ingress: yttt_transport::IngressKind::LocalAdmin,
                        credential_generation: 0,
                        nonce: yttt_transport::new_session_nonce(),
                    },
                    connection: (index == 0).then(|| watch::channel(false).0),
                    generation: 1,
                    slot: Arc::new(tokio::sync::Mutex::new(())),
                    last_disconnected: now - Duration::from_millis(1000 - index),
                },
            );
        }
        for id in expired_sessions(&sessions, now) {
            sessions.remove(&id);
        }
        assert_eq!(sessions.len(), MAX_IDLE_CLIENT_SESSIONS + 1);
        assert!(sessions.contains_key(&ClientInstanceId::new("0")));
        assert!(sessions.contains_key(&ClientInstanceId::new("999")));
        for id in expired_sessions(&sessions, now + CLIENT_SESSION_RETENTION) {
            sessions.remove(&id);
        }
        assert_eq!(sessions.len(), 1);
    }
}
