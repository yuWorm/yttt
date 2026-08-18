use std::{
    collections::BTreeMap,
    io::{self, Write},
    path::PathBuf,
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use crate::{
    host_runtime::{
        DesktopHostRuntime, HostRuntimeGlobal, TerminalPaneHostEvent, TerminalRecoveryError,
    },
    model::layout::{PaneConfig, PaneKind, ProcessExitBehavior, TerminalExecutionMode},
    runtime::{
        agent::classify_agent,
        agent_hooks::AGENT_HOOK_ENVIRONMENT_VARIABLES,
        agent_manager::{AgentPaneAddress, AgentPaneLaunch},
        notification::{ExitNotificationInput, NotificationEvent, notification_for_exit},
    },
    ui::{
        interaction::input_owner::TerminalInputGate,
        theme::{WorkbenchTheme, current_ui_style},
    },
};
use gpui::{
    AnyWindowHandle, Context, Entity, EventEmitter, IntoElement, Render, SharedString, Task,
    Window, div, prelude::*,
};
use yttt_agent_core::AgentInstanceId;
use yttt_client_core::{ClientEvent, ConnectionState, TerminalMirrorMetadata};
use yttt_core::model::ids::{ConnectionId, ProjectId, TerminalSessionId};
use yttt_protocol::{
    Request, Response, ServerEvent,
    terminal::{
        RemoteTerminalExecutionSpec, ResizeTerminal, ScrollTerminal, TerminalExecutionSpec,
        TerminalGeometry, TerminalInput, TerminalMutationContext, TerminalProcessState,
        TerminalSpawnSpec, TerminationMode,
    },
};
use yttt_terminal::{ExitReason, ProcessStatus, PtyIoOperation, TerminalConfig, TerminalView};

#[derive(Clone)]
pub struct SshTerminalContext {
    pub connection_id: ConnectionId,
}

impl std::fmt::Debug for SshTerminalContext {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SshTerminalContext")
            .field("connection_id", &self.connection_id)
            .finish()
    }
}

#[derive(Clone, Debug)]
pub struct TerminalPaneContext {
    pub project_id: String,
    pub project_path: PathBuf,
    pub project_title: String,
    pub tab_id: String,
    pub tab_title: String,
    pub pane: PaneConfig,
    pub shell: String,
    pub environment: Arc<RwLock<BTreeMap<String, String>>>,
    pub is_focused: bool,
    pub terminal_input_gate: TerminalInputGate,
    pub ssh: Option<SshTerminalContext>,
    pub agent_launch: Option<AgentPaneLaunch>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalPaneEvent {
    Notification(NotificationEvent),
    Started(TerminalPaneStartedEvent),
    StartFailed(TerminalPaneStartFailedEvent),
    Exited(TerminalPaneExitedEvent),
    AgentStatusFrame {
        pane_id: String,
        frame: String,
    },
    TitleChanged {
        pane_id: String,
        title: String,
    },
    IoError {
        pane_id: String,
        message: String,
        fatal: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneStartedEvent {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub generation: u64,
    pub agent_instance_id: Option<AgentInstanceId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneStartFailedEvent {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub generation: u64,
    pub agent_instance_id: Option<AgentInstanceId>,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneExitedEvent {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub status: ProcessStatus,
    pub exit_reason: ExitReason,
    pub exit_behavior: ProcessExitBehavior,
    pub generation: u64,
    pub agent_instance_id: Option<AgentInstanceId>,
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

#[derive(Debug, thiserror::Error)]
enum TerminalStartAttemptError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Recovery(#[from] TerminalRecoveryError),
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
    args: Vec<String>,
    execution_mode: TerminalExecutionMode,
    exit_behavior: ProcessExitBehavior,
    shell: String,
    environment: Arc<RwLock<BTreeMap<String, String>>>,
    notify_on_exit: bool,
    agent_launch: Option<AgentPaneLaunch>,
    agent_session_title: Option<String>,
    ssh: Option<SshTerminalContext>,
    terminal: Option<Entity<TerminalView>>,
    terminal_config: TerminalConfig,
    theme: WorkbenchTheme,
    host_runtime: Option<Arc<DesktopHostRuntime>>,
    host_session_id: Option<TerminalSessionId>,
    host_session_epoch: Option<u64>,
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
fn terminal_io_error_message(operation: PtyIoOperation, message: &str) -> String {
    format!("Terminal {operation:?} error: {message}")
}

fn interactive_shell_args(shell: &str) -> Vec<String> {
    let name = shell
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(shell)
        .to_ascii_lowercase();
    if matches!(name.as_str(), "cmd" | "cmd.exe") {
        vec!["/D".to_string()]
    } else if matches!(
        name.as_str(),
        "powershell" | "powershell.exe" | "pwsh" | "pwsh.exe"
    ) {
        vec!["-NoLogo".to_string()]
    } else if matches!(
        name.as_str(),
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

fn accepts_process_exit(
    active_generation: u64,
    callback_generation: u64,
    exit_emitted: bool,
) -> bool {
    active_generation == callback_generation && !exit_emitted
}

struct HostTerminalWriter {
    runtime: Arc<DesktopHostRuntime>,
    session_id: TerminalSessionId,
    mutation_context: Arc<RwLock<TerminalMutationContext>>,
    next_sequence: Arc<AtomicU64>,
}

fn next_mutation_context(
    context: &Arc<RwLock<TerminalMutationContext>>,
    next_sequence: &Arc<AtomicU64>,
    geometry_epoch: Option<u64>,
) -> TerminalMutationContext {
    let mut context = context.write().unwrap();
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
    let Some(geometry) = pending_geometry.write().unwrap().take() else {
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

impl Write for HostTerminalWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !matches!(self.runtime.state(), ConnectionState::Ready { .. }) {
            return Err(io::Error::new(
                io::ErrorKind::BrokenPipe,
                "Host terminal connection is not ready",
            ));
        }
        let context = next_mutation_context(&self.mutation_context, &self.next_sequence, None);
        self.runtime
            .send_terminal_input(TerminalInput {
                session_id: self.session_id.clone(),
                context,
                bytes: bytes.to_vec(),
            })
            .map_err(|error| io::Error::new(io::ErrorKind::WouldBlock, error.to_string()))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl TerminalPaneView {
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
            shell,
            environment,
            is_focused: _,
            terminal_input_gate,
            ssh,
            agent_launch,
        } = context;
        let agent_session_title = agent_launch
            .as_ref()
            .and_then(|launch| launch.restored_title_for(&pane.title))
            .map(ToOwned::to_owned);
        let initial_title = agent_session_title
            .clone()
            .unwrap_or_else(|| pane.title.clone());
        Self {
            project_id,
            project_path,
            project_title,
            tab_id,
            tab_title,
            pane_id: pane.id,
            default_title: pane.title,
            title: initial_title,
            command: pane.command,
            args: pane.args,
            execution_mode: pane.execution_mode,
            exit_behavior: pane.exit_behavior,
            shell,
            environment,
            kind: pane.kind,
            notify_on_exit: pane.notify_on_exit,
            agent_launch,
            agent_session_title,
            ssh,
            terminal: None,
            terminal_config,
            theme,
            host_runtime: None,
            host_session_id: None,
            host_session_epoch: None,
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
        self.terminal = Some(cx.new(|cx| {
            TerminalView::new_semantic(std::io::sink(), config, cx)
                .with_key_handler(move |_event| !terminal_input_allowed.load(Ordering::SeqCst))
        }));
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

    pub fn agent_instance_id(&self) -> Option<&AgentInstanceId> {
        self.agent_launch.as_ref().map(AgentPaneLaunch::instance_id)
    }

    pub fn agent_pane_address(&self) -> AgentPaneAddress {
        AgentPaneAddress::new(&self.project_id, &self.tab_id, &self.pane_id)
    }

    pub fn matches_agent_pane_address(&self, address: &AgentPaneAddress) -> bool {
        self.project_id == address.project_id
            && self.tab_id == address.tab_id
            && self.pane_id == address.pane_id
    }

    fn command_args(&self) -> Vec<String> {
        let mut args = if self
            .agent_launch
            .as_ref()
            .is_some_and(|launch| launch.program_override().is_some())
        {
            Vec::new()
        } else {
            self.args.clone()
        };
        if let Some(agent_launch) = &self.agent_launch {
            args.extend(agent_launch.additional_args().iter().cloned());
        }
        args
    }

    fn spawn_environment(&self) -> BTreeMap<String, String> {
        let mut environment = self
            .environment
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        if let Some(agent_launch) = &self.agent_launch {
            environment.extend(agent_launch.environment(self.generation));
        }
        remove_inherited_agent_hook_environment(&mut environment);
        environment
    }

    fn host_session_id(&self) -> TerminalSessionId {
        TerminalSessionId::new(format!(
            "{}:{}:{}",
            self.project_id, self.tab_id, self.pane_id
        ))
    }

    fn remote_terminal_execution(&self) -> RemoteTerminalExecutionSpec {
        if let Some(program) = self
            .agent_launch
            .as_ref()
            .and_then(AgentPaneLaunch::program_override)
        {
            return match self
                .agent_launch
                .as_ref()
                .and_then(|launch| launch.remote_command(&self.command, &self.args))
            {
                Some(command) => RemoteTerminalExecutionSpec::Shell { command },
                None => RemoteTerminalExecutionSpec::Command {
                    program: program.to_string(),
                    args: self.command_args(),
                },
            };
        }
        match self.execution_mode {
            TerminalExecutionMode::Shell => RemoteTerminalExecutionSpec::Shell {
                command: self.command.clone(),
            },
            TerminalExecutionMode::Command => match self
                .agent_launch
                .as_ref()
                .and_then(|launch| launch.remote_command(&self.command, &self.args))
            {
                Some(command) => RemoteTerminalExecutionSpec::Shell { command },
                None => RemoteTerminalExecutionSpec::Command {
                    program: self.command.clone(),
                    args: self.command_args(),
                },
            },
        }
    }

    fn host_spawn_spec(&self, geometry: TerminalGeometry) -> anyhow::Result<TerminalSpawnSpec> {
        let execution = if let Some(ssh) = &self.ssh {
            TerminalExecutionSpec::Ssh {
                connection_id: ssh.connection_id.as_str().to_string(),
                execution: self.remote_terminal_execution(),
            }
        } else if let Some(program) = self
            .agent_launch
            .as_ref()
            .and_then(AgentPaneLaunch::program_override)
        {
            TerminalExecutionSpec::Command {
                shell: self.shell.clone(),
                program: program.to_string(),
                args: self.command_args(),
                return_to_shell: false,
            }
        } else {
            match self.execution_mode {
                TerminalExecutionMode::Shell => TerminalExecutionSpec::Shell {
                    program: self.shell.clone(),
                    args: interactive_shell_args(&self.shell),
                    initial_command: (!self.command.trim().is_empty())
                        .then(|| self.command.clone()),
                },
                TerminalExecutionMode::Command => TerminalExecutionSpec::Command {
                    shell: self.shell.clone(),
                    program: self.command.clone(),
                    args: self.command_args(),
                    return_to_shell: false,
                },
            }
        };
        let environment = self.spawn_environment().into_iter().collect();
        Ok(TerminalSpawnSpec {
            session_id: self.host_session_id(),
            project_id: ProjectId::new(self.project_id.clone()),
            cwd: yttt_protocol::ProjectRelativePath::root(),
            execution,
            geometry,
            geometry_epoch: 1,
            query_palette: self.terminal_config.colors.query_palette(),
            palette_revision: 1,
            scrollback_limit: self.terminal_config.scrollback.min(u32::MAX as usize) as u32,
            environment,
            removed_environment: AGENT_HOOK_ENVIRONMENT_VARIABLES
                .iter()
                .map(|name| (*name).to_string())
                .collect(),
        })
    }

    fn start_host_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        let Some(host_runtime) = cx
            .try_global::<HostRuntimeGlobal>()
            .and_then(HostRuntimeGlobal::runtime)
            .cloned()
        else {
            self.lifecycle = PaneLifecycle::Starting;
            self.generation = self.generation.wrapping_add(1);
            self.set_spawn_failure("Host runtime is unavailable".to_string(), cx);
            return false;
        };
        let host_epoch = match host_runtime.state() {
            ConnectionState::Ready { host_epoch, .. } => host_epoch,
            _ => {
                self.lifecycle = PaneLifecycle::Starting;
                self.generation = self.generation.wrapping_add(1);
                self.set_spawn_failure("Host runtime is not connected".to_string(), cx);
                return false;
            }
        };

        let initial_title = self
            .agent_session_title
            .clone()
            .unwrap_or_else(|| self.default_title.clone());
        self.set_runtime_title(initial_title, cx);
        self.terminal = None;
        self.host_events_task = None;
        self.lifecycle = PaneLifecycle::Starting;
        self.terminal_error = None;
        self.exit_emitted = false;
        self.generation = self.generation.wrapping_add(1);
        #[cfg(feature = "perf-metrics")]
        {
            self.performance_reporter = None;
            self.performance_probe_started = false;
        }

        let geometry = TerminalGeometry {
            cols: self.terminal_config.cols.min(u16::MAX as usize) as u16,
            rows: self.terminal_config.rows.min(u16::MAX as usize) as u16,
            cell_width: 0,
            cell_height: 0,
        };
        let spec = match self.host_spawn_spec(geometry) {
            Ok(spec) => spec,
            Err(error) => {
                self.set_spawn_failure(error.to_string(), cx);
                return false;
            }
        };
        let session_id = spec.session_id.clone();
        let generation = self.generation;
        let next_mutation_sequence = Arc::new(AtomicU64::new(1));
        let next_geometry_epoch = Arc::new(AtomicU64::new(spec.geometry_epoch));
        let mutation_context = Arc::new(RwLock::new(TerminalMutationContext {
            host_epoch,
            session_epoch: 0,
            lease_epoch: 0,
            geometry_epoch: spec.geometry_epoch,
            client_sequence: 0,
        }));
        let pending_resize_geometry = Arc::new(RwLock::new(None::<TerminalGeometry>));
        let resize_geometry_epoch = next_geometry_epoch.clone();
        let writer = HostTerminalWriter {
            runtime: host_runtime.clone(),
            session_id: session_id.clone(),
            mutation_context: mutation_context.clone(),
            next_sequence: next_mutation_sequence.clone(),
        };
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
        let error_parent = cx.weak_entity();
        let initial_config = self.terminal_config.clone();
        let terminal = cx.new(|cx| {
            TerminalView::new_semantic(writer, initial_config, cx)
                .with_key_handler(move |_event| !terminal_input_allowed.load(Ordering::SeqCst))
                .with_resize_callback(move |cols, rows| {
                    if !matches!(resize_runtime.state(), ConnectionState::Ready { .. }) {
                        return Err("Host terminal connection is not ready".to_string());
                    }
                    let geometry = TerminalGeometry {
                        cols,
                        rows,
                        cell_width: 0,
                        cell_height: 0,
                    };
                    if resize_mutation_context.read().unwrap().lease_epoch == 0 {
                        *resize_pending_geometry.write().unwrap() = Some(geometry);
                        return Ok(());
                    }
                    *resize_pending_geometry.write().unwrap() = Some(geometry);
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
                .with_semantic_scroll_callback(move |display_offset| {
                    if matches!(scroll_runtime.state(), ConnectionState::Ready { .. }) {
                        let context = next_mutation_context(
                            &scroll_mutation_context,
                            &scroll_mutation_sequence,
                            None,
                        );
                        let _ = scroll_runtime.request(Request::ScrollTerminal(ScrollTerminal {
                            session_id: scroll_session_id.clone(),
                            context,
                            display_offset,
                        }));
                    }
                })
                .with_io_error_callback(move |cx, operation, message, fatal| {
                    let message = terminal_io_error_message(operation, message);
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
        let terminal_updates = host_runtime.terminal_viewports(session_id.clone());
        terminal.update(cx, |terminal, cx| {
            terminal.attach_semantic_viewport_stream(terminal_updates, cx);
        });

        let host_events = host_runtime.terminal_pane_events(session_id.clone());
        let event_runtime = host_runtime.clone();
        let event_session_id = session_id.clone();
        let event_task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(event) = host_events.recv_async().await {
                let runtime = event_runtime.clone();
                let session_id = event_session_id.clone();
                let result = match event {
                    TerminalPaneHostEvent::Metadata(metadata) => {
                        this.update_in(cx, move |pane, _window, cx| {
                            pane.handle_host_viewport_metadata(
                                &metadata,
                                &runtime,
                                &session_id,
                                generation,
                                cx,
                            );
                        })
                    }
                    TerminalPaneHostEvent::Client(event) => {
                        this.update_in(cx, move |pane, _window, cx| {
                            pane.handle_host_event(event, runtime, &session_id, generation, cx);
                        })
                    }
                };
                if result.is_err() {
                    break;
                }
            }
        });

        self.terminal = Some(terminal);
        self.host_runtime = Some(host_runtime.clone());
        self.host_session_id = Some(session_id.clone());
        self.host_session_epoch = None;
        self.host_events_task = Some(event_task);
        #[cfg(feature = "perf-metrics")]
        self.start_performance_probe(cx);

        let request_runtime = host_runtime.clone();
        let request_spec = spec;
        let response_geometry_epoch = next_geometry_epoch.clone();
        let response_mutation_context = mutation_context.clone();
        let response_mutation_sequence = next_mutation_sequence.clone();
        let response_pending_resize = pending_resize_geometry.clone();
        cx.spawn(async move |this, cx| {
            let result = async {
                let catalog_response = request_runtime
                    .request(Request::ListResources)
                    .recv_async()
                    .await
                    .map_err(|_| {
                        TerminalStartAttemptError::Message(
                            "Host catalog request channel closed".to_string(),
                        )
                    })?
                    .map_err(|error| TerminalStartAttemptError::Message(error.to_string()))?;
                let Response::Resources(catalog) = catalog_response else {
                    return Err(TerminalStartAttemptError::Message(format!(
                        "unexpected Host catalog response: {catalog_response:?}"
                    )));
                };
                let spawn_fingerprint = request_spec.address_fingerprint();
                let request = request_runtime.terminal_start_request(request_spec, &catalog)?;
                let response = request_runtime
                    .request(request)
                    .recv_async()
                    .await
                    .map_err(|_| {
                        TerminalStartAttemptError::Message(
                            "Host terminal request channel closed".to_string(),
                        )
                    })?
                    .map_err(|error| TerminalStartAttemptError::Message(error.to_string()))?;
                Ok((response, catalog, spawn_fingerprint))
            }
            .await;
            let _ = this.update(cx, |pane, cx| {
                if pane.generation != generation {
                    return;
                }
                match result {
                    Ok((
                        Response::TerminalSpawned {
                            lease,
                            session_epoch,
                        },
                        catalog,
                        spawn_fingerprint,
                    )) if lease.session_id == session_id => {
                        if let Err(error) = request_runtime.bind_terminal(
                            &catalog,
                            lease.session_id.clone(),
                            session_epoch,
                            spawn_fingerprint,
                        ) {
                            pane.set_spawn_failure(error, cx);
                            return;
                        }
                        {
                            let mut context = response_mutation_context.write().unwrap();
                            context.session_epoch = session_epoch;
                            context.lease_epoch = lease.lease_epoch;
                        }
                        submit_pending_host_resize(
                            &request_runtime,
                            &session_id,
                            &response_mutation_context,
                            &response_mutation_sequence,
                            &response_geometry_epoch,
                            &response_pending_resize,
                        );
                        pane.host_session_epoch = Some(session_epoch);
                        pane.lifecycle = PaneLifecycle::Running;
                        cx.emit(TerminalPaneEvent::Started(TerminalPaneStartedEvent {
                            project_id: pane.project_id.clone(),
                            tab_id: pane.tab_id.clone(),
                            pane_id: pane.pane_id.clone(),
                            generation,
                            agent_instance_id: pane.agent_instance_id().cloned(),
                        }));
                        cx.notify();
                    }
                    Ok((
                        Response::TerminalAttached { lease, checkpoint },
                        catalog,
                        spawn_fingerprint,
                    )) if lease.session_id == session_id => {
                        let session_epoch = checkpoint.viewport.session_epoch;
                        if let Err(error) = request_runtime.bind_terminal(
                            &catalog,
                            lease.session_id,
                            session_epoch,
                            spawn_fingerprint,
                        ) {
                            pane.set_spawn_failure(error, cx);
                            return;
                        }
                        response_geometry_epoch
                            .store(checkpoint.viewport.geometry_epoch, Ordering::Relaxed);
                        {
                            let mut context = response_mutation_context.write().unwrap();
                            context.session_epoch = session_epoch;
                            context.lease_epoch = lease.lease_epoch;
                            context.geometry_epoch = checkpoint.viewport.geometry_epoch;
                        }
                        submit_pending_host_resize(
                            &request_runtime,
                            &session_id,
                            &response_mutation_context,
                            &response_mutation_sequence,
                            &response_geometry_epoch,
                            &response_pending_resize,
                        );
                        pane.host_session_epoch = Some(session_epoch);
                        let process_state = checkpoint.viewport.process_state;
                        let final_sequence = checkpoint.viewport.sequence;
                        if let Some(terminal) = pane.terminal.clone() {
                            terminal.update(cx, |terminal, cx| {
                                terminal.set_semantic_viewport(checkpoint.viewport, cx);
                            });
                        }
                        if let TerminalProcessState::Exited { code } = process_state {
                            pane.finalize_host_process_exit(
                                &request_runtime,
                                &session_id,
                                session_epoch,
                                final_sequence,
                                code,
                                generation,
                                cx,
                            );
                            return;
                        }
                        pane.lifecycle = PaneLifecycle::Running;
                        cx.emit(TerminalPaneEvent::Started(TerminalPaneStartedEvent {
                            project_id: pane.project_id.clone(),
                            tab_id: pane.tab_id.clone(),
                            pane_id: pane.pane_id.clone(),
                            generation,
                            agent_instance_id: pane.agent_instance_id().cloned(),
                        }));
                        cx.notify();
                    }
                    Ok((response, _, _)) => {
                        pane.set_spawn_failure(
                            format!("unexpected Host terminal response: {response:?}"),
                            cx,
                        );
                    }
                    Err(error) => pane.set_spawn_failure(error.to_string(), cx),
                }
            });
        })
        .detach();
        true
    }

    fn handle_host_viewport_metadata(
        &mut self,
        metadata: &TerminalMirrorMetadata,
        runtime: &DesktopHostRuntime,
        session_id: &TerminalSessionId,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation {
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
                session_id,
                metadata.session_epoch,
                metadata.sequence,
                code,
                generation,
                cx,
            );
        }
    }

    fn handle_host_event(
        &mut self,
        event: ClientEvent,
        runtime: Arc<DesktopHostRuntime>,
        session_id: &TerminalSessionId,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if self.generation != generation {
            return;
        }
        match event {
            ClientEvent::TerminalUnavailable(unavailable) if unavailable == *session_id => {
                self.set_terminal_lost(
                    "Terminal session was lost while the Host was unavailable".to_string(),
                    cx,
                );
            }
            ClientEvent::Server(host_event) => match host_event.body {
                ServerEvent::TerminalExit {
                    session_id: exited_session_id,
                    session_epoch,
                    code,
                    final_sequence,
                } if exited_session_id == *session_id => {
                    let final_viewport_is_ready = runtime
                        .terminal_metadata(session_id)
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
                        &runtime,
                        session_id,
                        session_epoch,
                        final_sequence,
                        code,
                        generation,
                        cx,
                    );
                }
                ServerEvent::TerminalLeaseRevoked {
                    session_id: revoked_session_id,
                    ..
                } if revoked_session_id == *session_id => {
                    self.terminal_error =
                        Some("Terminal input lease was revoked by another client".to_string());
                    cx.notify();
                }
                ServerEvent::TerminalLeaseReleased {
                    session_id: released_session_id,
                    ..
                } if released_session_id == *session_id => {
                    self.terminal_error = Some("Terminal input lease was released".to_string());
                    cx.notify();
                }
                ServerEvent::TerminalLeaseExpired {
                    session_id: expired_session_id,
                    ..
                } if expired_session_id == *session_id => {
                    self.terminal_error =
                        Some("Terminal input lease expired after idle takeover".to_string());
                    cx.notify();
                }
                ServerEvent::TerminalControlRequested {
                    session_id: requested_session_id,
                    requester,
                } if requested_session_id == *session_id => {
                    self.terminal_error = Some(format!(
                        "Another client ({}) requested terminal control",
                        requester.as_str()
                    ));
                    cx.notify();
                }
                ServerEvent::TerminalControlGranted { lease }
                    if lease.session_id == *session_id =>
                {
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
        session_id: &TerminalSessionId,
        session_epoch: u64,
        final_sequence: u64,
        code: Option<i32>,
        generation: u64,
        cx: &mut Context<Self>,
    ) {
        if !accepts_process_exit(self.generation, generation, self.exit_emitted) {
            return;
        }
        self.exit_emitted = true;
        self.terminal = None;
        self.host_session_epoch = Some(session_epoch);
        self.lifecycle = PaneLifecycle::Stopping {
            reason: ExitReason::Completed,
        };
        self.finalize_process_exit(
            generation,
            ExitReason::Completed,
            ProcessStatus::Exited { code },
            cx,
        );
        if let Err(error) =
            runtime.acknowledge_terminal_exit(session_id.clone(), session_epoch, final_sequence)
        {
            eprintln!("failed to queue terminal exit acknowledgement: {error}");
        }
        self.host_session_id = None;
        self.host_session_epoch = None;
    }

    pub(crate) fn start_terminal(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        self.window_handle = Some(window.window_handle());
        self.start_host_terminal(window, cx)
    }

    fn set_spawn_failure(&mut self, message: String, cx: &mut Context<Self>) {
        self.lifecycle = PaneLifecycle::SpawnFailed {
            message: message.clone(),
        };
        self.terminal_error = Some(message.clone());
        self.terminal = None;
        self.host_events_task = None;
        self.host_session_id = None;
        self.host_session_epoch = None;
        self.host_runtime = None;
        cx.emit(TerminalPaneEvent::StartFailed(
            TerminalPaneStartFailedEvent {
                project_id: self.project_id.clone(),
                tab_id: self.tab_id.clone(),
                pane_id: self.pane_id.clone(),
                generation: self.generation,
                agent_instance_id: self.agent_instance_id().cloned(),
                message,
            },
        ));
        cx.notify();
    }

    fn set_terminal_lost(&mut self, message: String, cx: &mut Context<Self>) {
        self.lifecycle = PaneLifecycle::Lost {
            message: message.clone(),
        };
        self.terminal_error = Some(message);
        self.terminal = None;
        self.host_events_task = None;
        self.host_session_id = None;
        self.host_session_epoch = None;
        self.host_runtime = None;
        cx.notify();
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
        self.terminal = None;
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

        let exit_event = TerminalPaneExitedEvent {
            project_id: self.project_id.clone(),
            tab_id: self.tab_id.clone(),
            pane_id: self.pane_id.clone(),
            status,
            exit_reason,
            exit_behavior: self.exit_behavior,
            generation: self.generation,
            agent_instance_id: self.agent_instance_id().cloned(),
        };
        let notification = notification_for_terminal_pane_exit(TerminalPaneExitInput {
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
        });

        if let Some(notification) = notification {
            cx.emit(TerminalPaneEvent::Notification(notification));
        }
        cx.emit(TerminalPaneEvent::Exited(exit_event));

        if self.exit_behavior == ProcessExitBehavior::AutoRestart {
            self.schedule_auto_restart(cx);
        } else {
            cx.notify();
        }
    }

    fn schedule_auto_restart(&self, cx: &mut Context<Self>) {
        let generation = self.generation;
        let Some(window_handle) = self.window_handle else {
            return;
        };
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = window_handle.update(cx, |_root, window, cx| {
                let _ = this.update(cx, |pane, cx| {
                    if pane.generation == generation
                        && pane.exit_behavior == ProcessExitBehavior::AutoRestart
                        && matches!(pane.lifecycle, PaneLifecycle::Exited { .. })
                    {
                        pane.start_terminal(window, cx);
                    }
                });
            });
        })
        .detach();
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
        if let (Some(runtime), Some(session_id)) = (&self.host_runtime, self.host_session_id.take())
        {
            let _ = runtime.request(Request::TerminateTerminal {
                session_id,
                mode: TerminationMode::Terminate,
            });
        }
        self.host_session_epoch = None;
    }

    pub fn is_running(&self) -> bool {
        self.lifecycle == PaneLifecycle::Running
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
    fn start_performance_probe(&mut self, cx: &mut Context<Self>) {
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
            terminal.start_performance_input_probe(Duration::from_millis(delay_ms), start_file, cx);
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
        self.start_performance_probe(cx);
        let ui_style = current_ui_style(cx);
        let body = if let Some(terminal) = &self.terminal {
            div().flex().flex_1().child(terminal.clone())
        } else {
            let lines = if matches!(
                self.lifecycle,
                PaneLifecycle::SpawnFailed { .. } | PaneLifecycle::Lost { .. }
            ) {
                spawn_failure_lines(&TerminalSpawnFailure {
                    command: self.command.clone(),
                    cwd: self.project_path.clone(),
                    message: terminal_start_error(&self.lifecycle, &self.terminal_error),
                })
            } else {
                vec![
                    format!("Process {}", pane_lifecycle_label(&self.lifecycle)),
                    format!("command: {}", self.command),
                    format!("cwd: {}", self.project_path.display()),
                ]
            };
            let can_restart = self.exit_behavior != ProcessExitBehavior::Close;
            let restart_id = SharedString::from(format!("restart-pane-{}", self.pane_id));

            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.md)
                .flex_1()
                .items_center()
                .justify_center()
                .bg(self.theme.terminal_background)
                .text_color(self.theme.danger)
                .children(lines)
                .when(can_restart, |body| {
                    body.child(
                        div()
                            .id(restart_id)
                            .cursor_pointer()
                            .rounded(ui_style.radius.action)
                            .border(ui_style.border.hairline)
                            .border_color(self.theme.border)
                            .bg(self.theme.surface)
                            .px(ui_style.spacing.lg)
                            .py(ui_style.spacing.xs)
                            .text_color(self.theme.text)
                            .hover(|button| button.bg(ui_style.hover_background(self.theme)))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.start_terminal(window, cx);
                            }))
                            .child("Restart"),
                    )
                })
        };

        div().flex().flex_col().flex_1().child(body)
    }
}

pub fn pane_lifecycle_label(lifecycle: &PaneLifecycle) -> String {
    match lifecycle {
        PaneLifecycle::Idle => "idle".to_string(),
        PaneLifecycle::Starting => "starting".to_string(),
        PaneLifecycle::Running => "running".to_string(),
        PaneLifecycle::Stopping { .. } => "stopping".to_string(),
        PaneLifecycle::Exited {
            code: Some(code), ..
        } => format!("exited {code}"),
        PaneLifecycle::Exited {
            code: None,
            reason: ExitReason::KilledByUser,
        } => "killed".to_string(),
        PaneLifecycle::Exited { code: None, .. } => "exited".to_string(),
        PaneLifecycle::SpawnFailed { .. } => "spawn failed".to_string(),
        PaneLifecycle::Lost { .. } => "lost".to_string(),
    }
}

pub fn spawn_failure_lines(failure: &TerminalSpawnFailure) -> Vec<String> {
    vec![
        "Failed to start terminal".to_string(),
        format!("command: {}", failure.command),
        format!("cwd: {}", failure.cwd.display()),
        format!("error: {}", failure.message),
    ]
}

fn remove_inherited_agent_hook_environment(environment: &mut BTreeMap<String, String>) {
    for name in AGENT_HOOK_ENVIRONMENT_VARIABLES {
        environment.remove(name);
    }
}

fn terminal_start_error(lifecycle: &PaneLifecycle, terminal_error: &Option<String>) -> String {
    match lifecycle {
        PaneLifecycle::SpawnFailed { message } | PaneLifecycle::Lost { message } => message.clone(),
        _ => terminal_error
            .clone()
            .unwrap_or_else(|| "terminal did not start".to_string()),
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

    #[test]
    fn terminal_runtime_title_reset_restores_configured_default() {
        let configured = "Configured shell";
        assert_eq!(
            resolved_terminal_title(configured, "vim main.rs"),
            "vim main.rs"
        );
        assert_eq!(resolved_terminal_title(configured, ""), configured);
        assert_eq!(configured, "Configured shell");
    }

    #[test]
    fn terminal_title_changed_event_contains_only_runtime_identity() {
        let event = TerminalPaneEvent::TitleChanged {
            pane_id: "shell".to_string(),
            title: "runtime".to_string(),
        };
        assert_eq!(
            event,
            TerminalPaneEvent::TitleChanged {
                pane_id: "shell".to_string(),
                title: "runtime".to_string(),
            }
        );
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
            shell: "/bin/sh".to_string(),
            environment: Arc::new(RwLock::new(BTreeMap::new())),
            is_focused: true,
            terminal_input_gate: TerminalInputGate::default(),
            ssh: None,
            agent_launch: None,
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
            assert!(matches!(
                &pane.lifecycle,
                PaneLifecycle::SpawnFailed { message } if message == "Host runtime is unavailable"
            ));
        });
    }

    #[test]
    fn inherited_agent_hook_credentials_are_removed_from_terminal_environment() {
        let mut environment = BTreeMap::from([
            ("PATH".to_string(), "/usr/bin".to_string()),
            (
                "YTTT_AGENT_HOOK_ENDPOINT".to_string(),
                "http://outer-yttt".to_string(),
            ),
            (
                "YTTT_AGENT_HOOK_TOKEN".to_string(),
                "outer-token".to_string(),
            ),
            (
                "YTTT_AGENT_HOOK_SCOPE".to_string(),
                "outer-scope".to_string(),
            ),
        ]);

        remove_inherited_agent_hook_environment(&mut environment);

        assert_eq!(
            environment.get("PATH").map(String::as_str),
            Some("/usr/bin")
        );
        for name in AGENT_HOOK_ENVIRONMENT_VARIABLES {
            assert!(!environment.contains_key(name));
        }
    }

    #[test]
    fn terminal_pane_io_error_lifecycle_is_single_shot() {
        let message = terminal_io_error_message(PtyIoOperation::Read, "broken pipe");
        let lifecycle = PaneLifecycle::Running;
        let event = TerminalPaneEvent::IoError {
            pane_id: "shell".to_string(),
            message: message.clone(),
            fatal: true,
        };
        assert_eq!(lifecycle, PaneLifecycle::Running);
        assert_eq!(message, "Terminal Read error: broken pipe");
        assert!(matches!(
            event,
            TerminalPaneEvent::IoError { fatal: true, .. }
        ));

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
