use gpui::{Pixels, Rems, Rgba, px, rems};

use crate::{
    palette::PaletteKind,
    ui::{
        components::SelectableState,
        i18n::{UiText, UiTextKey},
        primitives::row::{YtttRowKind, yttt_row_style},
        theme::WorkbenchTheme,
    },
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PalettePanelStyle {
    pub width: Pixels,
    pub max_width: Pixels,
    pub max_height: Pixels,
    pub list_max_height: Pixels,
    pub row_height: Rems,
    pub footer_height: Rems,
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
    pub height: Rems,
    pub padding_x: Rems,
    pub radius: Pixels,
    pub border_width: Pixels,
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
        row_height: rems(3.375),
        footer_height: rems(2.75),
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
    let row = yttt_row_style(YtttRowKind::Palette, state, enabled, theme);
    let tone = if !enabled {
        PaletteRowTone::Disabled
    } else {
        match state {
            SelectableState::Active => PaletteRowTone::Active,
            SelectableState::Inactive => PaletteRowTone::Inactive,
        }
    };

    PaletteRowStyle {
        tone,
        height: row.height,
        padding_x: row.padding_x,
        radius: row.radius,
        border_width: row.border_width,
        background: row.background,
        hover_background: row.hover_background,
        border: row.border,
        title: row.title,
        subtitle: row.subtitle,
        status: row.status,
    }
}

pub fn palette_footer_actions(ui_text: &UiText) -> Vec<PaletteFooterAction> {
    vec![
        PaletteFooterAction {
            label: ui_text.get(UiTextKey::PaletteRun),
            key: "enter",
        },
        PaletteFooterAction {
            label: ui_text.get(UiTextKey::PaletteClose),
            key: "esc",
        },
    ]
}

pub fn palette_input_placeholder(kind: PaletteKind, ui_text: &UiText) -> &'static str {
    match kind {
        PaletteKind::Command => ui_text.get(UiTextKey::PalettePlaceholderCommand),
        PaletteKind::NewTabCommand => ui_text.get(UiTextKey::PalettePlaceholderNewTabCommand),
        PaletteKind::Project => ui_text.get(UiTextKey::PalettePlaceholderProject),
        PaletteKind::OpenedProject => ui_text.get(UiTextKey::PalettePlaceholderOpenedProject),
        PaletteKind::RecentProject => ui_text.get(UiTextKey::PalettePlaceholderRecentProject),
        PaletteKind::Tab => ui_text.get(UiTextKey::PalettePlaceholderTab),
        PaletteKind::Pane => ui_text.get(UiTextKey::PalettePlaceholderPane),
        PaletteKind::GitBranch => ui_text.get(UiTextKey::PalettePlaceholderGitBranch),
    }
}
