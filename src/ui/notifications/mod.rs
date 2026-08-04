use crate::runtime::notification::{NotificationEvent, NotificationKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastTone {
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastItem {
    pub title: String,
    pub context: String,
    pub tone: ToastTone,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ToastQueue {
    events: Vec<NotificationEvent>,
}

impl ToastQueue {
    pub fn push(&mut self, event: NotificationEvent) {
        self.events.push(event);
    }

    pub fn titles(&self) -> Vec<String> {
        self.events.iter().map(NotificationEvent::title).collect()
    }

    pub fn events(&self) -> &[NotificationEvent] {
        &self.events
    }
}

pub fn visible_toast_items(queue: &ToastQueue) -> Vec<ToastItem> {
    queue
        .events()
        .iter()
        .rev()
        .take(3)
        .map(toast_item_for_event)
        .collect()
}

pub fn toast_item_for_event(event: &NotificationEvent) -> ToastItem {
    ToastItem {
        title: event.title(),
        context: event.context(),
        tone: match event.kind {
            NotificationKind::AgentWaiting => ToastTone::Warning,
            NotificationKind::AgentCompleted => ToastTone::Success,
            NotificationKind::AgentFailed => ToastTone::Error,
        },
    }
}
