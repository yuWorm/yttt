use std::{path::PathBuf, sync::atomic::Ordering};

use gpui::{Context, Entity, EventEmitter, IntoElement, Render, Window, div, prelude::*};
use yttt_terminal::{
    ExitReason, PortablePtySession, ProcessStatus, TerminalConfig, TerminalSpawnRequest,
    TerminalView, spawn_portable_pty_session,
};

use crate::{
    model::layout::{PaneConfig, PaneKind},
    runtime::{
        agent::classify_agent,
        notification::{ExitNotificationInput, NotificationEvent, notification_for_exit},
    },
    ui::{input_owner::TerminalInputGate, theme::WorkbenchTheme},
};

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
    Exited(TerminalPaneExitedEvent),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TerminalPaneExitedEvent {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub status: ProcessStatus,
    pub exit_reason: ExitReason,
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
    notify_on_exit: bool,
    terminal: Option<Entity<TerminalView>>,
    terminal_config: TerminalConfig,
    theme: WorkbenchTheme,
    session: Option<PortablePtySession>,
    lifecycle: PaneLifecycle,
    launch_error: Option<String>,
    exit_emitted: bool,
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
            is_focused: _,
            terminal_input_gate,
        } = context;
        let mut session = match spawn_portable_pty_session(
            TerminalSpawnRequest::for_shell(&pane.id, &pane.command).cwd(project_path.clone()),
        ) {
            Ok(session) => session,
            Err(error) => {
                let message = error.to_string();
                return Self {
                    project_id,
                    project_path,
                    project_title,
                    tab_id,
                    tab_title,
                    pane_id: pane.id,
                    title: pane.title,
                    command: pane.command,
                    kind: pane.kind,
                    notify_on_exit: pane.notify_on_exit,
                    terminal: None,
                    terminal_config,
                    theme,
                    session: None,
                    lifecycle: PaneLifecycle::SpawnFailed {
                        message: message.clone(),
                    },
                    launch_error: Some(message),
                    exit_emitted: false,
                };
            }
        };

        let Some(io) = session.take_io() else {
            let message = "pty session I/O was already taken".to_string();
            return Self {
                project_id,
                project_path,
                project_title,
                tab_id,
                tab_title,
                pane_id: pane.id,
                title: pane.title,
                command: pane.command,
                kind: pane.kind,
                notify_on_exit: pane.notify_on_exit,
                terminal: None,
                terminal_config,
                theme,
                session: Some(session),
                lifecycle: PaneLifecycle::SpawnFailed {
                    message: message.clone(),
                },
                launch_error: Some(message),
                exit_emitted: false,
            };
        };

        let resize_handle = session.resize_handle();
        let parent = cx.weak_entity();
        let initial_config = terminal_config.clone();
        let terminal_input_allowed = terminal_input_gate.shared_flag();
        let terminal = cx.new(|cx| {
            TerminalView::new(io.writer, io.reader, initial_config, cx)
                .with_key_handler(move |_event| !terminal_input_allowed.load(Ordering::SeqCst))
                .with_resize_callback(move |cols, rows| {
                    let _ = resize_handle.resize(cols, rows);
                })
                .with_exit_callback(move |_window, cx| {
                    let _ = parent.update(cx, |pane, cx| {
                        pane.emit_exit_notification(ExitReason::Completed, cx);
                    });
                })
        });

        Self {
            project_id,
            project_path,
            project_title,
            tab_id,
            tab_title,
            pane_id: pane.id,
            title: pane.title,
            command: pane.command,
            kind: pane.kind,
            notify_on_exit: pane.notify_on_exit,
            terminal: Some(terminal),
            terminal_config,
            theme,
            session: Some(session),
            lifecycle: PaneLifecycle::Running,
            launch_error: None,
            exit_emitted: false,
        }
    }

    fn emit_exit_notification(&mut self, exit_reason: ExitReason, cx: &mut Context<Self>) {
        if self.exit_emitted {
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
        let exit_event = TerminalPaneExitedEvent {
            project_id: self.project_id.clone(),
            tab_id: self.tab_id.clone(),
            pane_id: self.pane_id.clone(),
            status: status.clone(),
            exit_reason,
        };
        let event = notification_for_terminal_pane_exit(TerminalPaneExitInput {
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

        if let Some(event) = event {
            cx.emit(TerminalPaneEvent::Notification(event));
        }
        cx.emit(TerminalPaneEvent::Exited(exit_event));
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
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let body = if let Some(terminal) = &self.terminal {
            div().flex().flex_1().child(terminal.clone())
        } else {
            let failure = TerminalSpawnFailure {
                command: self.command.clone(),
                cwd: self.project_path.clone(),
                message: terminal_start_error(&self.lifecycle, &self.launch_error),
            };

            div()
                .flex()
                .flex_col()
                .gap_1()
                .flex_1()
                .items_center()
                .justify_center()
                .bg(self.theme.terminal_background)
                .text_color(self.theme.danger)
                .children(spawn_failure_lines(&failure))
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
