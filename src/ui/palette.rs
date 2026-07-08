use gpui::{App, ClickEvent, Div, IntoElement, Window, div, prelude::*, px, rgb, rgba};
use gpui_component::list::ListItem;

use crate::palette::{ActivePalette, PaletteItem, PaletteKind};
use crate::ui::components::SelectableState;
use crate::ui::i18n::{UiText, UiTextKey};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteRow {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub status: Option<String>,
    pub state: SelectableState,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

pub fn visible_palette_rows(
    active_palette: &ActivePalette,
    items: &[PaletteItem],
) -> Vec<PaletteRow> {
    let filtered_items = active_palette.filtered_items(items);
    let selected_index = active_palette
        .selected_index
        .min(filtered_items.len().saturating_sub(1));

    filtered_items
        .into_iter()
        .enumerate()
        .map(|(index, item)| PaletteRow {
            id: item.id.clone(),
            title: item.title.clone(),
            subtitle: item.subtitle.clone(),
            status: item.status.clone(),
            state: if index == selected_index {
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
    on_confirm_item: F,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let rows = visible_palette_rows(active_palette, items);

    div()
        .absolute()
        .inset_0()
        .flex()
        .items_start()
        .justify_center()
        .pt_16()
        .bg(rgba(0x00000099))
        .child(
            div()
                .flex()
                .flex_col()
                .w(px(560.0))
                .max_w_full()
                .rounded_md()
                .border_1()
                .border_color(rgb(0x2a2a2a))
                .bg(rgb(0x151515))
                .text_color(rgb(0xf5f5f5))
                .child(palette_header(active_palette, ui_text))
                .child(palette_items(rows, ui_text, on_confirm_item)),
        )
}

fn palette_header(active_palette: &ActivePalette, ui_text: &UiText) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .border_b_1()
        .border_color(rgb(0x2a2a2a))
        .p_3()
        .child(
            div()
                .text_sm()
                .text_color(rgb(0xd4d4d4))
                .child(palette_title(active_palette.kind)),
        )
        .child(div().text_sm().text_color(rgb(0x737373)).child(
            if active_palette.query.is_empty() {
                ui_text.get(UiTextKey::TypeToFilter).to_string()
            } else {
                active_palette.query.clone()
            },
        ))
}

fn palette_items<H, F>(rows: Vec<PaletteRow>, ui_text: &UiText, mut on_confirm_item: F) -> Div
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    if rows.is_empty() {
        return div()
            .p_4()
            .text_sm()
            .text_color(rgb(0x737373))
            .child(palette_empty_label(ui_text));
    }

    rows.into_iter().enumerate().fold(
        div().flex().flex_col().gap_1().p_2(),
        |list, (index, row)| list.child(palette_item(row, index, on_confirm_item(index))),
    )
}

fn palette_item<H>(row: PaletteRow, index: usize, on_click: H) -> ListItem
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let status = if row.enabled {
        row.status.clone().unwrap_or_default()
    } else {
        row.disabled_reason.clone().unwrap_or_default()
    };
    let title_color = if row.enabled { 0xf5f5f5 } else { 0x737373 };

    ListItem::new(("palette-item", index))
        .selected(row.state == SelectableState::Active)
        .on_click(on_click)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .overflow_hidden()
                .child(
                    div()
                        .text_sm()
                        .text_color(rgb(title_color))
                        .truncate()
                        .child(row.title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x8a8a8a))
                        .truncate()
                        .child(row.subtitle.unwrap_or_default()),
                ),
        )
        .suffix(move |_, _| {
            div()
                .text_xs()
                .text_color(rgb(0xa3a3a3))
                .child(status.clone())
        })
}

pub fn palette_empty_label(ui_text: &UiText) -> &'static str {
    ui_text.get(UiTextKey::NoResults)
}

fn palette_title(kind: PaletteKind) -> &'static str {
    match kind {
        PaletteKind::Command => "Command Palette",
        PaletteKind::Project => "Project Palette",
        PaletteKind::Tab => "Tab Palette",
        PaletteKind::Pane => "Pane Palette",
    }
}
