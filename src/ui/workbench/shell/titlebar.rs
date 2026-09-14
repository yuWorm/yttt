use gpui::{IntoElement, Window, div, prelude::*, px};
use gpui_component::TitleBar;

use crate::ui::theme::{UiStyle, UiStyleId, WorkbenchTheme};

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
    window: &Window,
) -> impl IntoElement {
    TitleBar::new()
        .bg(if window.is_window_active() {
            theme.titlebar_background
        } else {
            theme.titlebar_inactive_background
        })
        .when(ui_style.id == UiStyleId::Zed, |this| {
            this.h(if cfg!(target_os = "windows") {
                px(32.0)
            } else {
                (window.rem_size() * 1.75).max(px(34.0))
            })
        })
        .border_color(theme.border)
        .child(
            div()
                .debug_selector(|| "window-bar".to_string())
                .flex()
                .items_center()
                .size_full()
                .min_w_0()
                .px(ui_style.shell.titlebar_padding_x)
                .text_sm()
                .text_color(theme.text)
                .child(bar_sections_content(sections, BarHost::Window, ui_style)),
        )
}
