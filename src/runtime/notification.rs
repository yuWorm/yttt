use yttt_terminal::ExitReason;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotificationKind {
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
    })
}
