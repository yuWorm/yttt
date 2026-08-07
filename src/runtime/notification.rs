use std::thread;

use notify_rust::Notification;
use yttt_agent_core::AgentViewState;
use yttt_terminal::ExitReason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationKind {
    AgentWaiting,
    AgentCompleted,
    AgentFailed,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NotificationEvent {
    pub kind: NotificationKind,
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub project_title: String,
    pub tab_title: String,
    pub pane_title: String,
    pub summary: Option<String>,
}

impl NotificationEvent {
    pub fn title(&self) -> String {
        match self.kind {
            NotificationKind::AgentWaiting => format!("{} needs attention", self.pane_title),
            NotificationKind::AgentCompleted => format!("{} completed", self.pane_title),
            NotificationKind::AgentFailed => format!("{} failed", self.pane_title),
        }
    }

    pub fn context(&self) -> String {
        if self.tab_title == self.pane_title {
            format!("{} › {}", self.project_title, self.tab_title)
        } else {
            format!(
                "{} › {} › {}",
                self.project_title, self.tab_title, self.pane_title
            )
        }
    }
}

pub trait SystemNotifier {
    fn notify(&self, event: &NotificationEvent) -> anyhow::Result<()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub struct NoopSystemNotifier;

impl SystemNotifier for NoopSystemNotifier {
    fn notify(&self, _event: &NotificationEvent) -> anyhow::Result<()> {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct DesktopSystemNotifier;

impl SystemNotifier for DesktopSystemNotifier {
    fn notify(&self, event: &NotificationEvent) -> anyhow::Result<()> {
        let title = event.title();
        let body = event.context();
        thread::Builder::new()
            .name("yttt-system-notification".to_string())
            .spawn(move || {
                let _ = Notification::new().summary(&title).body(&body).show();
            })?;
        Ok(())
    }
}

pub fn maybe_notify_system(
    notifier: &dyn SystemNotifier,
    enabled: bool,
    event: &NotificationEvent,
) -> anyhow::Result<bool> {
    if !enabled {
        return Ok(false);
    }

    notifier.notify(event)?;
    Ok(true)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AgentTransitionNotificationInput {
    pub previous_state: Option<AgentViewState>,
    pub state: AgentViewState,
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub project_title: String,
    pub tab_title: String,
    pub pane_title: String,
    pub summary: Option<String>,
}

pub fn notification_for_agent_transition(
    input: AgentTransitionNotificationInput,
) -> Option<NotificationEvent> {
    if input.previous_state == Some(input.state)
        || (matches!(
            input.previous_state,
            Some(AgentViewState::Completed | AgentViewState::Failed | AgentViewState::Interrupted)
        ) && matches!(
            input.state,
            AgentViewState::Completed | AgentViewState::Failed
        ))
    {
        return None;
    }
    let kind = match input.state {
        AgentViewState::Waiting => NotificationKind::AgentWaiting,
        AgentViewState::Completed => NotificationKind::AgentCompleted,
        AgentViewState::Failed => NotificationKind::AgentFailed,
        AgentViewState::Starting
        | AgentViewState::Idle
        | AgentViewState::Working
        | AgentViewState::Interrupted
        | AgentViewState::Stale => return None,
    };
    Some(NotificationEvent {
        kind,
        project_id: input.project_id,
        tab_id: input.tab_id,
        pane_id: input.pane_id,
        project_title: input.project_title,
        tab_title: input.tab_title,
        pane_title: input.pane_title,
        summary: input.summary,
    })
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExitNotificationInput {
    pub is_agent: bool,
    pub notify_on_exit: bool,
    pub exit_code: Option<i32>,
    pub exit_reason: ExitReason,
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub project_title: String,
    pub tab_title: String,
    pub pane_title: String,
}

pub fn notification_for_exit(input: ExitNotificationInput) -> Option<NotificationEvent> {
    if !input.is_agent || !input.notify_on_exit || input.exit_reason == ExitReason::KilledByUser {
        return None;
    }

    let kind = if input.exit_code == Some(0) {
        NotificationKind::AgentCompleted
    } else {
        NotificationKind::AgentFailed
    };

    Some(NotificationEvent {
        kind,
        project_id: input.project_id,
        tab_id: input.tab_id,
        pane_id: input.pane_id,
        project_title: input.project_title,
        tab_title: input.tab_title,
        pane_title: input.pane_title,
        summary: None,
    })
}
