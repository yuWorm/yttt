use gpui::{Div, IntoElement, div, prelude::*, rgb, rgba};
use gpui_component::{Icon, IconName};

use crate::runtime::notification::{NotificationEvent, NotificationKind};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ToastTone {
    Success,
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
        self.events.iter().map(toast_title).collect()
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
        .map(|event| ToastItem {
            title: toast_title(event),
            context: format!("{} / {}", event.project_title, event.tab_title),
            tone: match event.kind {
                NotificationKind::AgentCompleted => ToastTone::Success,
                NotificationKind::AgentFailed => ToastTone::Error,
            },
        })
        .collect()
}

pub fn toast_overlay(queue: &ToastQueue) -> impl IntoElement {
    let items = visible_toast_items(queue);
    if items.is_empty() {
        return div();
    }

    items.into_iter().fold(
        div().absolute().top_4().right_4().flex().flex_col().gap_2(),
        |list, item| list.child(toast_item(item)),
    )
}

fn toast_item(item: ToastItem) -> Div {
    let (accent, icon) = match item.tone {
        ToastTone::Success => (rgb(0x22c55e), IconName::CircleCheck),
        ToastTone::Error => (rgb(0xef4444), IconName::CircleX),
    };

    div()
        .flex()
        .flex_col()
        .gap_1()
        .w_80()
        .rounded_md()
        .border_1()
        .border_color(rgb(0x2a2a2a))
        .bg(rgba(0x151515ee))
        .p_3()
        .text_color(rgb(0xf5f5f5))
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .text_sm()
                .text_color(accent)
                .child(Icon::new(icon).size_4().text_color(accent))
                .child(item.title),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(0xa3a3a3))
                .child(item.context),
        )
}

fn toast_title(event: &NotificationEvent) -> String {
    match event.kind {
        NotificationKind::AgentCompleted => format!("{} completed", event.pane_title),
        NotificationKind::AgentFailed => format!("{} failed", event.pane_title),
    }
}
