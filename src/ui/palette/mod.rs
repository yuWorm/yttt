pub mod picker;
pub mod surface;

use gpui::{App, ClickEvent, Entity, IntoElement, ScrollHandle, Window};
use gpui_component::input::InputState;

use crate::palette::{ActivePalette, PaletteItem};
use crate::ui::components::SelectableState;
use crate::ui::i18n::{UiText, UiTextKey};
use crate::ui::palette::picker::{
    PalettePickerDelegate, PickerDelegate, PickerItem, PickerOverlayRow, PickerState,
    picker_overlay,
};
use crate::ui::theme::{UiStyle, WorkbenchTheme};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub status: Option<String>,
    pub keybinding: Option<String>,
    pub state: SelectableState,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

pub fn visible_palette_rows(
    active_palette: &ActivePalette,
    items: &[PaletteItem],
) -> Vec<PaletteRow> {
    let picker_items = items
        .iter()
        .map(PickerItem::from_palette_item)
        .collect::<Vec<_>>();
    let picker_state = PickerState {
        query: active_palette.query.clone(),
        selected_index: active_palette.selected_index,
    };
    let selected_index = picker_state.clamped_selected_index(&picker_items);

    picker_state
        .filtered_items(&picker_items)
        .into_iter()
        .enumerate()
        .map(|(index, item)| PaletteRow {
            id: item.id.clone(),
            title: item.title.clone(),
            subtitle: item.subtitle.clone(),
            status: item.status.clone(),
            keybinding: item.keybinding.clone(),
            state: if Some(index) == selected_index {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            },
            enabled: item.enabled,
            disabled_reason: item.disabled_reason.clone(),
        })
        .collect()
}

pub fn palette_overlay<H, F>(
    active_palette: &ActivePalette,
    items: &[PaletteItem],
    ui_text: &UiText,
    query_input: &Entity<InputState>,
    scroll_handle: &ScrollHandle,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_confirm_item: F,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let delegate = PalettePickerDelegate::new(active_palette.kind, items.to_vec());
    let picker_state = PickerState {
        query: active_palette.query.clone(),
        selected_index: active_palette.selected_index,
    };
    let selected_index = picker_state.clamped_selected_index(delegate.items());
    let rows = picker_state
        .filtered_items(delegate.items())
        .into_iter()
        .enumerate()
        .map(|(index, item)| PickerOverlayRow {
            item: item.clone(),
            state: if Some(index) == selected_index {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            },
        })
        .collect::<Vec<_>>();

    picker_overlay(
        rows,
        ui_text,
        query_input,
        scroll_handle,
        theme,
        ui_style,
        on_confirm_item,
    )
}

pub fn palette_empty_label(ui_text: &UiText) -> &'static str {
    ui_text.get(UiTextKey::NoResults)
}
