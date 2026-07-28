use crate::{
    palette::PaletteKind,
    ui::i18n::{UiText, UiTextKey},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PaletteFooterAction {
    pub label: &'static str,
    pub key: &'static str,
}

pub fn palette_scroll_anchor_index(selected_index: usize) -> Option<usize> {
    (selected_index > 0).then(|| selected_index.saturating_sub(4))
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
        PaletteKind::File => ui_text.get(UiTextKey::PalettePlaceholderFile),
        PaletteKind::Project => ui_text.get(UiTextKey::PalettePlaceholderProject),
        PaletteKind::OpenedProject => ui_text.get(UiTextKey::PalettePlaceholderOpenedProject),
        PaletteKind::RecentProject => ui_text.get(UiTextKey::PalettePlaceholderRecentProject),
        PaletteKind::Tab => ui_text.get(UiTextKey::PalettePlaceholderTab),
        PaletteKind::Pane => ui_text.get(UiTextKey::PalettePlaceholderPane),
        PaletteKind::GitBranch => ui_text.get(UiTextKey::PalettePlaceholderGitBranch),
    }
}
