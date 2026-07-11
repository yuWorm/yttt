use std::{path::PathBuf, sync::atomic::Ordering, time::Duration};

use gpui::{
    Context, Entity, EventEmitter, IntoElement, Render, SharedString, Window, div, prelude::*,
};
use yttt_terminal::{
    ExitReason, PortablePtySession, ProcessStatus, TerminalConfig, TerminalSpawnRequest,
    TerminalView, spawn_portable_pty_session,
};

use crate::{
    model::layout::{PaneConfig, PaneKind, ProcessExitBehavior, TerminalExecutionMode},
    runtime::{
        agent::classify_agent,
        notification::{ExitNotificationInput, NotificationEvent, notification_for_exit},
    },
    ui::{interaction::input_owner::TerminalInputGate, theme::WorkbenchTheme},
};

#[derive(Clone, Debug)]
pub struct TerminalPaneContext {
    pub project_id: String,
    pub project_path: PathBuf,
    pub project_title: String,
    pub tab_id: String,
    pub tab_title: String,
    pub pane: PaneConfig,
    pub shell: String,
    pub is_focused: bool,
    pub terminal_input_gate: TerminalInputGate,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalPaneEvent {
    Notification(NotificationEvent),
    Started(TerminalPaneStartedEvent),
    Exited(TerminalPaneExitedEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneStartedEvent {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneExitedEvent {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub status: ProcessStatus,
    pub exit_reason: ExitReason,
    pub exit_behavior: ProcessExitBehavior,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PaneLifecycle {
    Idle,
    Starting,
    Running,
    Exited {
        code: Option<i32>,
        reason: ExitReason,
    },
    SpawnFailed {
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
    title: String,
    command: String,
    kind: PaneKind,
    args: Vec<String>,
    execution_mode: TerminalExecutionMode,
    exit_behavior: ProcessExitBehavior,
    shell: String,
    notify_on_exit: bool,
    terminal: Option<Entity<TerminalView>>,
    terminal_config: TerminalConfig,
    theme: WorkbenchTheme,
    session: Option<PortablePtySession>,
    lifecycle: PaneLifecycle,
    launch_error: Option<String>,
    exit_emitted: bool,
    terminal_input_gate: TerminalInputGate,
    generation: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalPaneChrome {
    pub shows_header: bool,
}

impl TerminalPaneView {
    pub fn new(
        context: TerminalPaneContext,
        terminal_config: TerminalConfig,
        theme: WorkbenchTheme,
        cx: &mut Context<Self>,
    ) -> Self {
        let TerminalPaneContext {
            project_id,
            project_path,
            project_title,
            tab_id,
            tab_title,
            pane,
            shell,
            is_focused: _,
            terminal_input_gate,
        } = context;
        let mut view = Self {
            project_id,
            project_path,
            project_title,
            tab_id,
            tab_title,
            pane_id: pane.id,
            title: pane.title,
            command: pane.command,
            args: pane.args,
            execution_mode: pane.execution_mode,
            exit_behavior: pane.exit_behavior,
            shell,
            kind: pane.kind,
            notify_on_exit: pane.notify_on_exit,
            terminal: None,
            terminal_config,
            theme,
            session: None,
            lifecycle: PaneLifecycle::Idle,
            launch_error: None,
            exit_emitted: false,
            terminal_input_gate,
            generation: 0,
        };
        view.start_terminal(cx);
        view
    }

    fn spawn_request(&self) -> TerminalSpawnRequest {
        let request = match self.execution_mode {
            TerminalExecutionMode::Shell => {
                TerminalSpawnRequest::for_shell(&self.pane_id, &self.shell, &self.command)
            }
            TerminalExecutionMode::Command => {
                TerminalSpawnRequest::for_command(&self.pane_id, &self.command, self.args.clone())
            }
        };
        request.cwd(self.project_path.clone())
    }

    fn start_terminal(&mut self, cx: &mut Context<Self>) -> bool {
        if let Some(mut session) = self.session.take() {
            let _ = session.kill();
        }
        self.terminal = None;
        self.lifecycle = PaneLifecycle::Starting;
        self.launch_error = None;
        self.exit_emitted = false;
        self.generation = self.generation.wrapping_add(1);

        let mut session = match spawn_portable_pty_session(self.spawn_request()) {
            Ok(session) => session,
            Err(error) => {
                self.set_spawn_failure(error.to_string(), cx);
                return false;
            }
        };
        let Some(io) = session.take_io() else {
            self.set_spawn_failure("pty session I/O was already taken".to_string(), cx);
            return false;
        };

        let resize_handle = session.resize_handle();
        let parent = cx.weak_entity();
        let generation = self.generation;
        let initial_config = self.terminal_config.clone();
        let terminal_input_allowed = self.terminal_input_gate.shared_flag();
        let terminal = cx.new(|cx| {
            TerminalView::new(io.writer, io.reader, initial_config, cx)
                .with_key_handler(move |_event| !terminal_input_allowed.load(Ordering::SeqCst))
                .with_resize_callback(move |cols, rows| {
                    let _ = resize_handle.resize(cols, rows);
                })
                .with_exit_callback(move |_window, cx| {
                    let _ = parent.update(cx, |pane, cx| {
                        pane.handle_process_exit(generation, ExitReason::Completed, cx);
                    });
                })
        });

        self.terminal = Some(terminal);
        self.session = Some(session);
        self.lifecycle = PaneLifecycle::Running;
        cx.emit(TerminalPaneEvent::Started(TerminalPaneStartedEvent {
            project_id: self.project_id.clone(),
            tab_id: self.tab_id.clone(),
            pane_id: self.pane_id.clone(),
        }));
        cx.notify();
        true
    }

    fn set_spawn_failure(&mut self, message: String, cx: &mut Context<Self>) {
        self.lifecycle = PaneLifecycle::SpawnFailed {
            message: message.clone(),
        };
        self.launch_error = Some(message);
        self.session = None;
        self.terminal = None;
        cx.notify();
    }

    fn handle_process_exit(
        &mut self,
        generation: u64,
        exit_reason: ExitReason,
        cx: &mut Context<Self>,
    ) {
        if generation != self.generation || self.exit_emitted {
            return;
        }
        self.exit_emitted = true;

        let status = self
            .session
            .as_mut()
            .map(|session| session.status())
            .unwrap_or(ProcessStatus::Exited { code: None });
        let code = match status {
            ProcessStatus::Running => None,
            ProcessStatus::Exited { code } => code,
        };
        self.lifecycle = PaneLifecycle::Exited {
            code,
            reason: exit_reason,
        };
        self.terminal = None;
        self.session = None;

        let exit_event = TerminalPaneExitedEvent {
            project_id: self.project_id.clone(),
            tab_id: self.tab_id.clone(),
            pane_id: self.pane_id.clone(),
            status,
            exit_reason,
            exit_behavior: self.exit_behavior,
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
        cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_millis(500))
                .await;
            let _ = this.update(cx, |pane, cx| {
                if pane.generation == generation
                    && pane.exit_behavior == ProcessExitBehavior::AutoRestart
                    && matches!(pane.lifecycle, PaneLifecycle::Exited { .. })
                {
                    pane.start_terminal(cx);
                }
            });
        })
        .detach();
    }

    pub fn is_running(&self) -> bool {
        self.lifecycle == PaneLifecycle::Running
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
}

impl EventEmitter<TerminalPaneEvent> for TerminalPaneView {}

impl Drop for TerminalPaneView {
    fn drop(&mut self) {
        if let Some(session) = &mut self.session {
            let _ = session.kill();
        }
    }
}

impl Render for TerminalPaneView {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = if let Some(terminal) = &self.terminal {
            div().flex().flex_1().child(terminal.clone())
        } else {
            let lines = if matches!(self.lifecycle, PaneLifecycle::SpawnFailed { .. }) {
                spawn_failure_lines(&TerminalSpawnFailure {
                    command: self.command.clone(),
                    cwd: self.project_path.clone(),
                    message: terminal_start_error(&self.lifecycle, &self.launch_error),
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
                .gap_2()
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
                            .rounded_sm()
                            .border_1()
                            .border_color(self.theme.border)
                            .bg(self.theme.surface)
                            .px_3()
                            .py_1()
                            .text_color(self.theme.text)
                            .hover(|button| button.bg(self.theme.hover_surface))
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.start_terminal(cx);
                            }))
                            .child("Restart"),
                    )
                })
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .bg(self.theme.terminal_background)
            .child(body)
    }
}

pub fn pane_lifecycle_label(lifecycle: &PaneLifecycle) -> String {
    match lifecycle {
        PaneLifecycle::Idle => "idle".to_string(),
        PaneLifecycle::Starting => "starting".to_string(),
        PaneLifecycle::Running => "running".to_string(),
        PaneLifecycle::Exited {
            code: Some(code), ..
        } => format!("exited {code}"),
        PaneLifecycle::Exited {
            code: None,
            reason: ExitReason::KilledByUser,
        } => "killed".to_string(),
        PaneLifecycle::Exited { code: None, .. } => "exited".to_string(),
        PaneLifecycle::SpawnFailed { .. } => "spawn failed".to_string(),
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

fn terminal_start_error(lifecycle: &PaneLifecycle, launch_error: &Option<String>) -> String {
    match lifecycle {
        PaneLifecycle::SpawnFailed { message } => message.clone(),
        _ => launch_error
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
