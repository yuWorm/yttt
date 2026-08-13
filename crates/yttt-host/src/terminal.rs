use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use crate::diagnostics::{
    LatencyDiagnostics, QueueDiagnostics, TerminalPipelineDiagnosticsSnapshot,
};
use crate::terminal_data::SharedTerminalUpdate;
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::vte::ansi::Rgb;
use parking_lot::Mutex;
use portable_pty::{Child, CommandBuilder, MasterPty, PtySize, native_pty_system};
use tokio::sync::{Notify, broadcast};
use yttt_core::model::{
    ids::{ConnectionId, TerminalSessionId},
    project::RemotePathBuf,
};
use yttt_protocol::terminal::{
    RemoteTerminalExecutionSpec, SearchTerminal, SemanticViewport, TerminalCheckpoint,
    TerminalExecutionSpec, TerminalGeometry, TerminalProcessState, TerminalSearchResults,
    TerminalSpawnSpec, TerminalStreamUpdate, TerminalViewportAnchor,
};
use yttt_ssh::{
    RemoteTerminalExecution, RemoteTerminalRequest, RemoteTerminalResizeHandle,
    RemoteTerminalSession, TransportService,
};
use yttt_terminal_core::{
    TerminalParser, TerminalState,
    semantic::{SemanticAccessError, SemanticCaptureContext, SemanticSnapshotter},
};

const RAW_REPLAY_BYTES: usize = 8 * 1024 * 1024;
const WRITER_QUEUE_CAPACITY: usize = 1024;
const EVENT_QUEUE_CAPACITY: usize = 256;
const SUBSCRIBER_CAPACITY: usize = 64;
// Capture at most once per quarter frame so parser, IPC, and GPUI scheduling
// still have time to reach the next 60 Hz presentation without busy-polling.
const OUTPUT_CAPTURE_INTERVAL: Duration = Duration::from_millis(4);

struct TerminalPipelineDiagnostics {
    subscribers: AtomicUsize,
    bytes_parsed: AtomicU64,
    semantic_encode_count: AtomicU64,
    shared_ipc_encode_count: Arc<AtomicU64>,
    skipped_unsubscribed_captures: AtomicU64,
    parser: LatencyDiagnostics,
    semantic_encode: LatencyDiagnostics,
    input_to_pty: LatencyDiagnostics,
    writer_queue: Arc<QueueDiagnostics>,
    event_queue: Arc<QueueDiagnostics>,
}

impl TerminalPipelineDiagnostics {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            subscribers: AtomicUsize::new(0),
            bytes_parsed: AtomicU64::new(0),
            semantic_encode_count: AtomicU64::new(0),
            shared_ipc_encode_count: Arc::new(AtomicU64::new(0)),
            skipped_unsubscribed_captures: AtomicU64::new(0),
            parser: LatencyDiagnostics::default(),
            semantic_encode: LatencyDiagnostics::default(),
            input_to_pty: LatencyDiagnostics::default(),
            writer_queue: QueueDiagnostics::new("terminal_writer", WRITER_QUEUE_CAPACITY),
            event_queue: QueueDiagnostics::new("terminal_events", EVENT_QUEUE_CAPACITY),
        })
    }

    fn snapshot(&self, session_id: &TerminalSessionId) -> TerminalPipelineDiagnosticsSnapshot {
        TerminalPipelineDiagnosticsSnapshot {
            session_id: session_id.to_string(),
            subscribers: self.subscribers.load(Ordering::Acquire),
            bytes_parsed: self.bytes_parsed.load(Ordering::Acquire),
            semantic_encode_count: self.semantic_encode_count.load(Ordering::Acquire),
            shared_ipc_encode_count: self.shared_ipc_encode_count.load(Ordering::Acquire),
            skipped_unsubscribed_captures: self
                .skipped_unsubscribed_captures
                .load(Ordering::Acquire),
            parser: self.parser.snapshot(),
            semantic_encode: self.semantic_encode.snapshot(),
            input_to_pty: self.input_to_pty.snapshot(),
            queues: vec![self.writer_queue.snapshot(), self.event_queue.snapshot()],
        }
    }

    fn reset(&self) {
        self.bytes_parsed.store(0, Ordering::Release);
        self.semantic_encode_count.store(0, Ordering::Release);
        self.shared_ipc_encode_count.store(0, Ordering::Release);
        self.skipped_unsubscribed_captures
            .store(0, Ordering::Release);
        self.parser.reset();
        self.semantic_encode.reset();
        self.input_to_pty.reset();
        self.writer_queue.reset();
        self.event_queue.reset();
    }
}

#[derive(Clone, Debug)]
pub enum HostTerminalEvent {
    Update {
        session_id: TerminalSessionId,
        update: Arc<SharedTerminalUpdate>,
    },
    Exited {
        session_id: TerminalSessionId,
        session_epoch: u64,
        code: Option<i32>,
        final_sequence: u64,
    },
    TitleChanged {
        session_id: TerminalSessionId,
        title: Option<String>,
    },
    Bell {
        session_id: TerminalSessionId,
    },
    LeaseRevoked {
        session_id: TerminalSessionId,
        previous_owner: yttt_core::model::ids::ClientInstanceId,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum HostedTerminalError {
    #[error("terminal execution kind is not supported by the local PTY runtime")]
    UnsupportedExecution,
    #[error("terminal geometry must contain at least one row and one column")]
    InvalidGeometry,
    #[error("terminal writer is backpressured")]
    Backpressure,
    #[error("terminal session has stopped")]
    Stopped,
    #[error("stale geometry epoch {received}; current epoch is {current}")]
    StaleGeometry { received: u64, current: u64 },
    #[error("stale scrollback epoch {received}; current epoch is {current}")]
    StaleScrollback { received: u64, current: u64 },
    #[error("terminal line ID {0} is no longer available")]
    UnknownLineId(u64),
    #[error("terminal search query and result limit must be non-empty")]
    InvalidSearch,
    #[error("terminal I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("terminal PTY failed: {0}")]
    Pty(#[from] anyhow::Error),
}

impl From<SemanticAccessError> for HostedTerminalError {
    fn from(error: SemanticAccessError) -> Self {
        match error {
            SemanticAccessError::StaleScrollback { received, current } => {
                Self::StaleScrollback { received, current }
            }
            SemanticAccessError::UnknownLineId(line_id) => Self::UnknownLineId(line_id),
            SemanticAccessError::InvalidSearch => Self::InvalidSearch,
        }
    }
}

#[derive(Clone)]
struct HostEventProxy {
    events: flume::Sender<Event>,
    diagnostics: Arc<QueueDiagnostics>,
}

impl EventListener for HostEventProxy {
    fn send_event(&self, event: Event) {
        let required = matches!(
            event,
            Event::PtyWrite(_)
                | Event::ClipboardLoad(..)
                | Event::ColorRequest(..)
                | Event::TextAreaSizeRequest(_)
                | Event::Exit
        );
        if required {
            if self.events.send(event).is_ok() {
                self.diagnostics.observe(self.events.len());
            }
        } else {
            match self.events.try_send(event) {
                Ok(()) => self.diagnostics.observe(self.events.len()),
                Err(flume::TrySendError::Full(_)) => self.diagnostics.dropped(),
                Err(flume::TrySendError::Disconnected(_)) => {}
            }
        }
    }
}

#[derive(Clone)]
pub struct HostedTerminal {
    inner: Arc<HostedTerminalInner>,
}

struct HostedTerminalInner {
    spec: TerminalSpawnSpec,
    session_epoch: u64,
    state: Mutex<TerminalState<HostEventProxy>>,
    snapshots: Mutex<SemanticSnapshotter>,
    metadata: Mutex<TerminalMetadata>,
    raw_replay: Mutex<RawReplayRing>,
    query_palette: Mutex<Vec<u32>>,
    backend: TerminalBackend,
    writer: flume::Sender<WriterCommand>,
    events: broadcast::Sender<HostTerminalEvent>,
    capture_notify: Notify,
    capture_requested: AtomicBool,
    stopped: AtomicBool,
    exited: AtomicBool,
    exited_at_millis: AtomicU64,
    reader_finished: AtomicBool,
    diagnostics: Arc<TerminalPipelineDiagnostics>,
}

enum TerminalBackend {
    Local {
        master: Arc<Mutex<Box<dyn MasterPty + Send>>>,
        child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
    },
    Remote {
        resize: RemoteTerminalResizeHandle,
        session: Mutex<Option<RemoteTerminalSession>>,
    },
}

#[derive(Clone)]
struct TerminalMetadata {
    geometry: TerminalGeometry,
    geometry_epoch: u64,
    palette_revision: u64,
    title: Option<String>,
    cwd: Option<String>,
    process_state: TerminalProcessState,
}

impl TerminalMetadata {
    fn capture_context(&self) -> SemanticCaptureContext {
        SemanticCaptureContext {
            geometry: self.geometry,
            geometry_epoch: self.geometry_epoch,
            palette_revision: self.palette_revision,
            title: self.title.clone(),
            cwd: self.cwd.clone(),
            process_state: self.process_state,
        }
    }
}

enum WriterCommand {
    Input {
        bytes: Vec<u8>,
        enqueued_at: Instant,
    },
    Reply(Vec<u8>),
    Shutdown,
}

struct RawReplayRing {
    bytes: VecDeque<u8>,
    dropped_bytes: u64,
}

impl RawReplayRing {
    fn new() -> Self {
        Self {
            bytes: VecDeque::new(),
            dropped_bytes: 0,
        }
    }

    fn append(&mut self, bytes: &[u8]) {
        if bytes.len() >= RAW_REPLAY_BYTES {
            self.dropped_bytes = self
                .dropped_bytes
                .saturating_add(self.bytes.len() as u64)
                .saturating_add((bytes.len() - RAW_REPLAY_BYTES) as u64);
            self.bytes.clear();
            self.bytes
                .extend(bytes[bytes.len() - RAW_REPLAY_BYTES..].iter().copied());
            return;
        }
        let overflow = self
            .bytes
            .len()
            .saturating_add(bytes.len())
            .saturating_sub(RAW_REPLAY_BYTES);
        for _ in 0..overflow {
            self.bytes.pop_front();
        }
        self.dropped_bytes = self.dropped_bytes.saturating_add(overflow as u64);
        self.bytes.extend(bytes.iter().copied());
    }

    fn snapshot(&self) -> (Vec<u8>, u64) {
        (self.bytes.iter().copied().collect(), self.dropped_bytes)
    }
}

impl HostedTerminal {
    pub fn spawn(spec: TerminalSpawnSpec, session_epoch: u64) -> Result<Self, HostedTerminalError> {
        validate_geometry(spec.geometry)?;
        let pty_system = native_pty_system();
        let pair = pty_system.openpty(PtySize {
            rows: spec.geometry.rows,
            cols: spec.geometry.cols,
            pixel_width: spec.geometry.cell_width,
            pixel_height: spec.geometry.cell_height,
        })?;
        let command = command_builder(&spec)?;
        let mut child = pair.slave.spawn_command(command)?;
        drop(pair.slave);
        let reader = pair.master.try_clone_reader()?;
        let mut writer = pair.master.take_writer()?;
        if let Some(command) = startup_command(&spec)
            && let Err(error) = writer.write_all(command.as_bytes())
        {
            let _ = child.kill();
            return Err(error.into());
        }
        let backend = TerminalBackend::Local {
            master: Arc::new(Mutex::new(pair.master)),
            child: Arc::new(Mutex::new(child)),
        };
        Self::from_io(spec, session_epoch, reader, writer, backend, false)
    }

    pub fn spawn_remote(
        spec: TerminalSpawnSpec,
        session_epoch: u64,
        transport: &TransportService,
    ) -> Result<Self, HostedTerminalError> {
        validate_geometry(spec.geometry)?;
        let TerminalExecutionSpec::Ssh {
            connection_id,
            execution,
        } = &spec.execution
        else {
            return Err(HostedTerminalError::UnsupportedExecution);
        };
        let execution = match execution {
            RemoteTerminalExecutionSpec::Shell { command } => RemoteTerminalExecution::Shell {
                command: command.clone(),
            },
            RemoteTerminalExecutionSpec::Command { program, args } => {
                RemoteTerminalExecution::Command {
                    program: program.clone(),
                    args: args.clone(),
                }
            }
        };
        let mut session = transport
            .terminal_session(RemoteTerminalRequest {
                connection_id: ConnectionId::new(connection_id.clone()),
                cwd: RemotePathBuf::new(spec.cwd.clone())
                    .map_err(|error| HostedTerminalError::Pty(error.into()))?,
                execution,
                environment: spec.environment.iter().cloned().collect::<BTreeMap<_, _>>(),
                cols: spec.geometry.cols,
                rows: spec.geometry.rows,
            })
            .map_err(|error| HostedTerminalError::Pty(error.into()))?;
        let resize = session.resize_handle();
        let io = session.take_io().ok_or(HostedTerminalError::Stopped)?;
        let backend = TerminalBackend::Remote {
            resize,
            session: Mutex::new(Some(session)),
        };
        Self::from_io(
            spec,
            session_epoch,
            Box::new(io.reader),
            Box::new(io.writer),
            backend,
            true,
        )
    }

    fn from_io(
        spec: TerminalSpawnSpec,
        session_epoch: u64,
        reader: Box<dyn Read + Send>,
        writer: Box<dyn Write + Send>,
        backend: TerminalBackend,
        mark_exit_on_eof: bool,
    ) -> Result<Self, HostedTerminalError> {
        let diagnostics = TerminalPipelineDiagnostics::new();
        let (terminal_event_tx, terminal_event_rx) = flume::bounded(EVENT_QUEUE_CAPACITY);
        let event_proxy = HostEventProxy {
            events: terminal_event_tx,
            diagnostics: diagnostics.event_queue.clone(),
        };
        let state = TerminalState::new_with_scrollback(
            spec.geometry.cols as usize,
            spec.geometry.rows as usize,
            spec.scrollback_limit as usize,
            event_proxy,
        );
        let (writer_tx, writer_rx) = flume::bounded(WRITER_QUEUE_CAPACITY);
        let (events, _) = broadcast::channel(SUBSCRIBER_CAPACITY);
        let inner = Arc::new(HostedTerminalInner {
            snapshots: Mutex::new(SemanticSnapshotter::new(
                spec.session_id.clone(),
                session_epoch,
            )),
            metadata: Mutex::new(TerminalMetadata {
                geometry: spec.geometry,
                geometry_epoch: spec.geometry_epoch,
                palette_revision: spec.palette_revision,
                title: None,
                cwd: Some(spec.cwd.clone()),
                process_state: TerminalProcessState::Running,
            }),
            query_palette: Mutex::new(spec.query_palette.clone()),
            raw_replay: Mutex::new(RawReplayRing::new()),
            spec,
            session_epoch,
            state: Mutex::new(state),
            backend,
            writer: writer_tx,
            events,
            capture_notify: Notify::new(),
            capture_requested: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            exited: AtomicBool::new(false),
            exited_at_millis: AtomicU64::new(0),
            reader_finished: AtomicBool::new(false),
            diagnostics,
        });
        spawn_writer(writer, writer_rx, inner.clone());
        spawn_event_processor(terminal_event_rx, inner.clone());
        inner.capture_and_publish();
        spawn_output_publisher(inner.clone());
        spawn_reader(reader, inner.clone(), mark_exit_on_eof);
        if let TerminalBackend::Local { child, .. } = &inner.backend {
            spawn_child_monitor(inner.clone(), child.clone());
        }
        Ok(Self { inner })
    }

    pub fn session_id(&self) -> &TerminalSessionId {
        &self.inner.spec.session_id
    }

    pub fn spec(&self) -> &TerminalSpawnSpec {
        &self.inner.spec
    }

    pub fn session_epoch(&self) -> u64 {
        self.inner.session_epoch
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HostTerminalEvent> {
        self.inner.events.subscribe()
    }

    pub fn input(&self, bytes: Vec<u8>) -> Result<(), HostedTerminalError> {
        if self.inner.stopped.load(Ordering::Acquire) {
            return Err(HostedTerminalError::Stopped);
        }
        match self.inner.writer.try_send(WriterCommand::Input {
            bytes,
            enqueued_at: Instant::now(),
        }) {
            Ok(()) => {
                self.inner
                    .diagnostics
                    .writer_queue
                    .observe(self.inner.writer.len());
                Ok(())
            }
            Err(flume::TrySendError::Full(_)) => {
                self.inner.diagnostics.writer_queue.dropped();
                Err(HostedTerminalError::Backpressure)
            }
            Err(flume::TrySendError::Disconnected(_)) => Err(HostedTerminalError::Stopped),
        }
    }

    pub fn set_subscriber_count(&self, count: usize) {
        let previous = self
            .inner
            .diagnostics
            .subscribers
            .swap(count, Ordering::AcqRel);
        if previous == 0 && count != 0 {
            self.inner.request_output_capture();
        }
    }

    pub fn diagnostics(&self) -> TerminalPipelineDiagnosticsSnapshot {
        self.inner.diagnostics.snapshot(self.session_id())
    }

    pub(crate) fn shared_update(&self, update: TerminalStreamUpdate) -> Arc<SharedTerminalUpdate> {
        SharedTerminalUpdate::new(
            update,
            self.inner.diagnostics.shared_ipc_encode_count.clone(),
        )
    }

    pub fn reset_diagnostics(&self) {
        self.inner.diagnostics.reset();
    }

    pub fn resize(
        &self,
        geometry: TerminalGeometry,
        geometry_epoch: u64,
    ) -> Result<(), HostedTerminalError> {
        validate_geometry(geometry)?;
        {
            let mut metadata = self.inner.metadata.lock();
            if geometry_epoch <= metadata.geometry_epoch {
                return Err(HostedTerminalError::StaleGeometry {
                    received: geometry_epoch,
                    current: metadata.geometry_epoch,
                });
            }
            self.inner.backend.resize(geometry)?;
            self.inner
                .state
                .lock()
                .resize(geometry.cols as usize, geometry.rows as usize);
            metadata.geometry = geometry;
            metadata.geometry_epoch = geometry_epoch;
        }
        self.inner.capture_and_publish();
        Ok(())
    }

    pub fn scroll(&self, lines: i32, scrollback_epoch: u64) -> Result<(), HostedTerminalError> {
        let current_epoch = self
            .latest_viewport()
            .map_or(scrollback_epoch, |viewport| viewport.scrollback_epoch);
        if current_epoch != scrollback_epoch {
            return Err(HostedTerminalError::StaleScrollback {
                received: scrollback_epoch,
                current: current_epoch,
            });
        }
        self.inner.state.lock().scroll_display(Scroll::Delta(lines));
        self.inner.capture_and_publish();
        Ok(())
    }

    pub fn set_query_palette(&self, colors: Vec<u32>, revision: u64) {
        *self.inner.query_palette.lock() = colors;
        self.inner.metadata.lock().palette_revision = revision;
        self.inner.capture_and_publish();
    }

    pub fn checkpoint(&self) -> Option<TerminalCheckpoint> {
        let (raw, start) = self.inner.raw_replay.lock().snapshot();
        self.inner.snapshots.lock().checkpoint(raw, start)
    }

    pub fn latest_viewport(&self) -> Option<SemanticViewport> {
        self.inner.snapshots.lock().latest_viewport().cloned()
    }

    pub fn read_viewport(
        &self,
        scrollback_epoch: u64,
        anchor: TerminalViewportAnchor,
    ) -> Result<SemanticViewport, HostedTerminalError> {
        let context = self.inner.metadata.lock().capture_context();
        let state = self.inner.state.lock();
        self.inner
            .snapshots
            .lock()
            .read_viewport(&state, &context, scrollback_epoch, anchor)
            .map_err(HostedTerminalError::from)
    }

    pub fn search(
        &self,
        request: &SearchTerminal,
    ) -> Result<TerminalSearchResults, HostedTerminalError> {
        let state = self.inner.state.lock();
        self.inner
            .snapshots
            .lock()
            .search(
                &state,
                request.scrollback_epoch,
                request.generation,
                &request.query,
                request.case_sensitive,
                request.max_results,
            )
            .map_err(HostedTerminalError::from)
    }

    pub fn terminate(&self) -> Result<(), HostedTerminalError> {
        if self.inner.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        if self.inner.writer.try_send(WriterCommand::Shutdown).is_ok() {
            self.inner
                .diagnostics
                .writer_queue
                .observe(self.inner.writer.len());
        }
        self.inner.backend.terminate()?;
        self.inner.mark_exited(None);
        Ok(())
    }

    pub fn is_stopped(&self) -> bool {
        self.inner.stopped.load(Ordering::Acquire)
    }

    pub fn is_exited(&self) -> bool {
        self.inner.exited.load(Ordering::Acquire)
    }

    pub fn exited_at_millis(&self) -> Option<u64> {
        let timestamp = self.inner.exited_at_millis.load(Ordering::Acquire);
        (timestamp != 0).then_some(timestamp)
    }
}

impl TerminalBackend {
    fn resize(&self, geometry: TerminalGeometry) -> Result<(), HostedTerminalError> {
        match self {
            Self::Local { master, .. } => master.lock().resize(PtySize {
                rows: geometry.rows,
                cols: geometry.cols,
                pixel_width: geometry.cell_width,
                pixel_height: geometry.cell_height,
            })?,
            Self::Remote { resize, .. } => resize
                .resize(geometry.cols as usize, geometry.rows as usize)
                .map_err(|message| HostedTerminalError::Pty(anyhow::anyhow!(message)))?,
        }
        Ok(())
    }

    fn terminate(&self) -> Result<(), HostedTerminalError> {
        match self {
            Self::Local { child, .. } => child.lock().kill().map_err(Into::into),
            Self::Remote { session, .. } => {
                if let Some(session) = session.lock().take() {
                    let _ = session.finish(true);
                }
                Ok(())
            }
        }
    }
}

fn validate_geometry(geometry: TerminalGeometry) -> Result<(), HostedTerminalError> {
    if geometry.cols == 0 || geometry.rows == 0 {
        return Err(HostedTerminalError::InvalidGeometry);
    }
    Ok(())
}

impl HostedTerminalInner {
    fn request_output_capture(&self) {
        self.capture_requested.store(true, Ordering::Release);
        self.capture_notify.notify_one();
    }

    fn capture_and_publish(&self) -> u64 {
        let started_at = Instant::now();
        let context = self.metadata.lock().capture_context();
        let (update, sequence) = {
            let state = self.state.lock();
            let mut snapshots = self.snapshots.lock();
            let update = snapshots.capture(&state, &context);
            let sequence = snapshots
                .latest_viewport()
                .map_or(0, |viewport| viewport.sequence);
            (update, sequence)
        };
        self.diagnostics
            .semantic_encode
            .record(started_at.elapsed());
        self.diagnostics
            .semantic_encode_count
            .fetch_add(1, Ordering::Relaxed);
        let update =
            SharedTerminalUpdate::new(update, self.diagnostics.shared_ipc_encode_count.clone());
        let _ = self.events.send(HostTerminalEvent::Update {
            session_id: self.spec.session_id.clone(),
            update,
        });
        sequence
    }

    fn capture_output_and_publish(&self) {
        if self.diagnostics.subscribers.load(Ordering::Acquire) == 0 {
            self.diagnostics
                .skipped_unsubscribed_captures
                .fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.capture_and_publish();
    }

    fn mark_exited(&self, code: Option<i32>) {
        if self.exited.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stopped.store(true, Ordering::Release);
        self.exited_at_millis
            .store(now_millis().max(1), Ordering::Release);
        self.metadata.lock().process_state = TerminalProcessState::Exited { code };
        let final_sequence = self.capture_and_publish();
        let _ = self.events.send(HostTerminalEvent::Exited {
            session_id: self.spec.session_id.clone(),
            session_epoch: self.session_epoch,
            code,
            final_sequence,
        });
        if self.writer.try_send(WriterCommand::Shutdown).is_ok() {
            self.diagnostics.writer_queue.observe(self.writer.len());
        }
    }
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn spawn_reader(
    mut reader: Box<dyn Read + Send>,
    inner: Arc<HostedTerminalInner>,
    mark_exit_on_eof: bool,
) {
    thread::Builder::new()
        .name(format!("yttt-host-pty-read-{}", inner.spec.session_id))
        .spawn(move || {
            let term = inner.state.lock().term_arc();
            let mut parser = TerminalParser::new(term);
            let mut buffer = vec![0_u8; u16::MAX as usize];
            loop {
                match reader.read(&mut buffer) {
                    Ok(0) => break,
                    Ok(length) => {
                        let bytes = &buffer[..length];
                        inner.raw_replay.lock().append(bytes);
                        let started_at = Instant::now();
                        parser.advance(bytes);
                        inner.diagnostics.parser.record(started_at.elapsed());
                        inner
                            .diagnostics
                            .bytes_parsed
                            .fetch_add(length as u64, Ordering::Relaxed);
                        inner.request_output_capture();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
            inner.reader_finished.store(true, Ordering::Release);
            if mark_exit_on_eof {
                inner.mark_exited(None);
            }
        })
        .expect("failed to spawn Host PTY reader");
}

fn spawn_output_publisher(inner: Arc<HostedTerminalInner>) {
    tokio::spawn(async move {
        let mut last_capture = Instant::now() - OUTPUT_CAPTURE_INTERVAL;
        loop {
            inner.capture_notify.notified().await;
            if inner.stopped.load(Ordering::Acquire) {
                break;
            }
            let elapsed = last_capture.elapsed();
            if elapsed < OUTPUT_CAPTURE_INTERVAL {
                tokio::time::sleep(OUTPUT_CAPTURE_INTERVAL - elapsed).await;
            }
            if inner.stopped.load(Ordering::Acquire) {
                break;
            }
            if !inner.capture_requested.swap(false, Ordering::AcqRel) {
                continue;
            }
            inner.capture_output_and_publish();
            last_capture = Instant::now();
        }
    });
}

fn spawn_writer(
    mut writer: Box<dyn Write + Send>,
    commands: flume::Receiver<WriterCommand>,
    inner: Arc<HostedTerminalInner>,
) {
    thread::Builder::new()
        .name(format!("yttt-host-pty-write-{}", inner.spec.session_id))
        .spawn(move || {
            while let Ok(command) = commands.recv() {
                inner.diagnostics.writer_queue.observe(commands.len());
                let (bytes, enqueued_at) = match command {
                    WriterCommand::Input { bytes, enqueued_at } => (bytes, Some(enqueued_at)),
                    WriterCommand::Reply(bytes) => (bytes, None),
                    WriterCommand::Shutdown => break,
                };
                if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
                    break;
                }
                if let Some(enqueued_at) = enqueued_at {
                    inner.diagnostics.input_to_pty.record(enqueued_at.elapsed());
                }
            }
        })
        .expect("failed to spawn Host PTY writer");
}

fn spawn_event_processor(events: flume::Receiver<Event>, inner: Arc<HostedTerminalInner>) {
    thread::Builder::new()
        .name(format!(
            "yttt-host-terminal-events-{}",
            inner.spec.session_id
        ))
        .spawn(move || {
            while let Ok(event) = events.recv() {
                inner.diagnostics.event_queue.observe(events.len());
                match event {
                    Event::PtyWrite(data) => {
                        if inner
                            .writer
                            .send(WriterCommand::Reply(data.into_bytes()))
                            .is_ok()
                        {
                            inner.diagnostics.writer_queue.observe(inner.writer.len());
                        }
                    }
                    Event::Title(title) => {
                        inner.metadata.lock().title = Some(title.clone());
                        let _ = inner.events.send(HostTerminalEvent::TitleChanged {
                            session_id: inner.spec.session_id.clone(),
                            title: Some(title),
                        });
                        inner.capture_and_publish();
                    }
                    Event::ResetTitle => {
                        inner.metadata.lock().title = None;
                        let _ = inner.events.send(HostTerminalEvent::TitleChanged {
                            session_id: inner.spec.session_id.clone(),
                            title: None,
                        });
                        inner.capture_and_publish();
                    }
                    Event::Bell => {
                        let _ = inner.events.send(HostTerminalEvent::Bell {
                            session_id: inner.spec.session_id.clone(),
                        });
                    }
                    Event::TextAreaSizeRequest(formatter) => {
                        let geometry = inner.metadata.lock().geometry;
                        let reply = formatter(WindowSize {
                            num_lines: geometry.rows,
                            num_cols: geometry.cols,
                            cell_width: geometry.cell_width,
                            cell_height: geometry.cell_height,
                        });
                        if inner
                            .writer
                            .send(WriterCommand::Reply(reply.into_bytes()))
                            .is_ok()
                        {
                            inner.diagnostics.writer_queue.observe(inner.writer.len());
                        }
                    }
                    Event::Exit => {
                        let _ = inner.backend.terminate();
                    }
                    Event::ChildExit(code) => inner.mark_exited(Some(code)),
                    Event::ColorRequest(index, formatter) => {
                        if let Some(color) = inner.query_palette.lock().get(index).copied() {
                            let reply = formatter(Rgb {
                                r: ((color >> 16) & 0xff) as u8,
                                g: ((color >> 8) & 0xff) as u8,
                                b: (color & 0xff) as u8,
                            });
                            if inner
                                .writer
                                .send(WriterCommand::Reply(reply.into_bytes()))
                                .is_ok()
                            {
                                inner.diagnostics.writer_queue.observe(inner.writer.len());
                            }
                        }
                    }
                    Event::Wakeup
                    | Event::ClipboardStore(..)
                    | Event::ClipboardLoad(..)
                    | Event::MouseCursorDirty
                    | Event::CursorBlinkingChange => {}
                }
            }
        })
        .expect("failed to spawn Host terminal event processor");
}

fn spawn_child_monitor(
    inner: Arc<HostedTerminalInner>,
    child: Arc<Mutex<Box<dyn Child + Send + Sync>>>,
) {
    thread::Builder::new()
        .name(format!("yttt-host-child-wait-{}", inner.spec.session_id))
        .spawn(move || {
            loop {
                let status = child.lock().try_wait();
                match status {
                    Ok(Some(status)) => {
                        while !inner.reader_finished.load(Ordering::Acquire) {
                            thread::sleep(Duration::from_millis(1));
                        }
                        inner.mark_exited(i32::try_from(status.exit_code()).ok());
                        break;
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(25)),
                    Err(_) => {
                        while !inner.reader_finished.load(Ordering::Acquire) {
                            thread::sleep(Duration::from_millis(1));
                        }
                        inner.mark_exited(None);
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn Host child monitor");
}

fn command_builder(spec: &TerminalSpawnSpec) -> Result<CommandBuilder, HostedTerminalError> {
    let mut command = match &spec.execution {
        TerminalExecutionSpec::Shell { program, args, .. } => {
            let mut command = CommandBuilder::new(program);
            command.args(args);
            command
        }
        TerminalExecutionSpec::Command {
            shell,
            program,
            args,
            return_to_shell,
        } => {
            if *return_to_shell {
                let mut command = CommandBuilder::new(shell);
                command.args(interactive_shell_args(shell));
                command
            } else {
                command_execution_builder(shell, program, args)
            }
        }
        TerminalExecutionSpec::Ssh { .. } => return Err(HostedTerminalError::UnsupportedExecution),
    };
    command.cwd(&spec.cwd);
    command.env("TERM", "xterm-256color");
    command.env("COLORTERM", "truecolor");
    command.env("TERM_PROGRAM", "yttt");
    command.env("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION"));
    #[cfg(unix)]
    if ["LC_ALL", "LC_CTYPE", "LANG"]
        .iter()
        .all(|name| command.get_env(name).is_none_or(|value| value.is_empty()))
    {
        command.env("LANG", "C.UTF-8");
    }
    for name in &spec.removed_environment {
        command.env_remove(name);
    }
    for (name, value) in &spec.environment {
        command.env(name, value);
    }
    if spec.environment.iter().any(|(name, _)| name == "NO_COLOR")
        && !spec.environment.iter().any(|(name, _)| name == "CLICOLOR")
    {
        command.env_remove("CLICOLOR");
    } else if !spec.environment.iter().any(|(name, _)| name == "CLICOLOR") {
        command.env("CLICOLOR", "1");
    }
    Ok(command)
}

fn command_execution_builder(shell: &str, program: &str, args: &[String]) -> CommandBuilder {
    #[cfg(unix)]
    {
        let shell_name = shell_executable_name(shell);
        if matches!(shell_name.as_str(), "fish" | "fish.exe") {
            let mut command = CommandBuilder::new(shell);
            command.args(["-lic", "exec $argv", "--", program]);
            command.args(args);
            return command;
        }
        if matches!(
            shell_name.as_str(),
            "sh" | "sh.exe"
                | "ash"
                | "ash.exe"
                | "bash"
                | "bash.exe"
                | "dash"
                | "dash.exe"
                | "ksh"
                | "ksh.exe"
                | "mksh"
                | "mksh.exe"
                | "zsh"
                | "zsh.exe"
        ) {
            let mut command = CommandBuilder::new(shell);
            command.args(["-lic", "exec \"$@\"", "yttt-command", program]);
            command.args(args);
            return command;
        }
    }
    let mut command = CommandBuilder::new(program);
    command.args(args);
    command
}

fn shell_executable_name(shell: &str) -> String {
    shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase()
}

fn interactive_shell_args(shell: &str) -> Vec<String> {
    let shell_name = shell_executable_name(shell);
    if matches!(shell_name.as_str(), "cmd" | "cmd.exe") {
        vec!["/D".to_string()]
    } else if matches!(
        shell_name.as_str(),
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    ) {
        vec!["-NoLogo".to_string()]
    } else if matches!(
        shell_name.as_str(),
        "sh" | "sh.exe"
            | "ash"
            | "ash.exe"
            | "bash"
            | "bash.exe"
            | "dash"
            | "dash.exe"
            | "fish"
            | "fish.exe"
            | "ksh"
            | "ksh.exe"
            | "mksh"
            | "mksh.exe"
            | "zsh"
            | "zsh.exe"
    ) {
        vec!["-li".to_string()]
    } else {
        Vec::new()
    }
}

fn startup_command(spec: &TerminalSpawnSpec) -> Option<String> {
    let command = match &spec.execution {
        TerminalExecutionSpec::Shell {
            initial_command, ..
        } => initial_command.clone()?,
        TerminalExecutionSpec::Command {
            program,
            args,
            return_to_shell: true,
            ..
        } => shell_words::join(
            std::iter::once(program.as_str()).chain(args.iter().map(String::as_str)),
        ),
        TerminalExecutionSpec::Command {
            return_to_shell: false,
            ..
        }
        | TerminalExecutionSpec::Ssh { .. } => return None,
    };
    Some(format!("{command}\r"))
}
