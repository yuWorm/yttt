#![forbid(unsafe_code)]
mod agent_control;
mod agent_hooks;
mod agent_processes;
mod audit;
mod device_settings;
pub mod diagnostics;
mod lifecycle;
pub mod logging;
mod p2p;
mod pane_launch;
mod pane_runtime;
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
    runtime::{AttachmentResyncReason, HostRuntime, HostRuntimeError},
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
    agent::{AgentSnapshotUpdate, ProjectTokenUsage},
    project::ProjectChange,
    terminal::{
        AttachTerminal, TerminalLeaseMode, TerminalSpawnSpec, TerminalViewportAnchor,
        TerminalViewportRead, TerminatedTerminal, TerminationMode,
    },
    workspace::WorkspaceProjectConfig,
};
use yttt_transport::{
    AuthToken, HostIdentity, SessionReader, SessionWriter, TransportListener, TransportStream,
    receive_lifecycle, send_lifecycle,
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
    // Held for the Host's life; without it, edits made on disk reach Clients
    // only when they next read the file.
    let _config_watcher = workspaces
        .watch_config_files()
        .inspect_err(|error| tracing::warn!(%error, "cannot watch the configuration files"))
        .ok();
    // The log detail follows `[diagnostics] verbose_logs` as it changes.
    tokio::spawn({
        let mut changes = workspaces.subscribe_config_changes();
        let config_root = bootstrap.config_root.clone();
        async move {
            while changes.changed().await.is_ok() {
                logging::set_verbose(logging::verbose_setting(&config_root));
            }
        }
    });
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
    let pane_runtime = pane_runtime::PaneRuntime::new(
        workspaces.clone(),
        runtime.clone(),
        agent_hooks.clone(),
        ssh.clone(),
        projects.clone(),
        lifecycle.clone(),
        &bootstrap.config_root,
    );
    pane_runtime.start(stop_rx.clone());
    let agent_control = Arc::new(agent_control::AgentControl::new(
        workspaces.clone(),
        projects.clone(),
        runtime.clone(),
        pane_runtime.clone(),
        audit.clone(),
        tokio::runtime::Handle::current(),
    ));
    agent_control.start(stop_rx.clone());
    agent_hooks.set_control_handler(agent_control.clone());
    if bootstrap.lifetime == HostLifetime::Independent {
        let startup = pane_runtime.clone();
        tokio::task::spawn_blocking(move || startup.ensure_all());
    }
    let (remote_access, remote_commands) = remote_access::RemoteAccessHandle::channel();
    let connection_context = ConnectionContext {
        identity: (*identity).clone(),
        profile_id: bootstrap.profile_id.clone(),
        host_sequence: next_host_sequence.clone(),
        ssh: ssh.clone(),
        agent_hooks: agent_hooks.clone(),
        projects: projects.clone(),
        workspaces: workspaces.clone(),
        pane_runtime: pane_runtime.clone(),
        agent_control: agent_control.clone(),
        device_settings: device_settings.clone(),
        remote_access,
        network_admission: None,
        device: None,
        handshake_permit: None,
        handshake_deadline: None,
        request_journals: request_journals.clone(),
        start_attempts: start_attempts.clone(),
        client_attachments: client_attachments.clone(),
        sessions: sessions.clone(),
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
    let mut next_accept = tokio::time::Instant::now();
    let mut accept_backoff = Duration::ZERO;
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
                if reaped_exits != 0 {
                    lifecycle.resource_changed();
                }
                agent_hooks.decay_stale_activity();
                prune_inactive_sessions(&connection_context);
            }
            accepted = async {
                tokio::time::sleep_until(next_accept).await;
                listener.accept().await
            } => {
                // A Client that gave up connecting, a peer that is not this
                // user, or running out of descriptors fails one accept, not
                // the Host: back off and keep listening.
                let stream = match accepted {
                    Ok(stream) => {
                        accept_backoff = Duration::ZERO;
                        stream
                    }
                    Err(error) => {
                        tracing::warn!("local accept failed: {error}");
                        accept_backoff = (accept_backoff * 2)
                            .clamp(Duration::from_millis(10), remote_access::MAX_ACCEPT_BACKOFF);
                        next_accept = tokio::time::Instant::now() + accept_backoff;
                        continue;
                    }
                };
                let mut context = connection_context.clone();
                context.identity.connection_sequence = next_connection.fetch_add(1, Ordering::Relaxed);
                let credential = Arc::new(yttt_transport::Credential::SharedSecret((*token).clone()));
                tokio::spawn(async move {
                    let _ = serve_connection(stream, credential, context).await;
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
                let credential = Arc::new(yttt_transport::Credential::SharedSecret((*work.token).clone()));
                tasks.spawn(async move { let _ = serve_connection(stream, credential, context).await; });
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
        session_queues: runtime.session_queue_diagnostics(),
    }
}

type SharedTerminalAttachments =
    Arc<tokio::sync::Mutex<HashMap<TerminalSessionId, TerminalAttachment>>>;

#[derive(Clone)]
struct PairedPeer {
    key: yttt_protocol::EndpointKey,
    preset: yttt_protocol::p2p::DevicePreset,
    devices: Arc<p2p::DeviceRegistry>,
}

#[derive(Clone)]
struct ConnectionContext {
    identity: HostIdentity,
    profile_id: ProfileId,
    host_sequence: Arc<AtomicU64>,
    ssh: Arc<HostSshRuntime>,
    agent_hooks: Arc<HostAgentHookRuntime>,
    projects: Arc<HostProjectRuntime>,
    workspaces: Arc<WorkspaceService>,
    pane_runtime: Arc<pane_runtime::PaneRuntime>,
    agent_control: Arc<agent_control::AgentControl>,
    device_settings: Arc<Mutex<device_settings::DeviceSettingsStore>>,
    remote_access: remote_access::RemoteAccessHandle,
    network_admission: Option<Arc<remote_access::NetworkAdmission>>,
    /// The paired device on the other end of a P2P connection.
    device: Option<PairedPeer>,
    handshake_permit: Option<Arc<Mutex<Option<tokio::sync::OwnedSemaphorePermit>>>>,
    handshake_deadline: Option<tokio::time::Instant>,
    request_journals:
        Arc<Mutex<HashMap<ClientInstanceId, Arc<tokio::sync::Mutex<RequestJournal>>>>>,
    start_attempts: Arc<Mutex<StartAttemptRegistry>>,
    client_attachments: Arc<Mutex<HashMap<ClientInstanceId, SharedTerminalAttachments>>>,
    sessions: Arc<Mutex<HashMap<ClientInstanceId, SessionBinding>>>,
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
    // A standalone read is worth no replay, and the journal is held while a
    // journaled request runs, which would hold up the session's queue anyway.
    !recovery_request(request)
        && !standalone_request(request)
        && !matches!(
            request,
            Request::SshConnect(_)
                | Request::CredentialAnswer { .. }
                | Request::RemoteAccess(_)
                | Request::TerminateMany { .. }
        )
}

fn authorize_ingress(
    request: &Request,
    ingress: yttt_transport::IngressKind,
    device: Option<&PairedPeer>,
) -> Result<(), ProtocolFailure> {
    if let Some(device) = device
        && !device.preset.allows(request.required_capability())
    {
        return Err(ProtocolFailure::new(
            FailureCode::PermissionDenied,
            "this paired device may only observe",
            false,
        ));
    }
    if ingress != yttt_transport::IngressKind::LocalAdmin
        && matches!(
            request,
            Request::DeleteSshCredential { .. }
                | Request::ReadDeviceSettings
                | Request::SetLoginStartupConsent { .. }
                | Request::RemoteAccess(_)
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
/// How long a disconnected Client session keeps its terminal leases.
const LEASE_RECONNECT_GRACE: Duration = Duration::from_secs(60);

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
        context.workspaces.release_client_documents(&id);
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
    credential: Arc<yttt_transport::Credential>,
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
        result = tokio::time::timeout_at(deadline, yttt_transport::server_handshake_with(&mut stream, &context.identity, credential.as_ref())) =>
            match result {
                Ok(Ok(authenticated)) => authenticated,
                Ok(Err(error)) => {
                    tracing::debug!(ingress = ?context.identity.ingress, %error, "a client handshake failed");
                    return Err(());
                }
                Err(_) => {
                    tracing::debug!(ingress = ?context.identity.ingress, "a client handshake timed out");
                    return Err(());
                }
            },
    };
    if let Some(permit) = context.handshake_permit.take() {
        permit.lock().take();
    }
    let _network_session = if let Some(admission) = &context.network_admission {
        let principal = match &context.device {
            Some(device) => remote_access::Principal::Device(device.key),
            None => remote_access::Principal::Session(authenticated.client_instance_id.clone()),
        };
        let session = admission
            .admit(
                context.identity.ingress,
                principal.clone(),
                &authenticated.client_instance_id,
                || {
                    context
                        .device
                        .as_ref()
                        .is_none_or(|device| device.devices.contains(device.key))
                },
            )
            .inspect_err(|()| {
                tracing::debug!(?principal, "refused a remote client: not admitted")
            })?;
        if let Some(device) = &context.device {
            device.devices.touch(device.key);
        }
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
                tracing::debug!(%client_id, "refused a session: the client ID belongs to another session");
                return Err(());
            }
        } else {
            if sessions.len() >= MAX_CLIENT_SESSIONS {
                tracing::debug!(%client_id, "refused a session: too many sessions");
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
    context.runtime.resume_client(&client_id);
    context.lifecycle.client_connected();
    let features = SessionFeatures {
        compress: compresses(context.identity.ingress, authenticated.accepts_compression),
        agent_board_patches: authenticated.accepts_agent_board_patches,
        agent_board_interest: omits_unwatched_boards(
            context.identity.ingress,
            authenticated.accepts_agent_board_interest,
        ),
    };
    tracing::info!(
        client = %client_id,
        generation,
        ingress = ?context.identity.ingress,
        protocol = authenticated.selected_version,
        compress = features.compress,
        board_patches = features.agent_board_patches,
        board_interest = features.agent_board_interest,
        "session connected"
    );
    let connected_at = std::time::Instant::now();
    let attachments = shared_terminal_attachments(context, &client_id);
    let request_journal = {
        let mut journals = context.request_journals.lock();
        journals
            .entry(client_id.clone())
            .or_insert_with(|| Arc::new(tokio::sync::Mutex::new(RequestJournal::default())))
            .clone()
    };
    let mut writer = None;
    let result = serve_session_connection(
        stream,
        client_id.clone(),
        attachments.clone(),
        request_journal,
        replaced,
        features,
        &mut writer,
        context,
    )
    .await;
    let stats = writer.map(|writer| writer.queue_stats());
    // A dropped connection keeps the Client's leases for a grace period: it
    // usually means the Client reconnects, not that it left.
    context.runtime.suspend_client(&client_id);
    tokio::spawn({
        let sessions = context.sessions.clone();
        let runtime = context.runtime.clone();
        let client_id = client_id.clone();
        async move {
            tokio::time::sleep(LEASE_RECONNECT_GRACE).await;
            let reconnected = sessions
                .lock()
                .get(&client_id)
                .is_some_and(|binding| binding.generation != generation);
            if !reconnected {
                runtime.release_client(&client_id);
            }
        }
    });
    context.projects.disconnect(&client_id);
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
    // The Host log is otherwise the only trace of why a Client reconnected.
    let lasted_millis = connected_at.elapsed().as_millis() as u64;
    let sent_bytes = stats.as_ref().map_or(0, |stats| stats.written_bytes);
    let queue_high_water_bytes = stats
        .as_ref()
        .map_or(0, |stats| stats.reliable_bytes_high_water);
    match &result {
        Ok(()) => tracing::info!(
            client = %client_id,
            generation,
            lasted_millis,
            sent_bytes,
            queue_high_water_bytes,
            "session ended"
        ),
        // A Client that closes its own connection, such as one that only
        // checked the Host is up, is no failure.
        Err(reason) if reason == CLIENT_CLOSED => tracing::info!(
            client = %client_id,
            generation,
            lasted_millis,
            sent_bytes,
            queue_high_water_bytes,
            %reason,
            "session ended"
        ),
        Err(reason) => tracing::warn!(
            client = %client_id,
            generation,
            lasted_millis,
            sent_bytes,
            queue_high_water_bytes,
            %reason,
            "session dropped"
        ),
    }
    result.map_err(|_| ())
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
        | Request::ReleaseTerminalControl { .. }
        | Request::TakeTerminalController => true,
        Request::Project(request) => matches!(
            request,
            yttt_protocol::project::ProjectRequest::Register { .. }
                | yttt_protocol::project::ProjectRequest::RegisterSsh { .. }
                | yttt_protocol::project::ProjectRequest::Close { .. }
        ),
        _ => false,
    }
}

/// Requests that read the Host machine's disk without touching any state a
/// session's other requests change, and may take seconds: scanning Agent
/// history. They run beside the session's queue instead of holding it, so a
/// remote window keeps reading files and configuration meanwhile.
fn standalone_request(request: &Request) -> bool {
    matches!(
        request,
        Request::Workspace(
            yttt_protocol::workspace::WorkspaceRequest::AgentSessions { .. }
                | yttt_protocol::workspace::WorkspaceRequest::OmpSessionExists { .. }
        )
    )
}

/// How many standalone requests of one session run at once; more wait in
/// the session's queue.
const MAX_STANDALONE_REQUESTS: usize = 2;

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

/// Whether a Session connection sends its large messages compressed: over a
/// network ingress, to a Client that reads them. Local IPC gains nothing.
fn compresses(ingress: yttt_transport::IngressKind, accepts_compression: bool) -> bool {
    accepts_compression && ingress != yttt_transport::IngressKind::LocalAdmin
}

/// Whether a Session connection leaves Agent boards out until its Client
/// asks for them: over a network ingress, to a Client that asks. Local IPC
/// costs nothing, and local tools read boards without asking.
fn omits_unwatched_boards(ingress: yttt_transport::IngressKind, accepts_interest: bool) -> bool {
    accepts_interest && ingress != yttt_transport::IngressKind::LocalAdmin
}

/// What a Session connection's Client reads beyond the baseline protocol.
#[derive(Clone, Copy)]
struct SessionFeatures {
    compress: bool,
    agent_board_patches: bool,
    /// Agent boards are sent only while the Client asks for them.
    agent_board_interest: bool,
}

/// Takes the boards out of an Agent snapshot page for a connection that did
/// not ask for them.
fn omit_page_boards(response: &mut HostResponse) {
    if let Ok(Response::AgentSnapshots { snapshots, .. }) = &mut response.result {
        snapshots.iter_mut().for_each(omit_board);
    }
}

fn omit_board(update: &mut AgentSnapshotUpdate) {
    update.snapshot.board = yttt_agent_core::AgentBoardSnapshot::default();
    update.board_omitted = true;
}

/// Serves one authenticated Client over its single Session connection:
/// requests and responses, terminal input, state events, and terminal frames.
/// `writer_out` receives the connection's writer, whose totals the caller logs.
#[allow(clippy::too_many_arguments)]
async fn serve_session_connection(
    stream: TransportStream,
    client_id: ClientInstanceId,
    attachments: SharedTerminalAttachments,
    request_journal: Arc<tokio::sync::Mutex<RequestJournal>>,
    mut replaced: watch::Receiver<bool>,
    features: SessionFeatures,
    writer_out: &mut Option<SessionWriter>,
    context: &ConnectionContext,
) -> Result<(), String> {
    const MAX_QUEUED_REQUESTS: usize = 64;
    let (reader, writer) = tokio::io::split(stream);
    let reader = SessionReader::new(reader);
    let (writer, _writer_task) = SessionWriter::spawn(writer);
    *writer_out = Some(writer.clone());
    if features.compress {
        writer.compress_large_messages();
    }
    let _diagnostics = context
        .runtime
        .register_session_writer(&client_id, writer.clone());
    // Whether this connection receives Agent boards.
    let (boards, boards_watched) = watch::channel(!features.agent_board_interest);
    let mut streams = TerminalStreams {
        context: TerminalStreamContext {
            writer: writer.clone(),
            runtime: context.runtime.clone(),
            host_sequence: context.host_sequence.clone(),
            client_id: client_id.clone(),
        },
        streams: HashMap::new(),
    };
    // Each opened workspace keeps at most one unsent layout, and each of its
    // documents one unsent state, on this connection, so a slow Client skips
    // straight to the latest ones.
    let _workspaces = context.workspaces.subscribe(client_id.clone(), {
        let writer = writer.clone();
        let host_sequence = context.host_sequence.clone();
        Box::new(move |event| {
            let (key, body) = match event {
                workspace::WorkspaceEvent::Layout(update) => (
                    format!("workspace-layout:{}", update.workspace_id.as_str()),
                    ServerEvent::WorkspaceChanged(Box::new(update.clone())),
                ),
                workspace::WorkspaceEvent::Document(update) => (
                    format!(
                        "workspace-document:{}:{}:{}",
                        update.workspace_id.as_str(),
                        update.state.document.project_id.as_str(),
                        update.state.document.canonical_path.display()
                    ),
                    ServerEvent::DocumentChanged(Box::new(update.clone())),
                ),
                workspace::WorkspaceEvent::AgentStateCleared(update) => (
                    format!("workspace-agent:{}", update.terminal_session_id.as_str()),
                    ServerEvent::AgentStateCleared(update.clone()),
                ),
            };
            let message = ControlMessage::Event(HostEvent {
                host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                body,
            });
            writer.send_reliable_latest(&key, &message).is_ok()
        })
    });
    let _state_events = AbortOnDrop(tokio::spawn({
        let events = forward_state_events(
            writer.clone(),
            client_id.clone(),
            attachments.clone(),
            features.agent_board_patches,
            boards_watched,
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
    // Reading never waits on request execution, so a Ping is answered even
    // while slow requests are queued.
    let (requests_tx, mut requests) = mpsc::channel(MAX_BUFFERED_REQUESTS);
    let (failed_tx, mut failed) = tokio::sync::oneshot::channel();
    let _reader = AbortOnDrop(tokio::spawn(read_session_requests(
        reader,
        writer.clone(),
        client_id.clone(),
        session_idle_timeout(context.identity.ingress),
        interactive,
        requests_tx,
        failed_tx,
    )));
    let mut stop = context.stop.clone();
    let mut queued = VecDeque::<ClientRequest>::new();
    let mut running = tokio::task::JoinSet::new();
    let mut standalone = tokio::task::JoinSet::new();
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
                let kind = request.body.kind();
                let started = std::time::Instant::now();
                let (response, changed) =
                    dispatch_control_request(request, &client_id, &attachments, &journal, &context)
                        .await;
                log_request(&client_id, kind, started, &response);
                let attached = session_id.filter(|_| response.result.is_ok());
                (response, changed, uses_attachments, attached)
            });
        }
        tokio::select! {
            request = requests.recv(), if queued.len() < MAX_QUEUED_REQUESTS => {
                let Some(request) = request else {
                    // The reader reports why it stopped before it drops its
                    // sender, so the reason is waiting even when this branch won.
                    return Err(failed
                        .try_recv()
                        .unwrap_or_else(|_| "session reader stopped".to_string()));
                };
                if let Request::SetAgentBoardInterest { watch } = request.body {
                    // A connection that always receives boards keeps them.
                    if features.agent_board_interest {
                        boards.send_replace(watch);
                    }
                    tracing::debug!(client = %client_id, watch, "Agent board interest");
                    writer
                        .send_reliable(&ControlMessage::Response(HostResponse {
                            request_id: request.request_id,
                            result: Ok(Response::Applied),
                        }))
                        .map_err(|error| error.to_string())?;
                } else if recovery_request(&request.body) {
                    let uses_attachments = request_uses_terminal_attachments(&request.body);
                    let kind = request.body.kind();
                    let started = std::time::Instant::now();
                    let (mut response, changed) = dispatch_control_request(
                        request,
                        &client_id,
                        &attachments,
                        &request_journal,
                        context,
                    )
                    .await;
                    log_request(&client_id, kind, started, &response);
                    if !*boards.borrow() {
                        omit_page_boards(&mut response);
                    }
                    writer
                        .send_reliable(&ControlMessage::Response(response))
                        .map_err(|error| error.to_string())?;
                    if uses_attachments {
                        streams.reconcile(&attachments, None).await;
                    }
                    if changed {
                        context.lifecycle.resource_changed();
                    }
                } else if standalone_request(&request.body)
                    && standalone.len() < MAX_STANDALONE_REQUESTS
                {
                    let context = context.clone();
                    let client_id = client_id.clone();
                    let attachments = attachments.clone();
                    let journal = request_journal.clone();
                    standalone.spawn(async move {
                        let kind = request.body.kind();
                        let started = std::time::Instant::now();
                        let (response, _) = dispatch_control_request(
                            request,
                            &client_id,
                            &attachments,
                            &journal,
                            &context,
                        )
                        .await;
                        log_request(&client_id, kind, started, &response);
                        response
                    });
                } else {
                    queued.push_back(request);
                }
            }
            finished = standalone.join_next(), if !standalone.is_empty() => {
                let response = finished
                    .ok_or("request set emptied")?
                    .map_err(|error| format!("request task failed: {error}"))?;
                writer
                    .send_reliable(&ControlMessage::Response(response))
                    .map_err(|error| error.to_string())?;
            }
            reason = &mut failed => {
                return Err(reason.unwrap_or_else(|_| "session reader stopped".to_string()));
            }
            finished = running.join_next(), if !running.is_empty() => {
                let (mut response, changed, uses_attachments, attached) = finished
                    .ok_or("request set emptied")?
                    .map_err(|error| format!("request task failed: {error}"))?;
                if !*boards.borrow() {
                    omit_page_boards(&mut response);
                }
                writer
                    .send_reliable(&ControlMessage::Response(response))
                    .map_err(|error| error.to_string())?;
                // Streams start after the response that attached them is queued.
                if uses_attachments {
                    streams.reconcile(&attachments, attached.as_ref()).await;
                }
                if changed {
                    context.lifecycle.resource_changed();
                }
            }
            () = writer.closed() => {
                return Err(writer
                    .close_reason()
                    .map_or_else(|| "writer closed".to_string(), |reason| reason.to_string()));
            }
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

/// Why a session ended when its Client closed the connection between messages.
const CLIENT_CLOSED: &str = "the Client closed the connection";

/// A request slower than this is logged at normal detail.
const SLOW_REQUEST: Duration = Duration::from_secs(1);

/// Logs a request the Session connection executed: every one in verbose
/// detail, a slow or failed one always.
fn log_request(
    client_id: &ClientInstanceId,
    kind: &'static str,
    started: std::time::Instant,
    response: &HostResponse,
) {
    let elapsed = started.elapsed();
    let millis = elapsed.as_millis() as u64;
    match &response.result {
        Err(failure) if failure.code != FailureCode::NotFound => tracing::info!(
            client = %client_id,
            request = kind,
            id = response.request_id,
            millis,
            code = ?failure.code,
            message = %failure.message,
            "request failed"
        ),
        _ if elapsed >= SLOW_REQUEST => tracing::info!(
            client = %client_id,
            request = kind,
            id = response.request_id,
            millis,
            "slow request"
        ),
        result if yttt_protocol::is_routine_request_kind(kind) => tracing::trace!(
            client = %client_id,
            request = kind,
            id = response.request_id,
            millis,
            ok = result.is_ok(),
            "request"
        ),
        result => tracing::debug!(
            client = %client_id,
            request = kind,
            id = response.request_id,
            millis,
            ok = result.is_ok(),
            "request"
        ),
    }
}

/// Requests a connection may buffer between its reader and its executor
/// before it is closed as unresponsive; a Client keeps far fewer in flight.
const MAX_BUFFERED_REQUESTS: usize = 1024;

/// How long a network Session connection may stay silent before it is closed;
/// a Client sends a heartbeat every 5 s.
const NETWORK_SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(60);

/// The silence after which a Session connection is presumed dead. Network
/// peers may vanish without closing; local IPC reports a closed peer itself.
fn session_idle_timeout(ingress: yttt_transport::IngressKind) -> Option<Duration> {
    (ingress != yttt_transport::IngressKind::LocalAdmin).then_some(NETWORK_SESSION_IDLE_TIMEOUT)
}

/// Reads a Session connection: answers Ping at once, hands terminal input,
/// resize and scroll to the interactive worker and every other request to
/// the connection's executor. Reports why it stopped through `failed`.
async fn read_session_requests<R: tokio::io::AsyncRead + Unpin>(
    mut reader: SessionReader<R>,
    writer: SessionWriter,
    client_id: ClientInstanceId,
    idle_timeout: Option<Duration>,
    interactive: mpsc::Sender<InteractiveRequest>,
    requests: mpsc::Sender<ClientRequest>,
    failed: tokio::sync::oneshot::Sender<String>,
) {
    let reason = loop {
        let received = match idle_timeout {
            Some(idle_timeout) => {
                match tokio::time::timeout(idle_timeout, reader.receive()).await {
                    Ok(received) => received,
                    Err(_) => {
                        break format!(
                            "no message from the Client for {} s",
                            idle_timeout.as_secs()
                        );
                    }
                }
            }
            None => reader.receive().await,
        };
        let message = match received {
            Ok(message) => message,
            Err(yttt_transport::WireError::Closed) => break CLIENT_CLOSED.to_string(),
            Err(error) => {
                let Some(request_id) = error.unsupported_request_id() else {
                    break format!("read failed: {error}");
                };
                if let Err(error) = writer.send_reliable(&ControlMessage::Response(
                    HostResponse::unsupported(request_id),
                )) {
                    break error.to_string();
                }
                continue;
            }
        };
        let queued = match message {
            ControlMessage::TerminalInput { input } => interactive
                .try_send(InteractiveRequest::Input(ClientRequest {
                    request_id: 0,
                    actor_device_id: Some(client_id.to_string()),
                    lease_epoch: None,
                    body: Request::TerminalInput(input),
                }))
                .map_err(|_| "interactive queue overflowed".to_string()),
            ControlMessage::Request(ClientRequest {
                request_id,
                body: Request::Ping { sent_millis },
                ..
            }) => writer
                .send_interactive(&ControlMessage::Response(HostResponse {
                    request_id,
                    result: Ok(Response::Pong {
                        sent_millis,
                        host_millis: now_millis(),
                    }),
                }))
                .map_err(|error| error.to_string()),
            ControlMessage::Request(request) if interactive_lane_request(&request.body) => {
                interactive
                    .try_send(InteractiveRequest::Request(request))
                    .map_err(|_| "interactive queue overflowed".to_string())
            }
            ControlMessage::Request(request) => requests
                .try_send(request)
                .map_err(|_| "request queue overflowed".to_string()),
            ControlMessage::Response(_)
            | ControlMessage::Event(_)
            | ControlMessage::Chunk(_)
            | ControlMessage::Compressed(_) => Err("Client sent a Host message".to_string()),
        };
        if let Err(reason) = queued {
            break reason;
        }
    };
    let _ = failed.send(reason);
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
    if let Err(failure) = authorize_ingress(
        &request.body,
        context.identity.ingress,
        context.device.as_ref(),
    ) {
        return HostResponse {
            request_id: request.request_id,
            result: Err(failure),
        };
    }
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
                // A reconnected Client resubscribes to a terminal whose lease it
                // kept, and keeps writing to it.
                _ if !subscriptions.contains_key(&session_id) => {
                    context.runtime.lease(&session_id, client_id)
                }
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

/// How often one connection logs rejected input for the same terminal.
const INPUT_REJECTION_LOG_INTERVAL: Duration = Duration::from_secs(10);

async fn run_interactive_requests(
    mut requests: mpsc::Receiver<InteractiveRequest>,
    writer: SessionWriter,
    client_id: ClientInstanceId,
    attachments: SharedTerminalAttachments,
    context: ConnectionContext,
) {
    // Per terminal: when a rejection was last logged and how many since.
    let mut rejections = HashMap::<TerminalSessionId, (Option<std::time::Instant>, u64)>::new();
    while let Some(request) = requests.recv().await {
        let input_session = match &request {
            InteractiveRequest::Input(ClientRequest {
                body: Request::TerminalInput(input),
                ..
            }) => Some(input.session_id.clone()),
            _ => None,
        };
        let (request, reply) = match request {
            InteractiveRequest::Input(request) => (request, false),
            InteractiveRequest::Request(request) => (request, true),
        };
        let mut attachments = attachments.lock().await;
        let response = match authorize_ingress(
            &request.body,
            context.identity.ingress,
            context.device.as_ref(),
        ) {
            Ok(()) => {
                handle_request(
                    request,
                    &context,
                    &client_id,
                    &mut attachments,
                    context.host_sequence.fetch_add(1, Ordering::Relaxed),
                )
                .await
            }
            Err(failure) => HostResponse {
                request_id: request.request_id,
                result: Err(failure),
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
        } else if let (Err(error), Some(session_id)) = (response.result, input_session) {
            let now = std::time::Instant::now();
            let (logged_at, suppressed) = rejections.entry(session_id.clone()).or_default();
            if logged_at.is_none_or(|at| now.duration_since(at) >= INPUT_REJECTION_LOG_INTERVAL) {
                tracing::warn!(
                    client = %client_id,
                    terminal = %session_id,
                    suppressed = *suppressed,
                    ?error,
                    "terminal input rejected"
                );
                *logged_at = Some(now);
                *suppressed = 0;
            } else {
                *suppressed += 1;
            }
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

/// The Agent boards one connection has queued, by snapshot key, so an Agent's
/// next state can carry only what changed on the board its Client will hold.
#[derive(Default)]
struct SentAgentBoards {
    /// The board of each Agent's last message whose writing has started.
    written: HashMap<String, (u64, yttt_agent_core::AgentBoardSnapshot)>,
    /// The board of each Agent's message still waiting; a newer one replaces it.
    waiting: HashMap<String, (u64, yttt_agent_core::AgentBoardSnapshot)>,
}

/// Queues an Agent snapshot in place of an unsent one of the same Agent, so a
/// Client on a slow link holds at most the newest state of each Agent. The
/// generation is part of the key: a later Agent in the same pane must not
/// overtake the `AgentStateCleared` queued after its predecessor.
///
/// With `boards`, a snapshot after the first of an Agent travels as an
/// `AgentSnapshotPatch` to the board the Client will hold when it arrives:
/// the one whose writing started last, since a waiting one is replaced.
///
/// Without `with_board` the snapshot leaves its board out, and the next one
/// with a board is whole: the Client drops its patch base on such a snapshot.
/// A Client that starts watching boards reads the ones it lacks itself.
fn send_agent_snapshot(
    writer: &SessionWriter,
    host_sequence: &AtomicU64,
    boards: Option<&mut SentAgentBoards>,
    with_board: bool,
    mut update: AgentSnapshotUpdate,
) -> Result<(), ()> {
    let scope = &update.scope;
    let key = format!(
        "agent-snapshot:{}:{}:{}:{}",
        scope.project_id, scope.tab_id, scope.pane_id, scope.generation
    );
    if !with_board {
        if let Some(boards) = boards {
            boards.written.remove(&key);
            boards.waiting.remove(&key);
        }
        omit_board(&mut update);
        return send_latest_state_event(
            writer,
            host_sequence,
            &key,
            ServerEvent::AgentSnapshot(Box::new(update)),
        );
    }
    let Some(boards) = boards else {
        return send_latest_state_event(
            writer,
            host_sequence,
            &key,
            ServerEvent::AgentSnapshot(Box::new(update)),
        );
    };
    let board = std::mem::take(&mut update.snapshot.board);
    loop {
        let waiting = writer.is_waiting(&key);
        if !waiting && let Some(started) = boards.waiting.remove(&key) {
            boards.written.insert(key.clone(), started);
        }
        let body = match boards.written.get(&key) {
            Some((base_sequence, base)) => ServerEvent::AgentSnapshotPatch(Box::new(
                yttt_protocol::agent::AgentSnapshotPatch {
                    update: update.clone(),
                    base_sequence: *base_sequence,
                    board: board.patch_from(base),
                },
            )),
            None => {
                let mut full = update.clone();
                full.snapshot.board = board.clone();
                ServerEvent::AgentSnapshot(Box::new(full))
            }
        };
        let message = ControlMessage::Event(HostEvent {
            host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
            body,
        });
        // A waiting message that started meanwhile is now the Client's base.
        if writer
            .send_reliable_latest_if(&key, waiting, &message)
            .map_err(|_| ())?
        {
            boards.waiting.insert(key, (update.sequence, board));
            return Ok(());
        }
    }
}

/// Queues a project's token usage in place of an unsent one of the same project.
fn send_token_usage(
    writer: &SessionWriter,
    host_sequence: &AtomicU64,
    summary: ProjectTokenUsage,
) -> Result<(), ()> {
    let key = format!("project-token-usage:{}", summary.project_id);
    send_latest_state_event(
        writer,
        host_sequence,
        &key,
        ServerEvent::ProjectTokenUsage(summary),
    )
}

/// Most changed paths one merged project change lists; past it the change
/// asks for a full refresh instead.
const MAX_MERGED_PROJECT_PATHS: usize = 256;

/// Queues a project change, merged into the one still waiting for the same
/// project, so a burst of file events (a build, a checkout) costs one queued
/// message per project instead of one per file. `waiting` holds what was last
/// queued for each project.
fn send_project_change(
    writer: &SessionWriter,
    host_sequence: &AtomicU64,
    waiting: &mut HashMap<yttt_core::model::ids::ProjectId, ProjectChange>,
    change: ProjectChange,
) -> Result<(), ()> {
    let key = format!("project-changed:{}", change.project_id);
    let change = match waiting.remove(&change.project_id) {
        Some(queued) if writer.is_waiting(&key) => merge_project_change(queued, change),
        _ => change,
    };
    waiting.insert(change.project_id.clone(), change.clone());
    send_latest_state_event(
        writer,
        host_sequence,
        &key,
        ServerEvent::ProjectChanged(change),
    )
}

/// One change covering both. A newer registration supersedes the older one's change.
fn merge_project_change(mut queued: ProjectChange, next: ProjectChange) -> ProjectChange {
    if queued.registration_epoch != next.registration_epoch {
        return next;
    }
    queued.refresh_status |= next.refresh_status;
    queued.refresh_tree |= next.refresh_tree;
    queued.refresh_all |= next.refresh_all;
    for path in next.relative_paths {
        if !queued.relative_paths.contains(&path) {
            queued.relative_paths.push(path);
        }
    }
    if queued.refresh_all || queued.relative_paths.len() > MAX_MERGED_PROJECT_PATHS {
        queued.relative_paths.clear();
        queued.refresh_all = true;
    }
    queued
}

/// Queues a notice whose content the Client reads afresh, such as
/// `ResourceCatalogChanged`: one still waiting already covers a newer one.
fn send_notice(
    writer: &SessionWriter,
    host_sequence: &AtomicU64,
    body: ServerEvent,
) -> Result<(), ()> {
    let key = match &body {
        ServerEvent::ResourceCatalogChanged => "resource-catalog-changed",
        ServerEvent::ConfigChanged => "config-changed",
        _ => unreachable!("only content-free notices are coalesced"),
    };
    send_latest_state_event(writer, host_sequence, key, body)
}

fn send_latest_state_event(
    writer: &SessionWriter,
    host_sequence: &AtomicU64,
    key: &str,
    body: ServerEvent,
) -> Result<(), ()> {
    writer
        .send_reliable_latest(
            key,
            &ControlMessage::Event(HostEvent {
                host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                body,
            }),
        )
        .map_err(|_| ())
}

/// Forwards Host state changes to one Session connection as reliable events.
/// Terminal exits travel with their terminal's stream instead.
///
/// Subscribes before returning, so a change after the Client's first
/// `ReadAgentSnapshots` page always reaches it as an event. The Client reads
/// Agent states itself on every connection, so they are not pushed up front.
fn forward_state_events(
    writer: SessionWriter,
    client_id: ClientInstanceId,
    attachments: SharedTerminalAttachments,
    agent_board_patches: bool,
    boards_watched: watch::Receiver<bool>,
    context: ConnectionContext,
) -> impl Future<Output = Result<(), ()>> + Send {
    let mut ssh_events = context.ssh.subscribe();
    let mut agent_hook_events = context.agent_hooks.subscribe();
    let mut token_usage_events = context.agent_hooks.subscribe_token_usage();
    let mut project_events = context.projects.subscribe();
    let mut runtime_events = context.runtime.subscribe();
    let mut control_events = context.runtime.subscribe_control();
    let mut config_changes = context.workspaces.subscribe_config_changes();
    let mut resource_changes = context.lifecycle.subscribe_resource_changes();
    async move {
        let context = &context;
        let mut projects_waiting = HashMap::new();
        let mut agent_boards = agent_board_patches.then(SentAgentBoards::default);
        for summary in context.agent_hooks.project_token_usage() {
            send_token_usage(&writer, &context.host_sequence, summary)?;
        }
        for change in context.projects.resync_changes() {
            send_project_change(
                &writer,
                &context.host_sequence,
                &mut projects_waiting,
                change,
            )?;
        }
        loop {
            tokio::select! {
                event = runtime_events.recv() => match event {
                    // Only a scrolled-back attachment counts output; the check
                    // avoids taking the attachment lock for every frame.
                    Ok(HostTerminalEvent::Update { session_id, .. })
                        if context.runtime.attachment_scrolled(&session_id, &client_id) =>
                    {
                        if let Some(attachment) = attachments.lock().await.get_mut(&session_id)
                            && attachment.display_offset != 0
                        {
                            attachment.unseen_output = attachment.unseen_output.saturating_add(1);
                        }
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        for (session_id, attachment) in attachments.lock().await.iter_mut() {
                            let scrolled = attachment.display_offset != 0;
                            attachment.note_lagged(skipped);
                            context
                                .runtime
                                .note_attachment_lag(session_id, &client_id, skipped, scrolled);
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                },
                event = control_events.recv() => {
                    let bodies = {
                        let mut subscriptions = attachments.lock().await;
                        match event {
                            Ok(event) => control_event(event, &client_id, &mut subscriptions)
                                .into_iter()
                                .collect(),
                            // Missed lease changes are recovered from current state.
                            Err(broadcast::error::RecvError::Lagged(_)) => {
                                resync_leases(&context.runtime, &client_id, &mut subscriptions)
                            }
                            Err(broadcast::error::RecvError::Closed) => return Err(()),
                        }
                    };
                    for body in bodies {
                        send_host_state_event(&writer, context, body)?;
                    }
                }
                event = ssh_events.recv() => {
                    let bodies = match event {
                        Ok(event) => match event.for_client(&client_id) {
                            Some(body) => vec![body.clone()],
                            None => continue,
                        },
                        // Replay current states; the catalog refresh also drops
                        // connections whose disconnect was missed.
                        Err(broadcast::error::RecvError::Lagged(_)) => context
                            .ssh
                            .statuses()
                            .into_iter()
                            .map(ServerEvent::SshStateChanged)
                            .chain([ServerEvent::ResourceCatalogChanged])
                            .collect(),
                        Err(broadcast::error::RecvError::Closed) => return Err(()),
                    };
                    for body in bodies {
                        send_host_state_event(&writer, context, body)?;
                    }
                }
                event = project_events.recv() => {
                    let changes = match event {
                        Ok(change) => vec![change],
                        Err(broadcast::error::RecvError::Lagged(_)) => context.projects.resync_changes(),
                        Err(broadcast::error::RecvError::Closed) => return Err(()),
                    };
                    for change in changes {
                        send_project_change(&writer, &context.host_sequence, &mut projects_waiting, change)?;
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
                    let with_board = *boards_watched.borrow();
                    for update in updates {
                        send_agent_snapshot(
                            &writer,
                            &context.host_sequence,
                            agent_boards.as_mut(),
                            with_board,
                            update,
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
                        send_token_usage(&writer, &context.host_sequence, summary)?;
                    }
                }
                changed = config_changes.changed() => {
                    if changed.is_err() {
                        return Err(());
                    }
                    send_notice(&writer, &context.host_sequence, ServerEvent::ConfigChanged)?;
                }
                changed = resource_changes.changed() => {
                    if changed.is_err() {
                        return Err(());
                    }
                    send_notice(&writer, &context.host_sequence, ServerEvent::ResourceCatalogChanged)?;
                }
            }
        }
    }
}
/// The event a lease or control change sends to `client_id`, updating its
/// attachment.
fn control_event(
    event: HostTerminalEvent,
    client_id: &ClientInstanceId,
    subscriptions: &mut HashMap<TerminalSessionId, TerminalAttachment>,
) -> Option<ServerEvent> {
    match event {
        HostTerminalEvent::LeaseRevoked {
            session_id,
            previous_owner,
        } if previous_owner == *client_id => {
            if let Some(attachment) = subscriptions.get_mut(&session_id) {
                attachment.mode = TerminalLeaseMode::Observer;
            }
            Some(ServerEvent::TerminalLeaseRevoked {
                session_id,
                previous_owner,
            })
        }
        HostTerminalEvent::LeaseReleased {
            session_id,
            previous_owner,
        } if previous_owner == *client_id => {
            if let Some(attachment) = subscriptions.get_mut(&session_id) {
                attachment.mode = TerminalLeaseMode::Observer;
            }
            Some(ServerEvent::TerminalLeaseReleased {
                session_id,
                previous_owner,
            })
        }
        HostTerminalEvent::ControlGranted { lease, .. } if lease.owner == *client_id => {
            if let Some(attachment) = subscriptions.get_mut(&lease.session_id) {
                attachment.mode = TerminalLeaseMode::Interactive;
                attachment.lease_epoch = lease.lease_epoch;
                attachment.last_client_sequence = 0;
            }
            Some(ServerEvent::TerminalControlGranted { lease })
        }
        HostTerminalEvent::ControllerChanged(controller) => {
            Some(ServerEvent::TerminalControllerChanged(controller))
        }
        _ => None,
    }
}

/// Brings `client_id`'s attachments in line with the leases it holds now,
/// with the events it would have received.
fn resync_leases(
    runtime: &HostRuntime,
    client_id: &ClientInstanceId,
    subscriptions: &mut HashMap<TerminalSessionId, TerminalAttachment>,
) -> Vec<ServerEvent> {
    let mut events = vec![ServerEvent::TerminalControllerChanged(
        runtime.terminal_controller(),
    )];
    for (session_id, attachment) in subscriptions.iter_mut() {
        match (attachment.mode, runtime.lease(session_id, client_id)) {
            (TerminalLeaseMode::Interactive, None) => {
                attachment.mode = TerminalLeaseMode::Observer;
                events.push(ServerEvent::TerminalLeaseRevoked {
                    session_id: session_id.clone(),
                    previous_owner: client_id.clone(),
                });
            }
            (TerminalLeaseMode::Observer, Some(lease)) => {
                attachment.mode = TerminalLeaseMode::Interactive;
                attachment.lease_epoch = lease.lease_epoch;
                attachment.last_client_sequence = 0;
                events.push(ServerEvent::TerminalControlGranted { lease });
            }
            _ => {}
        }
    }
    events
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
    if let Err(failure) =
        authorize_ingress(&body, context.identity.ingress, context.device.as_ref())
    {
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
            .map(Response::TerminalLease)
            .map_err(runtime_failure),
        Request::ReadTerminalController => {
            Ok(Response::TerminalController(runtime.terminal_controller()))
        }
        Request::TakeTerminalController => Ok(Response::TerminalController(
            runtime.take_terminal_control(client_id),
        )),
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
            context
                .agent_control
                .person_typed(&input.session_id, &input.bytes);
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
        Request::RequestCheckpoint {
            session_id,
            subscribe_only,
            ..
        } => (|| {
            let terminal = runtime
                .terminal(&session_id)
                .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
            // The stream starts with a snapshot; repeating it here doubles
            // what a reconnecting Client downloads.
            if subscribe_only {
                return Ok(Response::Applied);
            }
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
            let client_id = client_id.clone();
            tokio::task::spawn_blocking(move || projects.handle(&client_id, request))
                .await
                .map_err(|error| {
                    ProtocolFailure::new(FailureCode::Internal, error.to_string(), false)
                })
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
        // The Session connection applies it before dispatching requests.
        Request::SetAgentBoardInterest { .. } => Ok(Response::Applied),
        Request::Workspace(request) => {
            use yttt_protocol::workspace::{WorkspaceRequest, WorkspaceResponse};
            let workspaces = context.workspaces.clone();
            let requesting_client = client_id.clone();
            let projects = context.projects.clone();
            let runtime = context.runtime.clone();
            let pane_runtime = context.pane_runtime.clone();
            tokio::task::spawn_blocking(move || {
                // Requests after which the workspace may have panes to start.
                let launching = match &request {
                    WorkspaceRequest::Open { workspace_id }
                    | WorkspaceRequest::Apply { workspace_id, .. }
                    | WorkspaceRequest::Register { workspace_id, .. }
                    | WorkspaceRequest::SetRestoreOnStartup { workspace_id, .. } => {
                        Some(workspace_id.clone())
                    }
                    _ => None,
                };
                // A forgotten workspace takes its panes with it.
                let forgotten_panes = match &request {
                    WorkspaceRequest::Forget { workspace_id, .. } => {
                        workspaces.pane_addresses(workspace_id)
                    }
                    _ => Vec::new(),
                };
                let (result, removed_panes) = match request {
                    WorkspaceRequest::Apply {
                        workspace_id,
                        intent,
                        expected_revision,
                    } => {
                        let applied =
                            workspaces.apply(workspace_id.clone(), intent, expected_revision)?;
                        let response = WorkspaceResponse::Applied {
                            workspace_id,
                            revision: applied.revision,
                        };
                        (response, applied.removed_panes)
                    }
                    request => (
                        workspaces.handle(&requesting_client, request)?,
                        forgotten_panes,
                    ),
                };
                // Subscribers already have the new layout queued, so a
                // terminal's exit reaches them after its pane is gone.
                for pane in &removed_panes {
                    let session_id = pane.terminal_session_id();
                    match runtime.terminate(&session_id) {
                        Ok(_) | Err(HostRuntimeError::NotFound(_)) => {}
                        Err(error) => {
                            tracing::warn!(
                                "closed pane terminal {session_id} was not terminated: {error}"
                            );
                        }
                    }
                }
                if matches!(
                    &result,
                    WorkspaceResponse::Applied { .. } | WorkspaceResponse::Forgotten { .. }
                ) {
                    projects.sync_workspaces(workspaces.referenced_projects());
                }
                if let Some(workspace_id) = launching {
                    pane_runtime.ensure(&workspace_id);
                }
                Ok(result)
            })
            .await
            .map_err(|error| ProtocolFailure::new(FailureCode::Internal, error.to_string(), false))
            .and_then(|result| result.map(Response::Workspace))
        }
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
            .request(request)
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
    // A Client that lost terminal control takes it back before starting one.
    context
        .runtime
        .claim_terminal_control(client_id)
        .map_err(runtime_failure)?;
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
            scope = Some(context.agent_hooks.secure_terminal_environment(
                spec,
                epoch,
                yttt_core::model::layout::PaneRole::Worker,
            ));
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
        | HostRuntimeError::NotTerminalController { .. }
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
                        peer: None,
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

#[cfg(test)]
mod lease_resync_tests {
    use super::*;
    use std::collections::HashSet;

    use yttt_core::model::ids::ProjectId;
    use yttt_protocol::terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalSpawnSpec};

    #[tokio::test]
    async fn a_lagged_connection_relearns_the_leases_it_holds() {
        let runtime = HostRuntime::new();
        let held = TerminalSessionId::new("held");
        let lost = TerminalSessionId::new("lost");
        for session_id in [&held, &lost] {
            runtime
                .spawn(TerminalSpawnSpec {
                    session_id: session_id.clone(),
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
                })
                .unwrap();
        }
        let client = ClientInstanceId::new("client");
        let lease = runtime
            .acquire_lease(&held, &client, TerminalLeaseMode::Interactive)
            .unwrap();
        let lost_lease = runtime.lease(&lost, &client).unwrap();
        // The attachment missed the grant of `held`.
        let mut subscriptions = HashMap::from([
            (
                held.clone(),
                TerminalAttachment::new(TerminalLeaseMode::Observer, 0),
            ),
            (
                lost.clone(),
                TerminalAttachment::new(TerminalLeaseMode::Interactive, lost_lease.lease_epoch),
            ),
        ]);

        let events = resync_leases(&runtime, &client, &mut subscriptions);
        assert!(matches!(
            &events[..],
            [
                ServerEvent::TerminalControllerChanged(state),
                ServerEvent::TerminalControlGranted { lease: granted },
            ] if state.controller.as_ref() == Some(&client) && *granted == lease
        ));
        assert_eq!(subscriptions[&held].mode, TerminalLeaseMode::Interactive);
        assert_eq!(subscriptions[&held].lease_epoch, lease.lease_epoch);

        // Then it missed another Client taking control.
        let other = ClientInstanceId::new("other");
        runtime.take_terminal_control(&other);
        let events = resync_leases(&runtime, &client, &mut subscriptions);
        assert!(matches!(
            &events[0],
            ServerEvent::TerminalControllerChanged(state) if state.controller.as_ref() == Some(&other)
        ));
        let revoked = events[1..]
            .iter()
            .filter_map(|event| match event {
                ServerEvent::TerminalLeaseRevoked { session_id, .. } => Some(session_id.clone()),
                _ => None,
            })
            .collect::<HashSet<_>>();
        assert_eq!(revoked, HashSet::from([held.clone(), lost.clone()]));
        assert_eq!(subscriptions[&held].mode, TerminalLeaseMode::Observer);
        assert_eq!(subscriptions[&lost].mode, TerminalLeaseMode::Observer);
        assert_eq!(
            resync_leases(&runtime, &client, &mut subscriptions).len(),
            1
        );
        for session_id in [&held, &lost] {
            runtime.terminate(session_id).unwrap();
        }
    }
}

#[cfg(test)]
mod session_idle_tests {
    use super::*;

    /// A reader over one end of a pipe; `client` writes the other end.
    struct Pipe {
        client: SessionWriter,
        failed: tokio::sync::oneshot::Receiver<String>,
        requests: mpsc::Receiver<ClientRequest>,
        _held: (
            [yttt_transport::SessionWriterTask; 2],
            tokio::io::ReadHalf<tokio::io::DuplexStream>,
            mpsc::Receiver<InteractiveRequest>,
            AbortOnDrop,
        ),
    }

    fn spawn_reader(idle_timeout: Option<Duration>) -> Pipe {
        let (host, client) = tokio::io::duplex(64 * 1024);
        let (host_read, host_write) = tokio::io::split(host);
        let (client_read, client_write) = tokio::io::split(client);
        let (host_writer, host_writer_task) = SessionWriter::spawn(host_write);
        let (client, client_writer_task) = SessionWriter::spawn(client_write);
        let (interactive, interactive_requests) = mpsc::channel(8);
        let (requests_tx, requests) = mpsc::channel(8);
        let (failed_tx, failed) = tokio::sync::oneshot::channel();
        let reader = AbortOnDrop(tokio::spawn(read_session_requests(
            SessionReader::new(host_read),
            host_writer,
            ClientInstanceId::new("idle"),
            idle_timeout,
            interactive,
            requests_tx,
            failed_tx,
        )));
        Pipe {
            client,
            failed,
            requests,
            _held: (
                [host_writer_task, client_writer_task],
                client_read,
                interactive_requests,
                reader,
            ),
        }
    }

    fn workspace_request(request_id: u64) -> ControlMessage {
        ControlMessage::Request(ClientRequest {
            request_id,
            actor_device_id: None,
            lease_epoch: None,
            body: Request::Workspace(yttt_protocol::workspace::WorkspaceRequest::Environment),
        })
    }

    #[tokio::test(start_paused = true)]
    async fn a_silent_network_connection_is_closed_after_the_idle_timeout() {
        let pipe = spawn_reader(Some(Duration::from_secs(60)));
        let started = tokio::time::Instant::now();
        let reason = pipe.failed.await.unwrap();
        assert!(reason.contains("no message"), "{reason}");
        assert!(started.elapsed() >= Duration::from_secs(60));
    }

    #[tokio::test(start_paused = true)]
    async fn any_message_restarts_the_idle_timeout() {
        let mut pipe = spawn_reader(Some(Duration::from_secs(60)));
        for request_id in 1..=3 {
            tokio::time::sleep(Duration::from_secs(40)).await;
            pipe.client
                .send_reliable(&workspace_request(request_id))
                .unwrap();
            assert_eq!(pipe.requests.recv().await.unwrap().request_id, request_id);
        }
        assert!(pipe.failed.try_recv().is_err());
        let reason = pipe.failed.await.unwrap();
        assert!(reason.contains("no message"), "{reason}");
    }

    #[test]
    fn only_network_sessions_to_willing_clients_compress() {
        use yttt_transport::IngressKind;
        assert!(!compresses(IngressKind::LocalAdmin, true));
        for ingress in [
            IngressKind::TlsWork,
            IngressKind::SshWork,
            IngressKind::P2pWork,
        ] {
            assert!(compresses(ingress, true));
            assert!(!compresses(ingress, false));
        }
    }

    #[test]
    fn only_network_sessions_to_asking_clients_leave_boards_out() {
        use yttt_transport::IngressKind;
        assert!(!omits_unwatched_boards(IngressKind::LocalAdmin, true));
        for ingress in [
            IngressKind::TlsWork,
            IngressKind::SshWork,
            IngressKind::P2pWork,
        ] {
            assert!(omits_unwatched_boards(ingress, true));
            assert!(!omits_unwatched_boards(ingress, false));
        }
    }

    #[tokio::test(start_paused = true)]
    async fn local_connections_have_no_idle_timeout() {
        assert_eq!(
            session_idle_timeout(yttt_transport::IngressKind::LocalAdmin),
            None
        );
        for ingress in [
            yttt_transport::IngressKind::TlsWork,
            yttt_transport::IngressKind::SshWork,
        ] {
            assert_eq!(
                session_idle_timeout(ingress),
                Some(NETWORK_SESSION_IDLE_TIMEOUT)
            );
        }
        let mut pipe = spawn_reader(None);
        tokio::time::sleep(Duration::from_secs(600)).await;
        assert!(pipe.failed.try_recv().is_err());
    }
}

#[cfg(test)]
mod state_event_coalescing_tests {
    use super::*;
    use yttt_agent_core::{
        AgentInstanceId, AgentProcessState, AgentSnapshot, AgentTurnState, ProviderId,
    };
    use yttt_protocol::agent::AgentHookScope;

    fn agent_update(pane: &str, sequence: u64) -> AgentSnapshotUpdate {
        AgentSnapshotUpdate {
            scope: AgentHookScope {
                project_id: "project".into(),
                tab_id: "tab".into(),
                pane_id: pane.into(),
                generation: 1,
            },
            terminal_session_id: TerminalSessionId::new(format!("project:{pane}:agent")),
            host_epoch: 1,
            sequence,
            snapshot: AgentSnapshot {
                instance_id: AgentInstanceId::new("agent").unwrap(),
                provider_id: ProviderId::from_static("omp"),
                generation: 1,
                process_state: AgentProcessState::Running,
                turn_state: AgentTurnState::Idle,
                turn_seq: 0,
                waiting_reason: None,
                waiting_message: None,
                task: None,
                current_action: None,
                last_action_failed: false,
                children: Vec::new(),
                session: None,
                process_exit: None,
                state_started_at: 0,
                updated_at: sequence,
                board: Default::default(),
            },
            board_omitted: false,
        }
    }

    fn token_usage(updated_at: u64) -> ProjectTokenUsage {
        ProjectTokenUsage {
            project_id: "project".into(),
            tokens: Some(updated_at),
            partial: false,
            reported_sources: 1,
            observed_sources: 1,
            updated_at,
        }
    }

    /// A Client on a link slower than agent activity must not accumulate
    /// every intermediate state: only the newest of each one stays queued.
    #[tokio::test]
    async fn a_reader_that_falls_behind_receives_only_the_latest_agent_states() {
        let (host, client) = tokio::io::duplex(1024);
        let (_host_read, host_write) = tokio::io::split(host);
        let (writer, _writer_task) = SessionWriter::spawn(host_write);
        let host_sequence = AtomicU64::new(1);
        for sequence in 1..=100 {
            for pane in ["a", "b"] {
                send_agent_snapshot(
                    &writer,
                    &host_sequence,
                    None,
                    true,
                    agent_update(pane, sequence),
                )
                .unwrap();
            }
            send_token_usage(&writer, &host_sequence, token_usage(sequence)).unwrap();
        }

        let mut reader = SessionReader::new(client);
        let mut snapshots = Vec::new();
        let mut usages = Vec::new();
        let received = tokio::time::timeout(Duration::from_secs(5), async {
            while snapshots
                .iter()
                .filter(|(_, sequence)| *sequence == 100)
                .count()
                < 2
                || usages.last() != Some(&100)
            {
                let ControlMessage::Event(event) = reader.receive().await.unwrap() else {
                    continue;
                };
                match event.body {
                    ServerEvent::AgentSnapshot(update) => {
                        snapshots.push((update.scope.pane_id.clone(), update.sequence));
                    }
                    ServerEvent::ProjectTokenUsage(summary) => usages.push(summary.updated_at),
                    _ => {}
                }
            }
        })
        .await;
        assert!(
            received.is_ok(),
            "latest states never arrived: {snapshots:?} {usages:?}"
        );
        // What the pipe buffered before the writer blocked, plus one latest each.
        assert!(
            snapshots.len() <= 8,
            "{} agent snapshots queued",
            snapshots.len()
        );
        assert!(usages.len() <= 4, "{} token usages queued", usages.len());
    }

    fn board_with(messages: usize) -> yttt_agent_core::AgentBoardSnapshot {
        yttt_agent_core::AgentBoardSnapshot {
            messages: (0..messages)
                .map(|id| yttt_agent_core::AgentBoardMessage {
                    id: format!("message-{id}"),
                    execution_id: "run".into(),
                    role: yttt_agent_core::AgentMessageRole::Assistant,
                    text: "x".repeat(1024),
                    timestamp: id as u64,
                    truncated: false,
                })
                .collect(),
            ..Default::default()
        }
    }

    /// Each patch must apply to the board the reader holds, even when newer
    /// states replace waiting ones and writing starts in between.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn agent_board_patches_rebuild_the_newest_board_on_a_slow_reader() {
        let (host, client) = tokio::io::duplex(1024);
        let (_host_read, host_write) = tokio::io::split(host);
        let (writer, _writer_task) = SessionWriter::spawn(host_write);
        let sender = tokio::spawn(async move {
            let host_sequence = AtomicU64::new(1);
            let mut boards = SentAgentBoards::default();
            for sequence in 1..=200 {
                let mut update = agent_update("a", sequence);
                update.snapshot.board = board_with(sequence as usize);
                send_agent_snapshot(&writer, &host_sequence, Some(&mut boards), true, update)
                    .unwrap();
                if sequence % 7 == 0 {
                    tokio::task::yield_now().await;
                }
            }
            writer
        });

        let mut reader = SessionReader::new(client);
        let mut held: Option<(u64, yttt_agent_core::AgentBoardSnapshot)> = None;
        let mut patches = 0;
        tokio::time::timeout(Duration::from_secs(10), async {
            while held.as_ref().is_none_or(|(sequence, _)| *sequence < 200) {
                let ControlMessage::Event(event) = reader.receive().await.unwrap() else {
                    continue;
                };
                match event.body {
                    ServerEvent::AgentSnapshot(update) => {
                        assert!(held.is_none(), "only the first state is sent whole");
                        held = Some((update.sequence, update.snapshot.board));
                    }
                    ServerEvent::AgentSnapshotPatch(patch) => {
                        let (base_sequence, base) = held.take().expect("a base board");
                        assert_eq!(patch.base_sequence, base_sequence);
                        assert!(patch.update.snapshot.board.messages.is_empty());
                        let board =
                            yttt_agent_core::AgentBoardSnapshot::patched(&base, &patch.board)
                                .expect("the patch fits its base");
                        held = Some((patch.update.sequence, board));
                        patches += 1;
                    }
                    _ => {}
                }
            }
        })
        .await
        .expect("the newest state arrives");
        let _writer = sender.await.unwrap();
        assert!(patches > 0);
        assert_eq!(held.unwrap().1, board_with(200));
    }

    fn file_change(project: &str, path: &str) -> ProjectChange {
        ProjectChange {
            project_id: yttt_core::model::ids::ProjectId::new(project),
            registration_epoch: 1,
            relative_paths: vec![
                yttt_protocol::path::ProjectRelativePath::from_utf8(path).unwrap(),
            ],
            refresh_status: true,
            refresh_tree: false,
            refresh_all: false,
        }
    }

    /// A build writes thousands of files; a Client that reads slower than the
    /// watcher reports them must not overflow its queue and be disconnected.
    #[tokio::test]
    async fn a_burst_of_file_events_queues_one_merged_change_per_project() {
        let (host, client) = tokio::io::duplex(1024);
        let (_host_read, host_write) = tokio::io::split(host);
        let (writer, _writer_task) = SessionWriter::spawn(host_write);
        let host_sequence = AtomicU64::new(1);
        let mut waiting = HashMap::new();
        for file in 0..4 * 1024 {
            let change = file_change("build", &format!("target/{file}.o"));
            send_project_change(&writer, &host_sequence, &mut waiting, change).unwrap();
        }
        for file in ["a.rs", "b.rs"] {
            let change = file_change("edit", file);
            send_project_change(&writer, &host_sequence, &mut waiting, change).unwrap();
        }

        let mut reader = SessionReader::new(client);
        let mut changes = Vec::new();
        let received = tokio::time::timeout(Duration::from_secs(5), async {
            while !changes
                .iter()
                .any(|change: &ProjectChange| change.project_id.as_str() == "edit")
            {
                if let ControlMessage::Event(HostEvent {
                    body: ServerEvent::ProjectChanged(change),
                    ..
                }) = reader.receive().await.unwrap()
                {
                    changes.push(change);
                }
            }
        })
        .await;
        assert!(
            received.is_ok(),
            "the edit never arrived: {}",
            changes.len()
        );
        assert!(
            changes.len() <= 8,
            "{} project changes queued",
            changes.len()
        );
        let build = changes
            .iter()
            .rfind(|change| change.project_id.as_str() == "build")
            .unwrap();
        assert!(build.refresh_all && build.relative_paths.is_empty());
        let edit = changes.last().unwrap();
        assert_eq!(edit.relative_paths.len(), 2);
        assert!(edit.refresh_status && !edit.refresh_all);
    }

    #[test]
    fn a_merged_project_change_covers_both_and_a_newer_registration_wins() {
        let mut tree = file_change("project", "src");
        tree.refresh_tree = true;
        let merged = merge_project_change(file_change("project", "a.rs"), tree);
        assert_eq!(merged.relative_paths.len(), 2);
        assert!(merged.refresh_status && merged.refresh_tree && !merged.refresh_all);

        let mut newer = file_change("project", "b.rs");
        newer.registration_epoch = 2;
        assert_eq!(merge_project_change(merged, newer.clone()), newer);
    }
}
