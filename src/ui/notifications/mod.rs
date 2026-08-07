use crate::{
    runtime::notification::{NotificationEvent, NotificationKind},
    ui::i18n::{UiText, UiTextKey},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastTone {
    Success,
    Warning,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToastItem {
    pub title: String,
    pub status: Option<String>,
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

pub fn visible_toast_items(queue: &ToastQueue, ui_text: &UiText) -> Vec<ToastItem> {
    queue
        .events()
        .iter()
        .rev()
        .take(3)
        .map(|event| toast_item_for_event(event, ui_text))
        .collect()
}

pub fn toast_item_for_event(event: &NotificationEvent, ui_text: &UiText) -> ToastItem {
    let (status_key, tone) = match event.kind {
        NotificationKind::AgentWaiting => {
            (UiTextKey::PaletteStatusAgentWaiting, ToastTone::Warning)
        }
        NotificationKind::AgentCompleted => {
            (UiTextKey::PaletteStatusAgentCompleted, ToastTone::Success)
        }
        NotificationKind::AgentFailed => (UiTextKey::PaletteStatusAgentFailed, ToastTone::Error),
    };

    ToastItem {
        title: event
            .summary
            .as_deref()
            .unwrap_or(&event.pane_title)
            .to_string(),
        status: Some(ui_text.get(status_key).to_string()),
        context: event.context(),
        tone,
    }
}
