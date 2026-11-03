pub mod picker;
pub mod surface;

use gpui::{
    AnyElement, App, ClickEvent, Entity, IntoElement, MouseDownEvent, ScrollHandle, Window,
};
use gpui_component::input::InputState;
use yttt_ui::{
    IconName,
    picker::{PickerFooterAction, PickerText, query_match},
};

use crate::palette::{ActivePalette, PaletteItem, PaletteKind};
use crate::ui::components::SelectableState;
use crate::ui::components::workbench_keybinding_badge;
use crate::ui::i18n::{UiText, UiTextKey};
use crate::ui::palette::picker::{
    PalettePickerDelegate, PickerDelegate, PickerItem, PickerOverlayRow, PickerState,
    picker_overlay, picker_overlay_with_preview,
};
use crate::ui::palette::surface::palette_footer_actions;
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
    let picker_items = items.iter().map(PickerItem::from).collect::<Vec<_>>();
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
    window: &Window,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_confirm_item: F,
    on_dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
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
        .map(|(index, item)| {
            let state = if Some(index) == selected_index {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            };
            palette_row(active_palette, item, state, theme, ui_style)
        })
        .collect::<Vec<_>>();

    picker_overlay(
        rows,
        palette_text(ui_text, theme, ui_style),
        query_input,
        scroll_handle,
        window,
        theme,
        ui_style,
        on_confirm_item,
        on_dismiss,
    )
}

pub fn file_finder_palette_overlay<H, F>(
    active_palette: &ActivePalette,
    items: &[PaletteItem],
    ui_text: &UiText,
    query_input: &Entity<InputState>,
    scroll_handle: &ScrollHandle,
    window: &Window,
    preview: AnyElement,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_confirm_item: F,
    on_dismiss: impl Fn(&MouseDownEvent, &mut Window, &mut App) + 'static,
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
        .map(|(index, item)| {
            let state = if Some(index) == selected_index {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            };
            palette_row(active_palette, item, state, theme, ui_style)
        })
        .collect();

    picker_overlay_with_preview(
        rows,
        palette_text(ui_text, theme, ui_style),
        query_input,
        scroll_handle,
        window,
        preview,
        theme,
        ui_style,
        on_confirm_item,
        on_dismiss,
    )
}

fn palette_row(
    active_palette: &ActivePalette,
    item: &PickerItem,
    state: SelectableState,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> PickerOverlayRow {
    let kind = active_palette.kind;
    let status = if !item.enabled {
        item.disabled_reason.clone().unwrap_or_default()
    } else if matches!(kind, PaletteKind::Project | PaletteKind::RecentProject) {
        String::new()
    } else {
        item.status.clone().unwrap_or_default()
    };
    PickerOverlayRow {
        state,
        status,
        show_subtitle: kind != PaletteKind::Command || ui_style.palette.show_command_subtitles,
        icon: picker_item_icon(kind, &item.id),
        keybinding: item
            .keybinding
            .clone()
            .filter(|keybinding| !keybinding.trim().is_empty())
            .map(|keybinding| workbench_keybinding_badge(keybinding, theme, ui_style)),
        highlight: query_match(&item.title, &active_palette.query),
        item: item.clone(),
    }
}

fn picker_item_icon(kind: PaletteKind, item_id: &str) -> Option<IconName> {
    match kind {
        PaletteKind::Command => None,
        PaletteKind::NewTabCommand | PaletteKind::Pane => Some(IconName::SquareTerminal),
        PaletteKind::File => Some(IconName::File),
        PaletteKind::Project | PaletteKind::OpenedProject | PaletteKind::RecentProject => {
            Some(IconName::FolderClosed)
        }
        PaletteKind::Tab if item_id.starts_with("file:") => Some(IconName::File),
        PaletteKind::Tab => Some(IconName::SquareTerminal),
        PaletteKind::GitBranch => Some(IconName::Network),
        PaletteKind::WorkspaceHistory => Some(IconName::FolderClosed),
    }
}

fn palette_text(ui_text: &UiText, theme: WorkbenchTheme, ui_style: UiStyle) -> PickerText {
    PickerText {
        no_results: palette_empty_label(ui_text).into(),
        footer: palette_footer_actions(ui_text)
            .into_iter()
            .map(|action| PickerFooterAction {
                label: action.label.into(),
                key: workbench_keybinding_badge(action.key, theme, ui_style),
            })
            .collect(),
    }
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
