use std::path::PathBuf;

use gpui::{
    Context, Edges, Entity, EventEmitter, IntoElement, Render, Window, div, prelude::*, px, rgb,
};
use gpui_terminal::{ColorPalette, TerminalConfig, TerminalView};

use crate::{
    model::layout::{PaneConfig, PaneKind},
    runtime::{
        agent::classify_agent,
        notification::{ExitNotificationInput, NotificationEvent, notification_for_exit},
        terminal::{
            ExitReason, PortablePtySession, ProcessStatus, TerminalSpawnRequest,
            spawn_portable_pty_session,
        },
    },
};

#[derive(Clone, Debug)]
pub struct TerminalPaneContext {
    pub project_id: String,
    pub project_path: PathBuf,
    pub project_title: String,
    pub tab_id: String,
    pub tab_title: String,
    pub pane: PaneConfig,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalPaneEvent {
    Notification(NotificationEvent),
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
    session: Option<PortablePtySession>,
    lifecycle: PaneLifecycle,
    launch_error: Option<String>,
    exit_emitted: bool,
}

impl TerminalPaneView {
    pub fn new(context: TerminalPaneContext, cx: &mut Context<Self>) -> Self {
        let TerminalPaneContext {
            project_id,
            project_path,
            project_title,
            tab_id,
            tab_title,
            pane,
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
        let terminal = cx.new(|cx| {
            TerminalView::new(io.writer, io.reader, terminal_config(), cx)
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
    }

    pub fn focus_terminal(&self, window: &Window, cx: &mut Context<Self>) -> bool {
        let Some(terminal) = self.terminal.clone() else {
            return false;
        };

        cx.defer_in(window, move |_this, window, cx| {
            terminal.read(cx).focus_handle().focus(window);
        });
        true
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
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .flex_none()
            .border_b_1()
            .border_color(rgb(0x2a2a2a))
            .bg(rgb(0x171717))
            .px_2()
            .py_1()
            .text_xs()
            .text_color(rgb(0xd4d4d4))
            .child(
                div()
                    .truncate()
                    .child(format!("{} · {}", self.title, self.pane_id)),
            )
            .child(
                div()
                    .truncate()
                    .text_color(rgb(0x737373))
                    .child(self.command.clone()),
            )
            .child(
                div()
                    .flex_none()
                    .text_color(lifecycle_color(&self.lifecycle))
                    .child(pane_lifecycle_label(&self.lifecycle)),
            );

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
                .bg(rgb(0x111111))
                .text_color(rgb(0xef4444))
                .children(spawn_failure_lines(&failure))
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .bg(rgb(0x111111))
            .child(header)
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

fn lifecycle_color(lifecycle: &PaneLifecycle) -> gpui::Rgba {
    match lifecycle {
        PaneLifecycle::Running => rgb(0x22c55e),
        PaneLifecycle::Exited { code: Some(0), .. } => rgb(0xa3a3a3),
        PaneLifecycle::Exited {
            reason: ExitReason::KilledByUser,
            ..
        } => rgb(0xa3a3a3),
        PaneLifecycle::Exited { .. } | PaneLifecycle::SpawnFailed { .. } => rgb(0xef4444),
        PaneLifecycle::Idle | PaneLifecycle::Starting => rgb(0xf59e0b),
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

fn terminal_config() -> TerminalConfig {
    TerminalConfig {
        font_family: "monospace".into(),
        font_size: px(13.0),
        cols: 80,
        rows: 24,
        scrollback: 10000,
        line_height_multiplier: 1.15,
        padding: Edges::all(px(6.0)),
        colors: ColorPalette::default(),
    }
}
