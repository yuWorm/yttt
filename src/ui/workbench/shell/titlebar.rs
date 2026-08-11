use gpui::{IntoElement, div, prelude::*};
use gpui_component::TitleBar;

use crate::ui::theme::{UiStyle, WorkbenchTheme};

use super::bar::{BarHost, BarSections, bar_sections_content};

pub fn display_path_for_titlebar(path: &str) -> String {
    if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{path}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    }
}

pub fn workbench_titlebar(
    sections: BarSections,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> impl IntoElement {
    TitleBar::new()
        .bg(theme.titlebar_background)
        .border_color(theme.border)
        .child(
            div()
                .debug_selector(|| "window-bar".to_string())
                .flex()
                .items_center()
                .size_full()
                .min_w_0()
                .px(ui_style.spacing.lg)
                .text_sm()
                .text_color(theme.text)
                .child(bar_sections_content(sections, BarHost::Window, ui_style)),
        )
}
