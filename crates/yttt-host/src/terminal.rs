use std::{
    collections::{BTreeMap, VecDeque},
    io::{Read, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

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
    RemoteTerminalExecutionSpec, SemanticViewport, TerminalCheckpoint, TerminalExecutionSpec,
    TerminalGeometry, TerminalProcessState, TerminalSpawnSpec, TerminalStreamUpdate,
};
use yttt_ssh::{
    RemoteTerminalExecution, RemoteTerminalRequest, RemoteTerminalResizeHandle,
    RemoteTerminalSession, TransportService,
};
use yttt_terminal_core::{
    TerminalParser, TerminalState,
    semantic::{SemanticCaptureContext, SemanticSnapshotter},
};

const RAW_REPLAY_BYTES: usize = 8 * 1024 * 1024;
const WRITER_QUEUE_CAPACITY: usize = 1024;
const EVENT_QUEUE_CAPACITY: usize = 256;
const SUBSCRIBER_CAPACITY: usize = 64;
const OUTPUT_CAPTURE_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostTerminalEvent {
    Update {
        session_id: TerminalSessionId,
        update: TerminalStreamUpdate,
    },
    Exited {
        session_id: TerminalSessionId,
        session_epoch: u64,
        code: Option<i32>,
    },
    TitleChanged {
        session_id: TerminalSessionId,
        title: Option<String>,
    },
    Bell {
        session_id: TerminalSessionId,
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
    #[error("terminal I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("terminal PTY failed: {0}")]
    Pty(#[from] anyhow::Error),
}

#[derive(Clone)]
struct HostEventProxy {
    events: flume::Sender<Event>,
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
            let _ = self.events.send(event);
        } else {
            let _ = self.events.try_send(event);
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
    Input(Vec<u8>),
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
            bytes: VecDeque::with_capacity(RAW_REPLAY_BYTES),
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
        let (terminal_event_tx, terminal_event_rx) = flume::bounded(EVENT_QUEUE_CAPACITY);
        let event_proxy = HostEventProxy {
            events: terminal_event_tx,
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
                palette_revision: 0,
                title: None,
                cwd: Some(spec.cwd.clone()),
                process_state: TerminalProcessState::Running,
            }),
            query_palette: Mutex::new(Vec::new()),
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
        self.inner
            .writer
            .try_send(WriterCommand::Input(bytes))
            .map_err(|error| match error {
                flume::TrySendError::Full(_) => HostedTerminalError::Backpressure,
                flume::TrySendError::Disconnected(_) => HostedTerminalError::Stopped,
            })
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

    pub fn terminate(&self) -> Result<(), HostedTerminalError> {
        if self.inner.stopped.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        let _ = self.inner.writer.try_send(WriterCommand::Shutdown);
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

    fn capture_and_publish(&self) {
        let context = self.metadata.lock().capture_context();
        let update = {
            let state = self.state.lock();
            self.snapshots.lock().capture(&state, &context)
        };
        let _ = self.events.send(HostTerminalEvent::Update {
            session_id: self.spec.session_id.clone(),
            update,
        });
    }

    fn mark_exited(&self, code: Option<i32>) {
        if self.exited.swap(true, Ordering::AcqRel) {
            return;
        }
        self.stopped.store(true, Ordering::Release);
        self.metadata.lock().process_state = TerminalProcessState::Exited { code };
        self.capture_and_publish();
        let _ = self.events.send(HostTerminalEvent::Exited {
            session_id: self.spec.session_id.clone(),
            session_epoch: self.session_epoch,
            code,
        });
        let _ = self.writer.try_send(WriterCommand::Shutdown);
    }
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
                        parser.advance(bytes);
                        inner.request_output_capture();
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break,
                }
            }
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
            inner.capture_and_publish();
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
                let bytes = match command {
                    WriterCommand::Input(bytes) | WriterCommand::Reply(bytes) => bytes,
                    WriterCommand::Shutdown => break,
                };
                if writer.write_all(&bytes).is_err() || writer.flush().is_err() {
                    break;
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
                match event {
                    Event::PtyWrite(data) => {
                        let _ = inner.writer.send(WriterCommand::Reply(data.into_bytes()));
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
                        let _ = inner.writer.send(WriterCommand::Reply(reply.into_bytes()));
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
                            let _ = inner.writer.send(WriterCommand::Reply(reply.into_bytes()));
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
                        inner.mark_exited(i32::try_from(status.exit_code()).ok());
                        break;
                    }
                    Ok(None) => thread::sleep(Duration::from_millis(25)),
                    Err(_) => {
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
