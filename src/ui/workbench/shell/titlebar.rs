use gpui::{IntoElement, div, prelude::*};
use gpui_component::TitleBar;

use crate::ui::theme::{UiStyle, WorkbenchTheme};

use super::bar::{BarHost, BarSections, bar_sections_content};

pub fn compact_path_for_titlebar(path: &str) -> String {
    const MAX_LEN: usize = 48;
    let path = if let Some(path) = path.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{path}")
    } else {
        path.strip_prefix(r"\\?\").unwrap_or(path).to_string()
    };
    if path.chars().count() <= MAX_LEN {
        return path;
    }

    let separator = if path.rfind('\\') > path.rfind('/') {
        '\\'
    } else {
        '/'
    };
    let mut parts = path.rsplit(['/', '\\']).filter(|part| !part.is_empty());
    let tail = parts.next().unwrap_or(&path);
    let parent = parts.next();

    match parent {
        Some(parent) => format!("...{separator}{parent}{separator}{tail}"),
        None => format!("...{separator}{tail}"),
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
