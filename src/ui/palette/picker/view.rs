use gpui::{
    AnyElement, App, ClickEvent, Div, Entity, InteractiveElement as _, IntoElement, ScrollHandle,
    StatefulInteractiveElement as _, Window, div, prelude::*, px, relative,
};
use gpui_component::{
    IconName,
    input::{Input, InputState},
};

use crate::ui::{
    components::{SelectableState, workbench_palette_item},
    i18n::{UiText, UiTextKey},
    interaction::overlay::capture_overlay_input,
    palette::picker::PickerItem,
    palette::surface::{
        PaletteFooterAction, palette_footer_actions, palette_panel_style,
        palette_scroll_anchor_index,
    },
    primitives::{
        input::{YtttInputKind, yttt_input_style},
        panel::{YtttPanelKind, yttt_panel_style},
    },
    theme::{UiStyle, WorkbenchTheme},
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
    ui_style: UiStyle,
    on_confirm_item: F,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let panel = yttt_panel_style(YtttPanelKind::Palette, theme, ui_style);

    capture_overlay_input(
        div().absolute().inset_0().child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_start()
                .justify_center()
                .pt(ui_style.spacing.overlay_top)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w(panel.width)
                        .max_w(panel.max_width)
                        .max_h(panel.max_height)
                        .rounded(panel.radius)
                        .border(panel.border_width)
                        .border_color(panel.border)
                        .bg(panel.background)
                        .when(panel.shadow, |this| this.shadow_lg())
                        .text_color(theme.text)
                        .overflow_hidden()
                        .child(picker_header(query_input, theme, ui_style))
                        .child(picker_items(
                            rows,
                            ui_text,
                            scroll_handle,
                            theme,
                            ui_style,
                            on_confirm_item,
                        ))
                        .child(picker_footer(ui_text, theme, ui_style)),
                ),
        ),
    )
}

pub fn picker_overlay_with_preview<H, F>(
    rows: Vec<PickerOverlayRow>,
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
    let panel = yttt_panel_style(YtttPanelKind::Palette, theme, ui_style);

    capture_overlay_input(
        div().absolute().inset_0().child(
            div()
                .absolute()
                .inset_0()
                .flex()
                .items_start()
                .justify_center()
                .pt(ui_style.spacing.overlay_top)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .w(relative(0.88))
                        .max_w(px(1_420.))
                        .h(relative(0.78))
                        .max_h(panel.max_height)
                        .rounded(panel.radius)
                        .border(panel.border_width)
                        .border_color(panel.border)
                        .bg(panel.background)
                        .when(panel.shadow, |this| this.shadow_lg())
                        .text_color(theme.text)
                        .overflow_hidden()
                        .child(picker_header(query_input, theme, ui_style))
                        .child(
                            div()
                                .flex()
                                .flex_1()
                                .min_h_0()
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .w(relative(0.42))
                                        .min_w(px(340.))
                                        .min_h_0()
                                        .overflow_hidden()
                                        .child(picker_items(
                                            rows,
                                            ui_text,
                                            scroll_handle,
                                            theme,
                                            ui_style,
                                            on_confirm_item,
                                        )),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .flex_col()
                                        .flex_1()
                                        .min_w_0()
                                        .min_h_0()
                                        .border_l(ui_style.border.hairline)
                                        .border_color(theme.border)
                                        .overflow_hidden()
                                        .child(preview),
                                ),
                        )
                        .child(picker_footer(ui_text, theme, ui_style)),
                ),
        ),
    )
}

fn picker_header(
    query_input: &Entity<InputState>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let input_style = yttt_input_style(YtttInputKind::Palette, theme, ui_style);
    div()
        .flex()
        .items_center()
        .border_b(ui_style.border.hairline)
        .border_color(theme.border)
        .px(ui_style.spacing.lg)
        .py(ui_style.spacing.md)
        .child(
            Input::new(query_input)
                .prefix(IconName::Search)
                .cleanable(true)
                .appearance(true)
                .h(input_style.height)
                .rounded(input_style.radius)
                .border_color(input_style.border)
                .bg(input_style.background)
                .text_color(input_style.text),
        )
}

fn picker_items<H, F>(
    rows: Vec<PickerOverlayRow>,
    ui_text: &UiText,
    scroll_handle: &ScrollHandle,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    mut on_confirm_item: F,
) -> AnyElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let panel_style = palette_panel_style(ui_style);

    if rows.is_empty() {
        return div()
            .id("palette-empty")
            .min_h(panel_style.row_height)
            .p(ui_style.spacing.xl)
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
                .debug_selector(|| "palette-list".to_string())
                .flex()
                .flex_col()
                .gap(ui_style.spacing.xs)
                .p(ui_style.spacing.md)
                .max_h(panel_style.list_max_height)
                .overflow_y_scroll()
                .track_scroll(scroll_handle),
            |list, (index, row)| {
                list.child(picker_item(
                    row,
                    index,
                    theme,
                    ui_style,
                    on_confirm_item(index),
                ))
            },
        )
        .into_any_element()
}

fn picker_item<H>(
    row: PickerOverlayRow,
    index: usize,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
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
        row.item.keybinding,
        row.state,
        row.item.enabled,
        theme,
        ui_style,
        on_click,
    )
}

fn picker_footer(ui_text: &UiText, theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    let style = palette_panel_style(ui_style);

    div()
        .flex()
        .items_center()
        .justify_end()
        .gap(ui_style.spacing.xl)
        .h(style.footer_height)
        .border_t(style.border_width)
        .border_color(theme.border)
        .px(ui_style.spacing.lg)
        .text_xs()
        .text_color(theme.text_muted)
        .children(
            palette_footer_actions(ui_text)
                .into_iter()
                .map(|action| picker_footer_action(action, theme, ui_style)),
        )
}

fn picker_footer_action(
    action: PaletteFooterAction,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(ui_style.spacing.md)
        .child(div().child(action.label))
        .child(
            div()
                .rounded(ui_style.radius.compact)
                .border(ui_style.border.hairline)
                .border_color(theme.border)
                .px(ui_style.spacing.xs)
                .text_color(theme.text_subtle)
                .child(action.key),
        )
}
