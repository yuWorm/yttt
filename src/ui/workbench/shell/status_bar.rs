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
        .px(ui_style.spacing.md)
        .border_t(ui_style.border.hairline)
        .border_color(theme.border)
        .bg(theme.surface_elevated)
        .line_height(relative(1.0))
        .text_xs()
        .text_color(theme.text_muted)
        .child(bar_sections_content(sections, BarHost::Status, ui_style))
}
