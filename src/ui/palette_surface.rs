use gpui::{Pixels, Rgba, px};

use crate::{
    palette::PaletteKind,
    ui::{components::SelectableState, theme::WorkbenchTheme},
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PalettePanelStyle {
    pub width: Pixels,
    pub max_width: Pixels,
    pub max_height: Pixels,
    pub list_max_height: Pixels,
    pub row_height: Pixels,
    pub footer_height: Pixels,
    pub border_width: Pixels,
    pub scrollable: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteRowTone {
    Active,
    Inactive,
    Disabled,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PaletteRowStyle {
    pub tone: PaletteRowTone,
    pub background: Rgba,
    pub hover_background: Rgba,
    pub border: Rgba,
    pub title: Rgba,
    pub subtitle: Rgba,
    pub status: Rgba,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteFooterAction {
    pub label: &'static str,
    pub key: &'static str,
}

pub fn palette_panel_style() -> PalettePanelStyle {
    PalettePanelStyle {
        width: px(760.0),
        max_width: px(900.0),
        max_height: px(480.0),
        list_max_height: px(376.0),
        row_height: px(54.0),
        footer_height: px(44.0),
        border_width: px(1.0),
        scrollable: true,
    }
}

pub fn palette_scroll_anchor_index(selected_index: usize) -> Option<usize> {
    (selected_index > 0).then(|| selected_index.saturating_sub(4))
}

pub fn palette_row_style(
    state: SelectableState,
    enabled: bool,
    theme: WorkbenchTheme,
) -> PaletteRowStyle {
    if !enabled {
        return PaletteRowStyle {
            tone: PaletteRowTone::Disabled,
            background: theme.surface_elevated,
            hover_background: theme.surface_elevated,
            border: theme.surface_elevated,
            title: theme.text_subtle,
            subtitle: theme.text_subtle,
            status: theme.text_subtle,
        };
    }

    match state {
        SelectableState::Active => PaletteRowStyle {
            tone: PaletteRowTone::Active,
            background: theme.active_surface,
            hover_background: theme.active_surface,
            border: theme.active_surface,
            title: theme.text,
            subtitle: theme.text_muted,
            status: theme.text_muted,
        },
        SelectableState::Inactive => PaletteRowStyle {
            tone: PaletteRowTone::Inactive,
            background: theme.surface_elevated,
            hover_background: theme.hover_surface,
            border: theme.surface_elevated,
            title: theme.text_muted,
            subtitle: theme.text_subtle,
            status: theme.text_muted,
        },
    }
}

pub fn palette_footer_actions() -> Vec<PaletteFooterAction> {
    vec![
        PaletteFooterAction {
            label: "Run",
            key: "enter",
        },
        PaletteFooterAction {
            label: "Close",
            key: "esc",
        },
    ]
}

pub fn palette_input_placeholder(kind: PaletteKind) -> &'static str {
    match kind {
        PaletteKind::Command => "Execute a command...",
        PaletteKind::Project => "Switch project...",
        PaletteKind::Tab => "Switch tab...",
        PaletteKind::Pane => "Switch pane...",
    }
}
