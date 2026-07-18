pub mod picker;
pub mod surface;

use gpui::{AnyElement, App, ClickEvent, Entity, IntoElement, ScrollHandle, Window};
use gpui_component::input::InputState;

use crate::palette::{ActivePalette, PaletteItem, PaletteKind};
use crate::ui::components::SelectableState;
use crate::ui::i18n::{UiText, UiTextKey};
use crate::ui::palette::picker::{
    PalettePickerDelegate, PickerDelegate, PickerItem, PickerOverlayRow, PickerState,
    picker_overlay, picker_overlay_with_preview,
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
    let visible_items = visible_picker_items(active_palette, &picker_state, &picker_items);
    let selected_index = (!visible_items.is_empty()).then(|| {
        active_palette
            .selected_index
            .min(visible_items.len().saturating_sub(1))
    });

    visible_items
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
    let visible_items = visible_picker_items(active_palette, &picker_state, delegate.items());
    let selected_index = (!visible_items.is_empty()).then(|| {
        active_palette
            .selected_index
            .min(visible_items.len().saturating_sub(1))
    });
    let rows = visible_items
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

pub fn file_finder_palette_overlay<H, F>(
    active_palette: &ActivePalette,
    items: &[PaletteItem],
    ui_text: &UiText,
    query_input: &Entity<InputState>,
    scroll_handle: &ScrollHandle,
    preview: AnyElement,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_confirm_item: F,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let delegate = PalettePickerDelegate::new(active_palette.kind, items.to_vec());
    let selected_index = (!delegate.items().is_empty()).then(|| {
        active_palette
            .selected_index
            .min(delegate.items().len().saturating_sub(1))
    });
    let rows = delegate
        .items()
        .iter()
        .enumerate()
        .map(|(index, item)| PickerOverlayRow {
            item: item.clone(),
            state: if Some(index) == selected_index {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            },
        })
        .collect();

    picker_overlay_with_preview(
        rows,
        ui_text,
        query_input,
        scroll_handle,
        preview,
        theme,
        ui_style,
        on_confirm_item,
    )
}

fn visible_picker_items<'a>(
    active_palette: &ActivePalette,
    picker_state: &PickerState,
    items: &'a [PickerItem],
) -> Vec<&'a PickerItem> {
    if active_palette.kind == PaletteKind::File {
        items.iter().collect()
    } else {
        picker_state.filtered_items(items)
    }
}

pub fn palette_empty_label(ui_text: &UiText) -> &'static str {
    ui_text.get(UiTextKey::NoResults)
}
