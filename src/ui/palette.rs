use gpui::{
    AnyElement, App, ClickEvent, Div, Entity, InteractiveElement as _, IntoElement, ScrollHandle,
    StatefulInteractiveElement as _, Window, div, prelude::*, rgba,
};
use gpui_component::{
    IconName,
    input::{Input, InputState},
};

use crate::palette::{ActivePalette, PaletteItem};
use crate::ui::components::SelectableState;
use crate::ui::i18n::{UiText, UiTextKey};
use crate::ui::overlay::capture_overlay_input;
use crate::ui::palette_surface::{
    PaletteFooterAction, palette_footer_actions, palette_panel_style, palette_row_style,
    palette_scroll_anchor_index,
};
use crate::ui::theme::WorkbenchTheme;

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
    query_input: &Entity<InputState>,
    scroll_handle: &ScrollHandle,
    theme: WorkbenchTheme,
    on_confirm_item: F,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let rows = visible_palette_rows(active_palette, items);
    let style = palette_panel_style();

    capture_overlay_input(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt_16()
            .bg(rgba(0x00000066))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(style.width)
                    .max_w(style.max_width)
                    .max_h(style.max_height)
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(theme.surface)
                    .text_color(theme.text)
                    .overflow_hidden()
                    .child(palette_header(query_input, theme))
                    .child(palette_items(
                        rows,
                        ui_text,
                        scroll_handle,
                        theme,
                        on_confirm_item,
                    ))
                    .child(palette_footer(theme)),
            ),
    )
}

fn palette_header(query_input: &Entity<InputState>, theme: WorkbenchTheme) -> Div {
    div()
        .flex()
        .items_center()
        .border_b_1()
        .border_color(theme.border)
        .px_3()
        .py_2()
        .child(
            Input::new(query_input)
                .prefix(IconName::Search)
                .cleanable(true)
                .appearance(false),
        )
}

fn palette_items<H, F>(
    rows: Vec<PaletteRow>,
    ui_text: &UiText,
    scroll_handle: &ScrollHandle,
    theme: WorkbenchTheme,
    mut on_confirm_item: F,
) -> AnyElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let panel_style = palette_panel_style();

    if rows.is_empty() {
        return div()
            .id("palette-empty")
            .min_h(panel_style.row_height)
            .p_4()
            .text_sm()
            .text_color(theme.text_subtle)
            .child(palette_empty_label(ui_text))
            .into_any_element();
    }

    let selected_index = rows
        .iter()
        .position(|row| row.state == SelectableState::Active)
        .unwrap_or(0);
    if let Some(index) = palette_scroll_anchor_index(selected_index) {
        scroll_handle.scroll_to_top_of_item(index);
    }

    rows.into_iter()
        .enumerate()
        .fold(
            div()
                .id("palette-list")
                .flex()
                .flex_col()
                .gap_1()
                .p_2()
                .max_h(panel_style.list_max_height)
                .overflow_y_scroll()
                .track_scroll(scroll_handle),
            |list, (index, row)| {
                list.child(palette_item(row, index, theme, on_confirm_item(index)))
            },
        )
        .into_any_element()
}

fn palette_item<H>(
    row: PaletteRow,
    index: usize,
    theme: WorkbenchTheme,
    on_click: H,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let status = if row.enabled {
        row.status.clone().unwrap_or_default()
    } else {
        row.disabled_reason.clone().unwrap_or_default()
    };
    let style = palette_row_style(row.state, row.enabled, theme);
    let panel_style = palette_panel_style();

    div()
        .id(("palette-item", index))
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .h(panel_style.row_height)
        .rounded_sm()
        .border_1()
        .border_color(style.border)
        .bg(style.background)
        .px_3()
        .hover(move |this| this.bg(style.hover_background))
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
                        .text_color(style.title)
                        .truncate()
                        .child(row.title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(style.subtitle)
                        .truncate()
                        .child(row.subtitle.unwrap_or_default()),
                ),
        )
        .child(
            div()
                .flex_none()
                .text_xs()
                .text_color(style.status)
                .child(status),
        )
}

pub fn palette_empty_label(ui_text: &UiText) -> &'static str {
    ui_text.get(UiTextKey::NoResults)
}

fn palette_footer(theme: WorkbenchTheme) -> Div {
    let style = palette_panel_style();

    palette_footer_actions().into_iter().fold(
        div()
            .flex()
            .items_center()
            .justify_end()
            .gap_4()
            .h(style.footer_height)
            .border_t_1()
            .border_color(theme.border)
            .px_3()
            .text_xs()
            .text_color(theme.text_muted),
        |footer, action| footer.child(palette_footer_action(action, theme)),
    )
}

fn palette_footer_action(action: PaletteFooterAction, theme: WorkbenchTheme) -> Div {
    div()
        .flex()
        .items_center()
        .gap_2()
        .child(div().child(action.label))
        .child(
            div()
                .rounded_sm()
                .border_1()
                .border_color(theme.border)
                .px_1()
                .text_color(theme.text_subtle)
                .child(action.key),
        )
}
