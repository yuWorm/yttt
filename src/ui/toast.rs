use gpui::{Div, IntoElement, div, prelude::*, rgb, rgba};

use crate::runtime::notification::{NotificationEvent, NotificationKind};

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

pub fn toast_overlay(queue: &ToastQueue) -> impl IntoElement {
    if queue.events().is_empty() {
        return div();
    }

    queue.events().iter().rev().take(3).fold(
        div().absolute().top_4().right_4().flex().flex_col().gap_2(),
        |list, event| list.child(toast_item(event)),
    )
}

fn toast_item(event: &NotificationEvent) -> Div {
    let accent = match event.kind {
        NotificationKind::AgentCompleted => rgb(0x22c55e),
        NotificationKind::AgentFailed => rgb(0xef4444),
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
        .child(div().text_sm().text_color(accent).child(toast_title(event)))
        .child(
            div()
                .text_xs()
                .text_color(rgb(0xa3a3a3))
                .child(format!("{} / {}", event.project_title, event.tab_title)),
        )
}

fn toast_title(event: &NotificationEvent) -> String {
    match event.kind {
        NotificationKind::AgentCompleted => format!("{} completed", event.pane_title),
        NotificationKind::AgentFailed => format!("{} failed", event.pane_title),
    }
}
