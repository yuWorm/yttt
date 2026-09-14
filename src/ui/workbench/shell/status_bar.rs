use gpui::{Div, div, prelude::*};

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
        .px(ui_style.shell.statusbar_padding_x)
        .py(ui_style.shell.statusbar_padding_y)
        .border_t(ui_style.border.hairline)
        .border_color(theme.border)
        .bg(theme.statusbar_background)
        .text_sm()
        .text_color(theme.text)
        .child(bar_sections_content(sections, BarHost::Status, ui_style))
}
