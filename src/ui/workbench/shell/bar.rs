use gpui::{AnyElement, Div, div, prelude::*, relative};

use crate::ui::theme::UiStyle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BarHost {
    Window,
    Status,
}

#[derive(Default)]
pub struct BarSections {
    pub left: Vec<AnyElement>,
    pub center: Vec<AnyElement>,
    pub right: Vec<AnyElement>,
}

pub fn bar_sections_content(sections: BarSections, host: BarHost, ui_style: UiStyle) -> Div {
    let center_is_empty = sections.center.is_empty();
    let (left_id, center_id, right_id) = match host {
        BarHost::Window => ("window-bar-left", "window-bar-center", "window-bar-right"),
        BarHost::Status => ("status-bar-left", "status-bar-center", "status-bar-right"),
    };
    let left = bar_group(left_id, sections.left, ui_style);
    let center = bar_group(center_id, sections.center, ui_style);
    let right = bar_group(right_id, sections.right, ui_style).justify_end();

    let content = div()
        .flex()
        .items_center()
        .size_full()
        .min_w_0()
        .overflow_hidden();
    if center_is_empty {
        content
            .child(left.flex_1())
            .child(right.flex_none().max_w(relative(0.6)))
    } else {
        content
            .child(left.flex_1())
            .child(center.flex_none().max_w(relative(0.4)))
            .child(right.flex_1())
    }
}

fn bar_group(id: &'static str, modules: Vec<AnyElement>, ui_style: UiStyle) -> Div {
    div()
        .debug_selector(move || id.to_string())
        .flex()
        .items_center()
        .min_w_0()
        .overflow_hidden()
        .gap(ui_style.spacing.sm)
        .children(modules)
}
