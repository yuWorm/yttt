use gpui::{
    AnyElement, App, ClickEvent, Div, Entity, InteractiveElement as _, IntoElement, ScrollHandle,
    StatefulInteractiveElement as _, Window, div, prelude::*,
};
use gpui_component::{
    IconName,
    input::{Input, InputState},
};

use crate::ui::{
    components::{SelectableState, workbench_palette_item},
    i18n::{UiText, UiTextKey},
    overlay::capture_overlay_input,
    palette_surface::{
        PaletteFooterAction, palette_footer_actions, palette_panel_style,
        palette_scroll_anchor_index,
    },
    picker::PickerItem,
    primitives::panel::{YtttPanelKind, yttt_panel_style},
    theme::WorkbenchTheme,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PickerOverlayRow {
    pub item: PickerItem,
    pub state: SelectableState,
}

pub fn picker_overlay<H, F>(
    rows: Vec<PickerOverlayRow>,
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
    let panel = yttt_panel_style(YtttPanelKind::Palette, theme);

    capture_overlay_input(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_start()
            .justify_center()
            .pt_16()
            .bg(panel.overlay)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(panel.width)
                    .max_w(panel.max_width)
                    .max_h(panel.max_height)
                    .rounded(panel.radius)
                    .border_1()
                    .border_color(panel.border)
                    .bg(panel.background)
                    .text_color(theme.text)
                    .overflow_hidden()
                    .child(picker_header(query_input, theme))
                    .child(picker_items(
                        rows,
                        ui_text,
                        scroll_handle,
                        theme,
                        on_confirm_item,
                    ))
                    .child(picker_footer(theme)),
            ),
    )
}

fn picker_header(query_input: &Entity<InputState>, theme: WorkbenchTheme) -> Div {
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

fn picker_items<H, F>(
    rows: Vec<PickerOverlayRow>,
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
            .child(ui_text.get(UiTextKey::NoResults))
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
            |list, (index, row)| list.child(picker_item(row, index, theme, on_confirm_item(index))),
        )
        .into_any_element()
}

fn picker_item<H>(
    row: PickerOverlayRow,
    index: usize,
    theme: WorkbenchTheme,
    on_click: H,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let status = if row.item.enabled {
        row.item.status.clone().unwrap_or_default()
    } else {
        row.item.disabled_reason.clone().unwrap_or_default()
    };
    workbench_palette_item(
        ("palette-item", index),
        row.item.title,
        row.item.subtitle.unwrap_or_default(),
        status,
        row.state,
        row.item.enabled,
        theme,
        on_click,
    )
}

fn picker_footer(theme: WorkbenchTheme) -> Div {
    let style = palette_panel_style();

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
        .text_color(theme.text_muted)
        .children(
            palette_footer_actions()
                .into_iter()
                .map(|action| picker_footer_action(action, theme)),
        )
}

fn picker_footer_action(action: PaletteFooterAction, theme: WorkbenchTheme) -> Div {
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
