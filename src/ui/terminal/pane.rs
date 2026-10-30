use std::{
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU8, AtomicU64, Ordering},
    },
};

use crate::{
    host_runtime::{DesktopHostRuntime, HostRuntimeGlobal, TerminalPaneHostEvent},
    model::{
        layout::{PaneConfig, PaneKind},
        workspace::PaneProcessState,
    },
    runtime::{
        agent::classify_agent,
        agent_manager::AgentPaneAddress,
        notification::{ExitNotificationInput, NotificationEvent, notification_for_exit},
    },
    ui::{
        i18n::{Locale, UiText, UiTextKey},
        interaction::input_owner::TerminalInputGate,
        theme::{WorkbenchTheme, current_ui_style},
    },
};
use gpui::{
    AnyWindowHandle, Context, Entity, EventEmitter, IntoElement, Render, SharedString,
    Subscription, Task, Window, div, prelude::*,
};
use parking_lot::RwLock;
use yttt_client_core::{ClientCoreError, ClientEvent, ConnectionState, TerminalMirrorMetadata};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::{
    Request, Response, ServerEvent, TerminalPlacement,
    terminal::{
        AttachTerminal, ResizeTerminal, ScrollTerminal, TerminalGeometry, TerminalInput,
        TerminalLeaseMode, TerminalMutationContext, TerminalProcessState, TerminationMode,
    },
};
use yttt_terminal::{ExitReason, ProcessStatus, PtyIoOperation, TerminalConfig, TerminalView};

#[derive(Clone, Debug)]
pub struct TerminalPaneContext {
    pub project_id: String,
    pub project_path: PathBuf,
    pub project_title: String,
    pub tab_id: String,
    pub tab_title: String,
    pub pane: PaneConfig,
    pub is_focused: bool,
    pub terminal_input_gate: TerminalInputGate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalPaneEvent {
    Notification(NotificationEvent),
    StartFailed(TerminalPaneStartFailedEvent),
    Lost(TerminalPaneLostEvent),
    RestartRequested(TerminalPaneRestartRequest),
    AgentStatusFrame {
        pane_id: String,
        frame: String,
    },
    TitleChanged {
        pane_id: String,
        title: String,
    },
    ViewportSizeChanged {
        pane_id: String,
    },
    IoError {
        pane_id: String,
        message: String,
        fatal: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneStartFailedEvent {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneLostEvent {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub message: String,
}

/// The user asked the Host to start an exited pane again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneRestartRequest {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub resume: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneLifecycle {
    Idle,
    Starting,
    Running,
    Stopping {
        reason: ExitReason,
    },
    Exited {
        code: Option<i32>,
        reason: ExitReason,
    },
    SpawnFailed {
        message: String,
    },
    Lost {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalSpawnFailure {
    pub command: String,
    pub cwd: PathBuf,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneExitInput {
    pub project_id: String,
    pub project_title: String,
    pub tab_id: String,
    pub tab_title: String,
    pub pane_id: String,
    pub pane_title: String,
    pub command: String,
    pub kind: PaneKind,
    pub notify_on_exit: bool,
    pub status: ProcessStatus,
    pub exit_reason: ExitReason,
}

pub struct TerminalPaneView {
    project_id: String,
    project_path: PathBuf,
    project_title: String,
    tab_id: String,
    tab_title: String,
    pane_id: String,
    default_title: String,
    title: String,
    command: String,
    kind: PaneKind,
    notify_on_exit: bool,
    agent_session_title: Option<String>,
    terminal: Option<Entity<TerminalView>>,
    terminal_viewport_subscription: Option<Subscription>,
    terminal_viewport_size: Option<(usize, usize)>,
    terminal_config: TerminalConfig,
    theme: WorkbenchTheme,
    host_runtime: Option<Arc<DesktopHostRuntime>>,
    control_writer: Option<HostTerminalWriter>,
    host_session_id: Option<TerminalSessionId>,
    host_epoch: Option<u64>,
    host_session_epoch: Option<u64>,
    /// The pane's process state in the Host layout.
    host_process_state: PaneProcessState,
    /// The pane's Agent holds a session a restart can resume.
    resumable: bool,
    /// An attach request is in flight.
    attaching: bool,
    /// The `(host_epoch, session_epoch)` of the terminal whose exit this pane
    /// already showed; the pane attaches only to a later one.
    finished_session: Option<(u64, u64)>,
    host_events_task: Option<Task<()>>,
    window_handle: Option<AnyWindowHandle>,
    lifecycle: PaneLifecycle,
    terminal_error: Option<String>,
    exit_emitted: bool,
    terminal_input_gate: TerminalInputGate,
    generation: u64,
    #[cfg(feature = "perf-metrics")]
    performance_reporter: Option<yttt_terminal::TerminalPerformanceReporter>,
    #[cfg(feature = "perf-metrics")]
    performance_probe_started: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalPaneChrome {
    pub shows_header: bool,
}

fn resolved_terminal_title(default_title: &str, title: &str) -> String {
    if title.is_empty() {
        default_title.to_string()
    } else {
        title.to_string()
    }
}
fn terminal_io_error_message(text: UiText, operation: PtyIoOperation, message: &str) -> String {
    let operation = text.get(match operation {
        PtyIoOperation::Read => UiTextKey::TerminalI18nIoRead,
        PtyIoOperation::Write => UiTextKey::TerminalI18nIoWrite,
        PtyIoOperation::Resize => UiTextKey::TerminalI18nIoResize,
        PtyIoOperation::Mailbox => UiTextKey::TerminalI18nIoMailbox,
        PtyIoOperation::InputQueue => UiTextKey::TerminalI18nIoInputQueue,
        PtyIoOperation::Clipboard => UiTextKey::TerminalI18nIoClipboard,
    });
    text.get(UiTextKey::TerminalI18nIoError)
        .replace("{operation}", operation)
        .replace("{message}", message)
}

fn accepts_process_exit(
    active_generation: u64,
    callback_generation: u64,
    exit_emitted: bool,
) -> bool {
    active_generation == callback_generation && !exit_emitted
}

#[derive(Clone)]
struct HostTerminalWriter {
    runtime: Arc<DesktopHostRuntime>,
    session_id: TerminalSessionId,
    mutation_context: Arc<RwLock<TerminalMutationContext>>,
    next_sequence: Arc<AtomicU64>,
    locale: Arc<AtomicU8>,
}

fn next_mutation_context(
    context: &Arc<RwLock<TerminalMutationContext>>,
    next_sequence: &Arc<AtomicU64>,
    geometry_epoch: Option<u64>,
) -> TerminalMutationContext {
    let mut context = context.write();
    if let Some(geometry_epoch) = geometry_epoch {
        context.geometry_epoch = geometry_epoch;
    }
    context.client_sequence = next_sequence.fetch_add(1, Ordering::Relaxed);
    *context
}

fn submit_pending_host_resize(
    runtime: &Arc<DesktopHostRuntime>,
    session_id: &TerminalSessionId,
    mutation_context: &Arc<RwLock<TerminalMutationContext>>,
    next_mutation_sequence: &Arc<AtomicU64>,
    next_geometry_epoch: &Arc<AtomicU64>,
    pending_geometry: &Arc<RwLock<Option<TerminalGeometry>>>,
) {
    if !runtime.is_connected() {
        pending_geometry.write().take();
        return;
    }
    let Some(geometry) = pending_geometry.write().take() else {
        return;
    };
    let geometry_epoch = next_geometry_epoch.fetch_add(1, Ordering::Relaxed) + 1;
    let context = next_mutation_context(
        mutation_context,
        next_mutation_sequence,
        Some(geometry_epoch),
    );
    let _ = runtime.request(Request::ResizeTerminal(ResizeTerminal {
        session_id: session_id.clone(),
        context,
        geometry,
    }));
}
impl HostTerminalWriter {
    fn ui_text(&self) -> UiText {
        UiText::new(match self.locale.load(Ordering::Relaxed) {
            1 => Locale::Chinese,
            _ => Locale::English,
        })
    }
}

impl Write for HostTerminalWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let text = self.ui_text();
        if !matches!(self.runtime.state(), ConnectionState::Ready { .. }) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                text.get(UiTextKey::TerminalI18nHostConnectionNotReady),
            ));
        }
        let context = next_mutation_context(&self.mutation_context, &self.next_sequence, None);
        self.runtime
            .send_terminal_input(TerminalInput {
                session_id: self.session_id.clone(),
                context,
                bytes: bytes.to_vec(),
            })
            .map_err(|error| {
                let kind = match error {
                    ClientCoreError::Backpressure => io::ErrorKind::WouldBlock,
                    ClientCoreError::NotConnected => io::ErrorKind::NotConnected,
                    _ => io::ErrorKind::BrokenPipe,
                };
                io::Error::new(kind, error.to_string())
            })?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TerminalPaneView {
    pub(crate) fn control_state(&self) -> String {
        match &self.lifecycle {
            PaneLifecycle::Idle => "idle",
            PaneLifecycle::Starting => "starting",
            PaneLifecycle::Running => "running",
            PaneLifecycle::Stopping { .. } => "stopping",
            PaneLifecycle::Exited { .. } => "exited",
            PaneLifecycle::SpawnFailed { .. } => "failed",
            PaneLifecycle::Lost { .. } => "lost",
        }
        .to_string()
    }

    pub(crate) fn control_input(
        &self,
        text: &str,
        enter: bool,
        raw: bool,
        cx: &gpui::App,
    ) -> Result<(flume::Receiver<Result<Response, ClientCoreError>>, usize), String> {
        let ui_text = UiText::current(cx);
        if self.lifecycle != PaneLifecycle::Running {
            return Err(ui_text
                .get(UiTextKey::TerminalI18nInputTerminalNotRunning)
                .to_string());
        }
        let writer = self.control_writer.as_ref().ok_or_else(|| {
            ui_text
                .get(UiTextKey::TerminalI18nInputNoHostChannel)
                .to_string()
        })?;
        if !writer.runtime.is_connected() || writer.mutation_context.read().lease_epoch == 0 {
            return Err(ui_text
                .get(UiTextKey::TerminalI18nInputControlUnavailable)
                .to_string());
        }
        let terminal = self.terminal.as_ref().ok_or_else(|| {
            ui_text
                .get(UiTextKey::TerminalI18nInputViewNotReady)
                .to_string()
        })?;
        let mut bytes = if raw {
            text.as_bytes().to_vec()
        } else {
            terminal.read(cx).encode_paste(text)
        };
        if enter {
            bytes.push(b'\r');
        }
        if bytes.is_empty() {
            return Err(ui_text.get(UiTextKey::TerminalI18nInputEmpty).to_string());
        }
        let count = bytes.len();
        let context = next_mutation_context(&writer.mutation_context, &writer.next_sequence, None);
        let response = writer
            .runtime
            .request(Request::TerminalInput(TerminalInput {
                session_id: writer.session_id.clone(),
                context,
                bytes,
            }));
        Ok((response, count))
    }

    pub fn new(
        context: TerminalPaneContext,
        terminal_config: TerminalConfig,
        theme: WorkbenchTheme,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new_deferred(context, terminal_config, theme);
        view.start_terminal(window, cx);
        view
    }

    pub(crate) fn new_without_processes(
        context: TerminalPaneContext,
        terminal_config: TerminalConfig,
        theme: WorkbenchTheme,
        cx: &mut Context<Self>,
    ) -> Self {
        let mut view = Self::new_deferred(context, terminal_config, theme);
        view.start_idle_terminal(cx);
        view
    }

    pub(crate) fn new_deferred(
        context: TerminalPaneContext,
        terminal_config: TerminalConfig,
        theme: WorkbenchTheme,
    ) -> Self {
        let TerminalPaneContext {
            project_id,
            project_path,
            project_title,
            tab_id,
            tab_title,
            pane,
            is_focused: _,
            terminal_input_gate,
        } = context;
        Self {
            project_id,
            project_path,
            project_title,
            tab_id,
            tab_title,
            pane_id: pane.id,
            title: pane.title.clone(),
            default_title: pane.title,
            command: pane.command,
            kind: pane.kind,
            notify_on_exit: pane.notify_on_exit,
            agent_session_title: None,
            terminal: None,
            terminal_viewport_subscription: None,
            terminal_viewport_size: None,
            terminal_config,
            theme,
            host_runtime: None,
            control_writer: None,
            host_session_id: None,
            host_epoch: None,
            host_session_epoch: None,
            host_process_state: PaneProcessState::Idle,
            resumable: false,
            attaching: false,
            finished_session: None,
            host_events_task: None,
            window_handle: None,
            lifecycle: PaneLifecycle::Idle,
            terminal_error: None,
            exit_emitted: false,
            terminal_input_gate,
            generation: 0,
            #[cfg(feature = "perf-metrics")]
            performance_reporter: None,
            #[cfg(feature = "perf-metrics")]
            performance_probe_started: false,
        }
    }

    fn start_idle_terminal(&mut self, cx: &mut Context<Self>) {
        let terminal_input_allowed = self.terminal_input_gate.shared_flag();
        let config = self.terminal_config.clone();
        let terminal = cx.new(|cx| {
            TerminalView::new_semantic(std::io::sink(), config, cx)
                .with_key_handler(move |_event| !terminal_input_allowed.load(Ordering::SeqCst))
        });
        self.track_terminal_viewport(terminal, cx);
        self.lifecycle = PaneLifecycle::Running;
        cx.notify();
    }

    fn set_runtime_title(&mut self, title: String, cx: &mut Context<Self>) {
        if self.title == title {
            return;
        }
        self.title = title.clone();
        cx.emit(TerminalPaneEvent::TitleChanged {
            pane_id: self.pane_id.clone(),
            title,
        });
        cx.notify();
    }

    pub(crate) fn set_agent_session_title(
        &mut self,
        provider_display_name: &str,
        title: Option<&str>,
        cx: &mut Context<Self>,
    ) {
        let default_is_generic = self.default_title.eq_ignore_ascii_case("shell")
            || self.default_title.eq_ignore_ascii_case("terminal")
            || self
                .default_title
                .eq_ignore_ascii_case(provider_display_name);
        if self.agent_session_title.is_none() && !default_is_generic {
            return;
        }
        let title = title
            .map(str::trim)
            .filter(|title| !title.is_empty())
            .map(ToOwned::to_owned);
        if self.agent_session_title == title {
            return;
        }
        self.agent_session_title = title;
        self.set_runtime_title(
            self.agent_session_title
                .clone()
                .unwrap_or_else(|| self.default_title.clone()),
            cx,
        );
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn agent_pane_address(&self) -> AgentPaneAddress {
        AgentPaneAddress::new(&self.project_id, &self.tab_id, &self.pane_id)
    }

    pub fn matches_agent_pane_address(&self, address: &AgentPaneAddress) -> bool {
        self.project_id == address.project_id
            && self.tab_id == address.tab_id
            && self.pane_id == address.pane_id
    }

    fn session_id(&self) -> TerminalSessionId {
        TerminalSessionId::new(format!(
            "{}:{}:{}",
            self.project_id, self.tab_id, self.pane_id
        ))
    }

    /// Shows the pane's Host terminal. The Host starts, restores and restarts
    /// pane processes; the pane attaches once the Host lists the terminal.
    pub(crate) fn start_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.window_handle = Some(window.window_handle());
        let ui_text = UiText::current(cx);
        let Some(host_runtime) = cx
            .try_global::<HostRuntimeGlobal>()
            .and_then(HostRuntimeGlobal::runtime)
            .cloned()
        else {
            self.set_spawn_failure(
                ui_text
                    .get(UiTextKey::TerminalI18nHostRuntimeUnavailable)
                    .to_string(),
                cx,
            );
            return false;
        };
        if self.host_events_task.is_none() {
            self.host_events_task = Some(self.follow_host_events(&host_runtime, window, cx));
        }
        self.host_runtime = Some(host_runtime.clone());
        if !matches!(host_runtime.state(), ConnectionState::Ready { .. }) {
            self.set_terminal_lost(
                ui_text
                    .get(UiTextKey::TerminalI18nHostRuntimeNotConnected)
                    .to_string(),
                cx,
            );
            return false;
        }
        self.sync_host_terminal(window, cx);
        true
    }

    /// Follows the Host: attaches to a terminal of this pane it has not
    /// finished yet, reattaches when workspace control changes and otherwise
    /// shows whether the Host is starting the pane or left it exited.
    fn sync_host_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.attaching {
            return;
        }
        let Some(runtime) = self.host_runtime.clone() else {
            return;
        };
        let ConnectionState::Ready { host_epoch, .. } = runtime.state() else {
            return;
        };
        let session_id = self.session_id();
        let placement = runtime
            .resource_catalog()
            .filter(|catalog| catalog.host_epoch == host_epoch)
            .and_then(|catalog| {
                catalog
                    .terminals
                    .iter()
                    .find(|placement| placement.session_id == session_id)
                    .cloned()
            })
            .filter(|placement| {
                self.finished_session != Some((host_epoch, placement.session_epoch))
            });
        match (&self.lifecycle, placement) {
            (PaneLifecycle::Running | PaneLifecycle::Stopping { .. }, _) => {}
            (_, Some(placement)) => {
                self.attach_host_terminal(runtime, host_epoch, placement, window, cx);
            }
            (PaneLifecycle::Exited { .. }, None) => {}
            (_, None) if self.host_process_state == PaneProcessState::Exited => {
                self.show_host_exit(cx);
            }
            // A terminal vanished while running: keep showing the loss until the
            // Host starts the pane again or records its exit.
            (PaneLifecycle::Starting | PaneLifecycle::Lost { .. }, None) => {}
            (_, None) => self.wait_for_host_terminal(cx),
        }
    }

    /// Records a granted (`lease_epoch > 0`) or revoked (`0`) Interactive lease:
    /// input and resize need one.
    fn set_interactive_lease(&mut self, lease_epoch: u64, cx: &mut Context<Self>) {
        if let Some(writer) = &self.control_writer {
            writer.mutation_context.write().lease_epoch = lease_epoch;
        }
        if let Some(terminal) = &self.terminal {
            terminal.update(cx, |terminal, cx| {
                terminal.set_read_only(lease_epoch == 0, cx)
            });
        }
    }

    fn attach_host_terminal(
        &mut self,
        host_runtime: Arc<DesktopHostRuntime>,
        host_epoch: u64,
        placement: TerminalPlacement,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ui_text = UiText::current(cx);
        let initial_title = self
            .agent_session_title
            .clone()
            .unwrap_or_else(|| self.default_title.clone());
        self.set_runtime_title(initial_title, cx);
        let was_focused = self.terminal_is_focused(window, cx);
        self.clear_terminal_viewport();
        self.attaching = true;
        self.lifecycle = PaneLifecycle::Starting;
        self.terminal_error = None;
        self.exit_emitted = false;
        self.generation = self.generation.wrapping_add(1);
        #[cfg(feature = "perf-metrics")]
        {
            self.performance_reporter = None;
            self.performance_probe_started = false;
        }

        let session_id = placement.session_id.clone();
        let generation = self.generation;
        let geometry = TerminalGeometry {
            cols: self.terminal_config.cols.min(u16::MAX as usize) as u16,
            rows: self.terminal_config.rows.min(u16::MAX as usize) as u16,
            cell_width: 0,
            cell_height: 0,
        };
        let geometry_epoch = placement.geometry_epoch.saturating_add(1).max(1);
        let next_mutation_sequence = Arc::new(AtomicU64::new(1));
        let next_geometry_epoch = Arc::new(AtomicU64::new(geometry_epoch));
        let mutation_context = Arc::new(RwLock::new(TerminalMutationContext {
            host_epoch,
            session_epoch: 0,
            lease_epoch: 0,
            geometry_epoch,
            client_sequence: 0,
        }));
        let pending_resize_geometry = Arc::new(RwLock::new(None::<TerminalGeometry>));
        let resize_geometry_epoch = next_geometry_epoch.clone();
        let writer = HostTerminalWriter {
            runtime: host_runtime.clone(),
            session_id: session_id.clone(),
            mutation_context: mutation_context.clone(),
            next_sequence: next_mutation_sequence.clone(),
            locale: Arc::new(AtomicU8::new(if ui_text.locale() == Locale::Chinese {
                1
            } else {
                0
            })),
        };
        self.control_writer = Some(writer.clone());
        let terminal_input_allowed = self.terminal_input_gate.shared_flag();
        let resize_runtime = host_runtime.clone();
        let resize_session_id = session_id.clone();
        let resize_mutation_context = mutation_context.clone();
        let resize_mutation_sequence = next_mutation_sequence.clone();
        let resize_pending_geometry = pending_resize_geometry.clone();
        let scroll_runtime = host_runtime.clone();
        let scroll_session_id = session_id.clone();
        let scroll_mutation_context = mutation_context.clone();
        let scroll_mutation_sequence = next_mutation_sequence.clone();
        let resize_text = ui_text;
        let error_parent = cx.weak_entity();
        let input_runtime = host_runtime.clone();
        let input_mutation_context = mutation_context.clone();
        let initial_config = self.terminal_config.clone();
        let terminal = cx.new(|cx| {
            TerminalView::new_semantic(writer, initial_config, cx)
                .with_key_handler(move |_event| {
                    !input_runtime.is_connected()
                        || input_mutation_context.read().lease_epoch == 0
                        || !terminal_input_allowed.load(Ordering::SeqCst)
                })
                .with_resize_callback(move |cols, rows, cell_width, cell_height| {
                    if !matches!(resize_runtime.state(), ConnectionState::Ready { .. }) {
                        return Err(resize_text
                            .get(UiTextKey::TerminalI18nHostConnectionNotReady)
                            .to_string());
                    }
                    *resize_pending_geometry.write() = Some(TerminalGeometry {
                        cols,
                        rows,
                        cell_width,
                        cell_height,
                    });
                    if resize_mutation_context.read().lease_epoch == 0 {
                        return Ok(());
                    }
                    submit_pending_host_resize(
                        &resize_runtime,
                        &resize_session_id,
                        &resize_mutation_context,
                        &resize_mutation_sequence,
                        &resize_geometry_epoch,
                        &resize_pending_geometry,
                    );
                    Ok(())
                })
                .with_semantic_scroll_callback(move |display_offset, geometry_epoch| {
                    if matches!(scroll_runtime.state(), ConnectionState::Ready { .. }) {
                        let mut context = next_mutation_context(
                            &scroll_mutation_context,
                            &scroll_mutation_sequence,
                            None,
                        );
                        context.geometry_epoch = geometry_epoch;
                        let _ = scroll_runtime.request(Request::ScrollTerminal(ScrollTerminal {
                            session_id: scroll_session_id.clone(),
                            context,
                            display_offset,
                        }));
                    }
                })
                .with_io_error_callback(move |cx, operation, message, fatal| {
                    let message =
                        terminal_io_error_message(UiText::current(cx), operation, message);
                    let _ = error_parent.update(cx, |pane, cx| {
                        pane.terminal_error = Some(message.clone());
                        cx.emit(TerminalPaneEvent::IoError {
                            pane_id: pane.pane_id.clone(),
                            message,
                            fatal,
                        });
                        cx.notify();
                    });
                })
        });
        #[cfg(feature = "perf-metrics")]
        if self.performance_metrics_enabled() {
            self.performance_reporter = terminal
                .read(cx)
                .performance_handle()
                .spawn_reporter_from_env()
                .unwrap_or_else(|error| {
                    eprintln!("failed to start Host terminal performance reporter: {error}");
                    None
                });
        }
        let terminal_updates = host_runtime.terminal_updates(session_id.clone());
        terminal.update(cx, |terminal, cx| {
            // Read-only until the Host grants this Client the Interactive lease.
            terminal.set_read_only(true, cx);
            terminal.attach_semantic_viewport_stream(terminal_updates, cx);
        });

        self.track_terminal_viewport(terminal, cx);
        if was_focused {
            self.focus_terminal(window, cx);
        }
        self.host_session_id = Some(session_id.clone());
        self.host_epoch = Some(host_epoch);
        self.host_session_epoch = None;
        #[cfg(feature = "perf-metrics")]
        self.start_performance_probe(window.window_handle(), cx);

        // The Host grants an Observer lease while another Client holds the
        // Interactive one.
        let mode = TerminalLeaseMode::Interactive;
        let response = host_runtime.request(Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: Some(placement.session_epoch),
            after_sequence: (placement.last_sequence > 0).then_some(placement.last_sequence),
            mode,
            geometry,
            geometry_epoch,
            query_palette: self.terminal_config.colors.query_palette(),
            palette_revision: 1,
        }));
        let request_runtime = host_runtime;
        cx.spawn_in(window, async move |this, cx| {
            let result = match response.recv_async().await {
                Ok(result) => result.map_err(|error| error.to_string()),
                Err(_) => Err(ui_text
                    .get(UiTextKey::TerminalI18nHostTerminalRequestClosed)
                    .to_string()),
            };
            let _ = this.update_in(cx, |pane, window, cx| {
                if pane.generation != generation {
                    return;
                }
                pane.attaching = false;
                match result {
                    Ok(Response::TerminalAttached { lease, checkpoint })
                        if lease.session_id == session_id =>
                    {
                        let session_epoch = checkpoint.viewport.session_epoch;
                        next_geometry_epoch
                            .store(checkpoint.viewport.geometry_epoch, Ordering::Relaxed);
                        {
                            let mut context = mutation_context.write();
                            context.session_epoch = session_epoch;
                            context.lease_epoch = lease.lease_epoch;
                            context.geometry_epoch = checkpoint.viewport.geometry_epoch;
                        }
                        submit_pending_host_resize(
                            &request_runtime,
                            &session_id,
                            &mutation_context,
                            &next_mutation_sequence,
                            &next_geometry_epoch,
                            &pending_resize_geometry,
                        );
                        pane.host_session_epoch = Some(session_epoch);
                        let process_state = checkpoint.viewport.process_state;
                        let final_sequence = checkpoint.viewport.sequence;
                        if let Some(terminal) = pane.terminal.clone() {
                            terminal.update(cx, |terminal, cx| {
                                terminal.set_read_only(
                                    lease.mode != TerminalLeaseMode::Interactive,
                                    cx,
                                );
                                terminal.set_semantic_viewport(checkpoint.viewport, cx);
                            });
                        }
                        if let TerminalProcessState::Exited { code } = process_state {
                            pane.finalize_host_process_exit(
                                &request_runtime,
                                session_epoch,
                                final_sequence,
                                code,
                                cx,
                            );
                            return;
                        }
                        pane.lifecycle = PaneLifecycle::Running;
                        cx.notify();
                        // Metadata may arrive before the reply establishes the session
                        // epoch. Reconcile it now so a short-lived process still
                        // finalizes its exit.
                        if let Some(metadata) = request_runtime.terminal_metadata(&session_id) {
                            pane.handle_host_viewport_metadata(&metadata, &request_runtime, cx);
                        }
                        pane.sync_host_terminal(window, cx);
                    }
                    Ok(response) => {
                        let message = ui_text
                            .get(UiTextKey::TerminalI18nHostTerminalResponseUnexpected)
                            .replace("{response}", &format!("{response:?}"));
                        pane.attach_failed(message, window, cx);
                    }
                    Err(message) => pane.attach_failed(message, window, cx),
                }
            });
        })
        .detach();
    }

    /// A terminal that left the catalog meanwhile is waited for again; any
    /// other failure leaves the pane lost until the Host lists it anew.
    fn attach_failed(&mut self, message: String, window: &mut Window, cx: &mut Context<Self>) {
        let session_id = self.session_id();
        let listed = self
            .host_runtime
            .as_ref()
            .and_then(|runtime| runtime.resource_catalog())
            .is_some_and(|catalog| {
                catalog
                    .terminals
                    .iter()
                    .any(|placement| placement.session_id == session_id)
            });
        if listed {
            self.set_terminal_lost(message, cx);
        } else {
            self.wait_for_host_terminal(cx);
            self.sync_host_terminal(window, cx);
        }
    }

    fn follow_host_events(
        &self,
        runtime: &Arc<DesktopHostRuntime>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Task<()> {
        let host_events = runtime.terminal_pane_events(self.session_id());
        let runtime = runtime.clone();
        cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = host_events.recv_async().await {
                let runtime = runtime.clone();
                let result = match event {
                    TerminalPaneHostEvent::Metadata(metadata) => {
                        this.update_in(cx, move |pane, _window, cx| {
                            pane.handle_host_viewport_metadata(&metadata, &runtime, cx);
                        })
                    }
                    TerminalPaneHostEvent::Client(event) => {
                        this.update_in(cx, move |pane, window, cx| {
                            pane.handle_host_event(event, &runtime, window, cx);
                        })
                    }
                };
                if result.is_err() {
                    break;
                }
            }
        })
    }

    fn handle_host_viewport_metadata(
        &mut self,
        metadata: &TerminalMirrorMetadata,
        runtime: &DesktopHostRuntime,
        cx: &mut Context<Self>,
    ) {
        if self.host_session_epoch != Some(metadata.session_epoch) {
            return;
        }
        if self.agent_session_title.is_none()
            && let Some(title) = metadata.title.as_deref()
        {
            let title = resolved_terminal_title(&self.default_title, title);
            self.set_runtime_title(title, cx);
        }
        if let TerminalProcessState::Exited { code } = metadata.process_state {
            self.finalize_host_process_exit(
                runtime,
                metadata.session_epoch,
                metadata.sequence,
                code,
                cx,
            );
        }
    }

    fn handle_host_event(
        &mut self,
        event: ClientEvent,
        runtime: &DesktopHostRuntime,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let ui_text = UiText::current(cx);
        let session_id = self.session_id();
        match event {
            ClientEvent::TerminalUnavailable(unavailable) if unavailable == session_id => {
                // Removal events carry no session epoch: only a pane showing a
                // live terminal the catalog no longer lists has lost it.
                if self.lifecycle != PaneLifecycle::Running
                    || runtime.resource_catalog().is_some_and(|catalog| {
                        catalog
                            .terminals
                            .iter()
                            .any(|placement| placement.session_id == session_id)
                    })
                {
                    return;
                }
                self.set_terminal_lost(
                    ui_text
                        .get(UiTextKey::TerminalI18nSessionMissingFromCatalog)
                        .to_string(),
                    cx,
                );
            }
            ClientEvent::ResourceCatalogUpdated(_) => self.sync_host_terminal(window, cx),
            ClientEvent::Server(host_event) => match host_event.body {
                ServerEvent::TerminalExit {
                    session_id: exited_session_id,
                    session_epoch,
                    code,
                    final_sequence,
                } if exited_session_id == session_id => {
                    let final_viewport_is_ready = runtime
                        .terminal_metadata(&session_id)
                        .is_some_and(|metadata| {
                            metadata.session_epoch == session_epoch
                                && metadata.sequence >= final_sequence
                                && matches!(
                                    metadata.process_state,
                                    TerminalProcessState::Exited { .. }
                                )
                        });
                    if !final_viewport_is_ready {
                        return;
                    }
                    self.finalize_host_process_exit(
                        runtime,
                        session_epoch,
                        final_sequence,
                        code,
                        cx,
                    );
                }
                ServerEvent::TerminalLeaseRevoked {
                    session_id: revoked_session_id,
                    ..
                } if revoked_session_id == session_id => {
                    self.set_interactive_lease(0, cx);
                    self.terminal_error =
                        Some(ui_text.get(UiTextKey::TerminalI18nLeaseRevoked).to_string());
                    cx.notify();
                }
                ServerEvent::TerminalLeaseReleased {
                    session_id: released_session_id,
                    ..
                } if released_session_id == session_id => {
                    self.terminal_error = Some(
                        ui_text
                            .get(UiTextKey::TerminalI18nLeaseReleased)
                            .to_string(),
                    );
                    cx.notify();
                }
                ServerEvent::TerminalLeaseExpired {
                    session_id: expired_session_id,
                    ..
                } if expired_session_id == session_id => {
                    self.terminal_error =
                        Some(ui_text.get(UiTextKey::TerminalI18nLeaseExpired).to_string());
                    cx.notify();
                }
                ServerEvent::TerminalControlRequested {
                    session_id: requested_session_id,
                    requester,
                } if requested_session_id == session_id => {
                    self.terminal_error = Some(
                        ui_text
                            .get(UiTextKey::TerminalI18nControlRequested)
                            .replace("{client}", requester.as_str()),
                    );
                    cx.notify();
                }
                ServerEvent::TerminalControlGranted { lease } if lease.session_id == session_id => {
                    self.set_interactive_lease(lease.lease_epoch, cx);
                    self.terminal_error = None;
                    cx.notify();
                }
                _ => {}
            },
            ClientEvent::Connection(ConnectionState::HostLost { message }) => {
                self.set_terminal_lost(message, cx);
            }
            _ => {}
        }
    }

    fn finalize_host_process_exit(
        &mut self,
        runtime: &DesktopHostRuntime,
        session_epoch: u64,
        final_sequence: u64,
        code: Option<i32>,
        cx: &mut Context<Self>,
    ) {
        if self.exit_emitted || self.host_session_epoch != Some(session_epoch) {
            return;
        }
        let Some(session_id) = self.host_session_id.clone() else {
            return;
        };
        self.exit_emitted = true;
        if let Some(host_epoch) = self.host_epoch {
            self.finished_session = Some((host_epoch, session_epoch));
        }
        self.clear_terminal_viewport();
        self.unbind_host_terminal();
        self.finalize_process_exit(
            self.generation,
            ExitReason::Completed,
            ProcessStatus::Exited { code },
            cx,
        );
        if let Err(error) =
            runtime.acknowledge_terminal_exit(session_id, session_epoch, final_sequence)
        {
            eprintln!("failed to queue terminal exit acknowledgement: {error}");
        }
    }

    /// Takes the pane's process state from the Host layout and whether its
    /// Agent holds a session a restart can resume.
    pub(crate) fn sync_host_pane_state(
        &mut self,
        process_state: PaneProcessState,
        resumable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.host_process_state == process_state && self.resumable == resumable {
            return;
        }
        let restarted = self.host_process_state == PaneProcessState::Exited
            && process_state != PaneProcessState::Exited;
        self.host_process_state = process_state;
        self.resumable = resumable;
        if restarted && matches!(self.lifecycle, PaneLifecycle::Exited { .. }) {
            self.wait_for_host_terminal(cx);
        }
        self.sync_host_terminal(window, cx);
        cx.notify();
    }

    fn request_restart(&mut self, resume: bool, cx: &mut Context<Self>) {
        cx.emit(TerminalPaneEvent::RestartRequested(
            TerminalPaneRestartRequest {
                project_id: self.project_id.clone(),
                tab_id: self.tab_id.clone(),
                pane_id: self.pane_id.clone(),
                resume,
            },
        ));
    }

    fn set_spawn_failure(&mut self, message: String, cx: &mut Context<Self>) {
        self.generation = self.generation.wrapping_add(1);
        self.lifecycle = PaneLifecycle::SpawnFailed {
            message: message.clone(),
        };
        self.terminal_error = Some(message.clone());
        self.clear_terminal_viewport();
        self.unbind_host_terminal();
        self.attaching = false;
        self.host_events_task = None;
        self.host_runtime = None;
        cx.emit(TerminalPaneEvent::StartFailed(
            TerminalPaneStartFailedEvent {
                project_id: self.project_id.clone(),
                tab_id: self.tab_id.clone(),
                pane_id: self.pane_id.clone(),
                message,
            },
        ));
        cx.notify();
    }

    fn set_terminal_lost(&mut self, message: String, cx: &mut Context<Self>) {
        // Replies to an attach sent before the loss belong to the old binding.
        self.generation = self.generation.wrapping_add(1);
        self.lifecycle = PaneLifecycle::Lost {
            message: message.clone(),
        };
        self.terminal_error = Some(message.clone());
        self.clear_terminal_viewport();
        self.unbind_host_terminal();
        self.attaching = false;
        cx.emit(TerminalPaneEvent::Lost(TerminalPaneLostEvent {
            project_id: self.project_id.clone(),
            tab_id: self.tab_id.clone(),
            pane_id: self.pane_id.clone(),
            message,
        }));
        cx.notify();
    }

    /// Shows that the Host has not started the pane's terminal yet.
    fn wait_for_host_terminal(&mut self, cx: &mut Context<Self>) {
        self.clear_terminal_viewport();
        self.unbind_host_terminal();
        self.lifecycle = PaneLifecycle::Starting;
        self.terminal_error = None;
        cx.notify();
    }

    /// Shows a pane the Host left exited — its process ended while no Client
    /// watched, or did not start — without reporting a new exit.
    fn show_host_exit(&mut self, cx: &mut Context<Self>) {
        self.clear_terminal_viewport();
        self.unbind_host_terminal();
        self.terminal_error = None;
        self.lifecycle = PaneLifecycle::Exited {
            code: None,
            reason: ExitReason::Completed,
        };
        cx.notify();
    }

    fn unbind_host_terminal(&mut self) {
        self.control_writer = None;
        self.host_session_id = None;
        self.host_epoch = None;
        self.host_session_epoch = None;
    }

    fn handle_process_exit(
        &mut self,
        generation: u64,
        exit_reason: ExitReason,
        cx: &mut Context<Self>,
    ) {
        if !accepts_process_exit(self.generation, generation, self.exit_emitted) {
            return;
        }
        self.exit_emitted = true;
        self.clear_terminal_viewport();
        self.lifecycle = PaneLifecycle::Stopping {
            reason: exit_reason,
        };

        self.finalize_process_exit(
            generation,
            exit_reason,
            ProcessStatus::Exited { code: None },
            cx,
        );
    }

    fn finalize_process_exit(
        &mut self,
        generation: u64,
        exit_reason: ExitReason,
        status: ProcessStatus,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation {
            return;
        }
        let code = match status {
            ProcessStatus::Running => None,
            ProcessStatus::Exited { code } => code,
        };
        self.lifecycle = PaneLifecycle::Exited {
            code,
            reason: exit_reason,
        };
        if let Some(notification) = notification_for_terminal_pane_exit(TerminalPaneExitInput {
            project_id: self.project_id.clone(),
            project_title: self.project_title.clone(),
            tab_id: self.tab_id.clone(),
            tab_title: self.tab_title.clone(),
            pane_id: self.pane_id.clone(),
            pane_title: self.title.clone(),
            command: self.command.clone(),
            kind: self.kind.clone(),
            notify_on_exit: self.notify_on_exit,
            status,
            exit_reason,
        }) {
            cx.emit(TerminalPaneEvent::Notification(notification));
        }
        cx.notify();
    }

    pub(crate) fn terminate_after_fatal_io(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.lifecycle,
            PaneLifecycle::Starting | PaneLifecycle::Running
        ) {
            self.terminate_host_session();
            self.handle_process_exit(self.generation, ExitReason::Failed, cx);
        }
    }

    pub(crate) fn terminate_host_session(&mut self) {
        let session_id = self.host_session_id.take();
        if let (Some(runtime), Some(session_id), Some(host_epoch), Some(session_epoch)) = (
            &self.host_runtime,
            session_id,
            self.host_epoch,
            self.host_session_epoch,
        ) && runtime.is_connected()
        {
            let _ = runtime.request(Request::TerminateTerminal {
                session_id,
                host_epoch,
                session_epoch,
                mode: TerminationMode::Terminate,
            });
        }
        self.host_epoch = None;
        self.host_session_epoch = None;
    }

    pub fn is_running(&self) -> bool {
        self.lifecycle == PaneLifecycle::Running
    }

    pub fn terminal_exit(&self) -> Option<(Option<i32>, ExitReason)> {
        let PaneLifecycle::Exited { code, reason } = &self.lifecycle else {
            return None;
        };
        Some((*code, *reason))
    }

    pub fn terminal_viewport_size(&self) -> Option<(usize, usize)> {
        self.terminal_viewport_size
    }

    fn track_terminal_viewport(&mut self, terminal: Entity<TerminalView>, cx: &mut Context<Self>) {
        self.terminal_viewport_size = terminal.read(cx).viewport_dimensions();
        let pane_id = self.pane_id.clone();
        self.terminal_viewport_subscription =
            Some(cx.observe(&terminal, move |pane, terminal, cx| {
                let size = terminal.read(cx).viewport_dimensions();
                if pane.terminal_viewport_size != size {
                    pane.terminal_viewport_size = size;
                    cx.emit(TerminalPaneEvent::ViewportSizeChanged {
                        pane_id: pane_id.clone(),
                    });
                    cx.notify();
                }
            }));
        self.terminal = Some(terminal);
    }

    fn clear_terminal_viewport(&mut self) {
        self.terminal = None;
        self.terminal_viewport_subscription = None;
        self.terminal_viewport_size = None;
    }
    #[cfg(feature = "perf-metrics")]
    pub(crate) fn performance_terminal(&self) -> Option<Entity<TerminalView>> {
        self.terminal.clone()
    }

    pub fn focus_terminal(&self, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(terminal) = self.terminal.clone() else {
            return false;
        };

        cx.defer_in(window, move |_this, window, cx| {
            let focus_handle = terminal.read(cx).focus_handle().clone();
            focus_handle.focus(window, cx);
        });
        true
    }

    pub fn terminal_is_focused(&self, window: &Window, cx: &gpui::App) -> bool {
        self.terminal
            .as_ref()
            .is_some_and(|terminal| terminal.read(cx).focus_handle().is_focused(window))
    }

    pub fn terminal_vi_mode(&self, cx: &gpui::App) -> bool {
        self.terminal
            .as_ref()
            .is_some_and(|terminal| terminal.read(cx).is_vi_mode())
    }

    pub fn terminal_search_is_active(&self, cx: &gpui::App) -> bool {
        self.terminal
            .as_ref()
            .is_some_and(|terminal| terminal.read(cx).search_is_active())
    }

    pub fn set_terminal_vi_mode(&mut self, enabled: bool, cx: &mut Context<Self>) -> bool {
        let Some(terminal) = self.terminal.clone() else {
            return false;
        };
        terminal.update(cx, |terminal, terminal_cx| {
            terminal.set_vi_mode(enabled, terminal_cx)
        })
    }

    pub fn default_chrome() -> TerminalPaneChrome {
        TerminalPaneChrome {
            shows_header: false,
        }
    }

    pub fn update_terminal_config(&mut self, config: TerminalConfig, cx: &mut Context<Self>) {
        if let Some(terminal) = &self.terminal {
            terminal.update(cx, |terminal, cx| {
                terminal.update_config(config.clone(), cx);
            });
        }
        self.terminal_config = config;
    }

    pub fn update_terminal_appearance(
        &mut self,
        config: TerminalConfig,
        theme: WorkbenchTheme,
        cx: &mut Context<Self>,
    ) {
        self.update_terminal_config(config, cx);
        self.theme = theme;
        cx.notify();
    }

    #[cfg(feature = "perf-metrics")]
    fn performance_metrics_enabled(&self) -> bool {
        std::env::var_os("YTTT_TERMINAL_PERF_OUTPUT").is_some()
            && std::env::var("YTTT_TERMINAL_PERF_PANE_ID")
                .ok()
                .is_none_or(|pane_id| pane_id == self.pane_id)
    }

    #[cfg(feature = "perf-metrics")]
    fn start_performance_probe(&mut self, window_handle: AnyWindowHandle, cx: &mut Context<Self>) {
        if self.performance_probe_started || !self.performance_metrics_enabled() {
            return;
        }
        let Some(delay_ms) = std::env::var("YTTT_TERMINAL_PERF_INPUT_DELAY_MS")
            .ok()
            .and_then(|value| value.parse::<u64>().ok())
        else {
            return;
        };
        let Some(terminal) = self.terminal.clone() else {
            return;
        };
        let start_file =
            std::env::var_os("YTTT_TERMINAL_PERF_START_FILE").map(std::path::PathBuf::from);
        terminal.update(cx, |terminal, cx| {
            terminal.start_performance_input_probe(
                window_handle,
                Duration::from_millis(delay_ms),
                start_file,
                cx,
            );
        });
        self.performance_probe_started = true;
    }
}

impl EventEmitter<TerminalPaneEvent> for TerminalPaneView {}

impl Drop for TerminalPaneView {
    fn drop(&mut self) {
        self.host_events_task = None;
        if let (Some(runtime), Some(session_id)) = (&self.host_runtime, self.host_session_id.take())
        {
            let _ = runtime.request(Request::DetachTerminal { session_id });
        }
        self.terminal.take();
    }
}

impl Render for TerminalPaneView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.window_handle = Some(window.window_handle());
        #[cfg(feature = "perf-metrics")]
        self.start_performance_probe(window.window_handle(), cx);
        let ui_style = current_ui_style(cx);
        let ui_text = UiText::current(cx);
        if let Some(writer) = &self.control_writer {
            writer.locale.store(
                if ui_text.locale() == Locale::Chinese {
                    1
                } else {
                    0
                },
                Ordering::Relaxed,
            );
        }
        let body = if let Some(terminal) = &self.terminal {
            div().flex().flex_1().child(terminal.clone())
        } else {
            let lines = match &self.lifecycle {
                PaneLifecycle::SpawnFailed { .. } => spawn_failure_lines(
                    &TerminalSpawnFailure {
                        command: self.command.clone(),
                        cwd: self.project_path.clone(),
                        message: terminal_start_error(
                            &self.lifecycle,
                            &self.terminal_error,
                            &ui_text,
                        ),
                    },
                    &ui_text,
                ),
                PaneLifecycle::Lost { message } => vec![
                    ui_text
                        .get(UiTextKey::TerminalI18nSessionUnavailable)
                        .to_string(),
                    ui_text
                        .get(UiTextKey::TerminalI18nCommand)
                        .replace("{command}", &self.command),
                    ui_text
                        .get(UiTextKey::TerminalI18nWorkingDirectory)
                        .replace("{cwd}", &self.project_path.display().to_string()),
                    ui_text
                        .get(UiTextKey::TerminalI18nStatus)
                        .replace("{status}", message),
                ],
                _ => vec![
                    ui_text
                        .get(UiTextKey::TerminalI18nProcessLifecycle)
                        .replace("{state}", &pane_lifecycle_label(&self.lifecycle, &ui_text)),
                    ui_text
                        .get(UiTextKey::TerminalI18nCommand)
                        .replace("{command}", &self.command),
                    ui_text
                        .get(UiTextKey::TerminalI18nWorkingDirectory)
                        .replace("{cwd}", &self.project_path.display().to_string()),
                ],
            };
            let actions = match &self.lifecycle {
                PaneLifecycle::SpawnFailed { .. } => vec![(
                    "retry",
                    ui_text.get(UiTextKey::TerminalI18nRetry),
                    PaneRecoveryAction::Retry,
                )],
                PaneLifecycle::Exited { .. } if self.resumable => vec![
                    (
                        "resume",
                        ui_text.get(UiTextKey::TerminalI18nResumeSavedSession),
                        PaneRecoveryAction::Restart { resume: true },
                    ),
                    (
                        "restart",
                        ui_text.get(UiTextKey::TerminalI18nStartNewProcess),
                        PaneRecoveryAction::Restart { resume: false },
                    ),
                ],
                PaneLifecycle::Exited { .. } => vec![(
                    "restart",
                    ui_text.get(UiTextKey::TerminalI18nRestart),
                    PaneRecoveryAction::Restart { resume: false },
                )],
                _ => Vec::new(),
            };
            let failed = matches!(
                self.lifecycle,
                PaneLifecycle::SpawnFailed { .. } | PaneLifecycle::Exited { .. }
            );
            let buttons = actions.into_iter().map(|(key, label, action)| {
                div()
                    .id(SharedString::from(format!("{key}-pane-{}", self.pane_id)))
                    .cursor_pointer()
                    .rounded(ui_style.radius.action)
                    .border(ui_style.border.hairline)
                    .border_color(self.theme.border)
                    .bg(self.theme.surface)
                    .px(ui_style.spacing.lg)
                    .py(ui_style.spacing.xs)
                    .text_color(self.theme.text)
                    .hover(|button| button.bg(ui_style.hover_background(self.theme)))
                    .on_click(cx.listener(move |this, _, window, cx| match action {
                        PaneRecoveryAction::Retry => {
                            this.start_terminal(window, cx);
                        }
                        PaneRecoveryAction::Restart { resume } => this.request_restart(resume, cx),
                    }))
                    .child(label)
            });

            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.md)
                .flex_1()
                .items_center()
                .justify_center()
                .bg(self.theme.terminal_background)
                .text_color(if failed {
                    self.theme.danger
                } else {
                    self.theme.text
                })
                .children(lines)
                .child(div().flex().gap(ui_style.spacing.md).children(buttons))
        };

        div().flex().flex_col().flex_1().child(body)
    }
}

pub fn pane_lifecycle_label(lifecycle: &PaneLifecycle, text: &UiText) -> String {
    match lifecycle {
        PaneLifecycle::Idle => text.get(UiTextKey::TerminalI18nLifecycleIdle).to_string(),
        PaneLifecycle::Starting => text
            .get(UiTextKey::TerminalI18nLifecycleStarting)
            .to_string(),
        PaneLifecycle::Running => text
            .get(UiTextKey::TerminalI18nLifecycleRunning)
            .to_string(),
        PaneLifecycle::Stopping { .. } => text
            .get(UiTextKey::TerminalI18nLifecycleStopping)
            .to_string(),
        PaneLifecycle::Exited {
            code: Some(code), ..
        } => text
            .get(UiTextKey::TerminalI18nLifecycleExitedCode)
            .replace("{code}", &code.to_string()),
        PaneLifecycle::Exited {
            code: None,
            reason: ExitReason::KilledByUser,
        } => text.get(UiTextKey::TerminalI18nLifecycleKilled).to_string(),
        PaneLifecycle::Exited { code: None, .. } => {
            text.get(UiTextKey::TerminalI18nLifecycleExited).to_string()
        }
        PaneLifecycle::SpawnFailed { .. } => text
            .get(UiTextKey::TerminalI18nLifecycleSpawnFailed)
            .to_string(),
        PaneLifecycle::Lost { .. } => text.get(UiTextKey::TerminalI18nLifecycleLost).to_string(),
    }
}

pub fn spawn_failure_lines(failure: &TerminalSpawnFailure, text: &UiText) -> Vec<String> {
    vec![
        text.get(UiTextKey::TerminalI18nStartFailed).to_string(),
        text.get(UiTextKey::TerminalI18nCommand)
            .replace("{command}", &failure.command),
        text.get(UiTextKey::TerminalI18nWorkingDirectory)
            .replace("{cwd}", &failure.cwd.display().to_string()),
        text.get(UiTextKey::TerminalI18nError)
            .replace("{error}", &failure.message),
    ]
}

#[derive(Clone, Copy)]
enum PaneRecoveryAction {
    Retry,
    Restart { resume: bool },
}

fn terminal_start_error(
    lifecycle: &PaneLifecycle,
    terminal_error: &Option<String>,
    text: &UiText,
) -> String {
    match lifecycle {
        PaneLifecycle::SpawnFailed { message } | PaneLifecycle::Lost { message } => message.clone(),
        _ => terminal_error
            .clone()
            .unwrap_or_else(|| text.get(UiTextKey::TerminalI18nDidNotStart).to_string()),
    }
}

pub fn notification_for_terminal_pane_exit(
    input: TerminalPaneExitInput,
) -> Option<NotificationEvent> {
    let exit_code = match input.status {
        ProcessStatus::Running => None,
        ProcessStatus::Exited { code } => code,
    };
    let is_agent = classify_agent(Some(input.kind), &input.command).is_agent();

    notification_for_exit(ExitNotificationInput {
        is_agent,
        notify_on_exit: input.notify_on_exit,
        exit_code,
        exit_reason: input.exit_reason,
        project_id: input.project_id,
        tab_id: input.tab_id,
        pane_id: input.pane_id,
        project_title: input.project_title,
        tab_title: input.tab_title,
        pane_title: input.pane_title,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::layout::{ProcessExitBehavior, TerminalExecutionMode};

    #[test]
    fn terminal_runtime_title_reset_restores_configured_default() {
        let configured = "Configured shell";
        assert_eq!(
            resolved_terminal_title(configured, "vim main.rs"),
            "vim main.rs"
        );
        assert_eq!(resolved_terminal_title(configured, ""), configured);
    }

    #[gpui::test]
    fn production_terminal_refuses_to_spawn_without_host_runtime(cx: &mut gpui::TestAppContext) {
        let context = TerminalPaneContext {
            project_id: "project".to_string(),
            project_path: PathBuf::from("/tmp/project"),
            project_title: "Project".to_string(),
            tab_id: "tab".to_string(),
            tab_title: "Tab".to_string(),
            pane: PaneConfig {
                id: "shell".to_string(),
                title: "Shell".to_string(),
                command: String::new(),
                args: Vec::new(),
                execution_mode: TerminalExecutionMode::Shell,
                exit_behavior: ProcessExitBehavior::ManualRestart,
                kind: PaneKind::Shell,
                notify_on_exit: false,
                detector: None,
            },
            is_focused: true,
            terminal_input_gate: TerminalInputGate::default(),
        };
        let (pane, cx) = cx.add_window_view(|window, cx| {
            TerminalPaneView::new(
                context,
                TerminalConfig::default(),
                WorkbenchTheme::one_dark(),
                window,
                cx,
            )
        });

        cx.read(|app| {
            let pane = pane.read(app);
            assert!(pane.terminal.is_none());
            assert!(matches!(&pane.lifecycle, PaneLifecycle::SpawnFailed { .. }));
        });
    }

    #[test]
    fn terminal_pane_io_error_lifecycle_is_single_shot() {
        let generation = 7;
        let mut exit_emitted = false;
        let mut handled = 0;
        for callback_generation in [generation - 1, generation, generation] {
            if accepts_process_exit(generation, callback_generation, exit_emitted) {
                exit_emitted = true;
                handled += 1;
            }
        }
        assert_eq!(handled, 1);
    }
}

#[cfg(all(test, unix))]
pub(crate) mod recovery_tests;
