use std::ops::Range;

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Entity, HighlightStyle, InteractiveElement as _,
    IntoElement, ScrollHandle, SharedString, Stateful, StatefulInteractiveElement as _, StyledText,
    Window, div, prelude::*, px, relative,
};
use gpui_component::input::InputState;

use super::PickerItem;
use crate::{
    Icon, IconName, SelectableState,
    primitives::{
        input::{YtttInputKind, yttt_input},
        panel::{
            YtttOverlayPlacement, YtttPanelKind, yttt_panel, yttt_panel_overlay, yttt_panel_style,
        },
        row::{YtttRowKind, yttt_row, yttt_row_style},
    },
    style::UiStyle,
    theme::WorkbenchTheme,
};

/// One row, with its presentation already decided by the caller.
pub struct PickerOverlayRow {
    pub item: PickerItem,
    pub state: SelectableState,
    pub status: String,
    pub show_subtitle: bool,
    pub icon: Option<IconName>,
    pub keybinding: Option<AnyElement>,
    /// Byte range of the title drawn in the accent color, as Zed marks matches.
    pub highlight: Option<Range<usize>>,
}

pub struct PickerFooterAction {
    pub label: SharedString,
    pub key: AnyElement,
}

/// Localized text a picker shows around its rows.
pub struct PickerText {
    pub no_results: SharedString,
    pub footer: Vec<PickerFooterAction>,
}

/// Keeps the selection a few rows below the top of the list.
pub fn picker_scroll_anchor_index(selected_index: usize) -> Option<usize> {
    (selected_index > 0).then(|| selected_index.saturating_sub(4))
}

pub fn picker_overlay<H, F>(
    rows: Vec<PickerOverlayRow>,
    text: PickerText,
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
    yttt_panel_overlay(
        picker_panel(
            rows,
            text,
            query_input,
            scroll_handle,
            theme,
            ui_style,
            on_confirm_item,
        ),
        YtttPanelKind::Palette,
        YtttOverlayPlacement::Top,
        theme,
        ui_style,
    )
}

/// The picker surface without its modal layer.
pub fn picker_panel<H, F>(
    rows: Vec<PickerOverlayRow>,
    text: PickerText,
    query_input: &Entity<InputState>,
    scroll_handle: &ScrollHandle,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_confirm_item: F,
) -> Div
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let PickerText { no_results, footer } = text;
    yttt_panel(YtttPanelKind::Palette, theme, ui_style)
        .p_0()
        .overflow_hidden()
        .child(picker_header(query_input, theme, ui_style))
        .child(picker_items(
            rows,
            no_results,
            scroll_handle,
            theme,
            ui_style,
            on_confirm_item,
        ))
        .child(picker_footer(footer, theme, ui_style))
}

#[allow(clippy::too_many_arguments)]
pub fn picker_overlay_with_preview<H, F>(
    rows: Vec<PickerOverlayRow>,
    text: PickerText,
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
    let PickerText { no_results, footer } = text;
    yttt_panel_overlay(
        yttt_panel(YtttPanelKind::Palette, theme, ui_style)
            .w(relative(0.88))
            .max_w(px(1_420.))
            .h(relative(0.78))
            .p_0()
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
                                no_results,
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
            .child(picker_footer(footer, theme, ui_style)),
        YtttPanelKind::Palette,
        YtttOverlayPlacement::Top,
        theme,
        ui_style,
    )
}

pub fn picker_header(
    query_input: &Entity<InputState>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let mut input =
        yttt_input(query_input, YtttInputKind::Palette, theme, ui_style).cleanable(true);
    if ui_style.palette.show_search_icon {
        input = input.prefix(IconName::Search);
    }

    div()
        .flex()
        .items_center()
        .border_b(ui_style.border.hairline)
        .border_color(theme.border)
        .px(ui_style.palette.header_padding_x)
        .py(ui_style.palette.header_padding_y)
        .child(input)
}

fn picker_items<H, F>(
    rows: Vec<PickerOverlayRow>,
    no_results: SharedString,
    scroll_handle: &ScrollHandle,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    mut on_confirm_item: F,
) -> AnyElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(usize) -> H,
{
    let panel_style = yttt_panel_style(YtttPanelKind::Palette, theme, ui_style);
    let row_style = yttt_row_style(
        YtttRowKind::PaletteCompact,
        SelectableState::Inactive,
        true,
        theme,
        ui_style,
    );

    if rows.is_empty() {
        return div()
            .id("palette-empty")
            .min_h(row_style.height)
            .p(ui_style.spacing.xl)
            .text_sm()
            .text_color(theme.text_subtle)
            .child(no_results)
            .into_any_element();
    }

    let selected_index = rows
        .iter()
        .position(|row| row.state == SelectableState::Active)
        .unwrap_or(0);
    if let Some(index) = picker_scroll_anchor_index(selected_index) {
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
                .gap(ui_style.palette.list_gap)
                .px(ui_style.palette.list_padding_x)
                .py(ui_style.palette.list_padding_y)
                .max_h(panel_style.body_max_height)
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
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    picker_row(("palette-item", index), row, theme, ui_style, on_click)
}

/// A picker row for lists that manage their own layout.
pub fn picker_row<H>(
    id: impl Into<ElementId>,
    row: PickerOverlayRow,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let PickerOverlayRow {
        item,
        state,
        status,
        show_subtitle,
        icon,
        keybinding,
        highlight,
    } = row;
    let subtitle = item.subtitle.unwrap_or_default();
    let show_subtitle = show_subtitle && !subtitle.trim().is_empty();
    let row_kind = if show_subtitle {
        YtttRowKind::Palette
    } else {
        YtttRowKind::PaletteCompact
    };
    let style = yttt_row_style(row_kind, state, item.enabled, theme, ui_style);

    yttt_row(row_kind, state, item.enabled, theme, ui_style)
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap(ui_style.palette.item_content_gap)
        .on_click(on_click)
        .child(
            div()
                .flex()
                .flex_1()
                .min_w_0()
                .items_center()
                .gap(ui_style.palette.item_content_gap)
                .children(icon.map(|icon| {
                    div()
                        .flex()
                        .flex_none()
                        .items_center()
                        .justify_center()
                        .w(ui_style.palette.icon_column_width)
                        .child(
                            Icon::new(icon)
                                .size(ui_style.palette.icon_size)
                                .text_color(style.subtitle),
                        )
                }))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .min_w_0()
                        .gap(ui_style.spacing.xs)
                        .child(
                            div()
                                .text_sm()
                                .text_color(style.title)
                                .truncate()
                                .child(highlighted_title(item.title, highlight, theme)),
                        )
                        .children(show_subtitle.then(|| {
                            div()
                                .text_xs()
                                .text_color(style.subtitle)
                                .truncate()
                                .child(subtitle)
                        })),
                ),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_end()
                .gap(ui_style.spacing.md)
                .when(!status.is_empty(), |this| {
                    this.child(
                        div()
                            .text_xs()
                            .text_color(style.status)
                            .truncate()
                            .child(status),
                    )
                })
                .children(keybinding),
        )
}

fn highlighted_title(
    title: String,
    highlight: Option<Range<usize>>,
    theme: WorkbenchTheme,
) -> StyledText {
    let highlight = highlight.filter(|range| {
        range.end <= title.len()
            && title.is_char_boundary(range.start)
            && title.is_char_boundary(range.end)
    });
    let text = StyledText::new(title);
    match highlight {
        Some(range) => text.with_highlights([(
            range,
            HighlightStyle {
                color: Some(theme.accent.into()),
                ..HighlightStyle::default()
            },
        )]),
        None => text,
    }
}

fn picker_footer(
    actions: Vec<PickerFooterAction>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    div()
        .flex()
        .items_center()
        .justify_end()
        .gap(ui_style.spacing.xl)
        .h(ui_style.controls.palette_footer_height)
        .border_t(ui_style.border.hairline)
        .border_color(theme.border)
        .px(ui_style.spacing.lg)
        .text_sm()
        .text_color(theme.text_muted)
        .children(actions.into_iter().map(|action| {
            div()
                .flex()
                .items_center()
                .gap(ui_style.palette.footer_action_gap)
                .child(div().text_color(theme.text).child(action.label))
                .child(action.key)
        }))
}
