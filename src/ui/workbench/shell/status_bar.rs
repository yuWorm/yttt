use gpui::{Div, div, prelude::*, relative};

use crate::ui::theme::{UiStyle, WorkbenchTheme};

use super::bar::{BarHost, BarSections, bar_sections_content};

pub fn workbench_status_bar(
    sections: BarSections,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    div()
        .debug_selector(|| "status-bar".to_string())
        .flex()
        .flex_none()
        .items_center()
        .h(ui_style.controls.status_bar_height)
        .px(ui_style.shell.statusbar_padding_x)
        .border_t(ui_style.border.hairline)
        .border_color(theme.border)
        .bg(theme.statusbar_background)
        .line_height(relative(1.0))
        .text_sm()
        .text_color(theme.text)
        .child(bar_sections_content(sections, BarHost::Status, ui_style))
}
