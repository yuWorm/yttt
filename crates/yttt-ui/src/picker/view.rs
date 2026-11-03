use std::ops::Range;

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, Entity, HighlightStyle, InteractiveElement as _,
    IntoElement, MouseDownEvent, Pixels, ScrollHandle, SharedString, Stateful,
    StatefulInteractiveElement as _, StyledText, Window, div, point, prelude::*, px, relative,
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

#[allow(clippy::too_many_arguments)]
pub fn picker_overlay<H, F>(
    rows: Vec<PickerOverlayRow>,
    text: PickerText,
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
    yttt_panel_overlay(
        picker_panel(
            rows,
            text,
            query_input,
            scroll_handle,
            window,
            theme,
            ui_style,
            on_confirm_item,
        )
        .on_mouse_down_out(on_dismiss),
        YtttPanelKind::Palette,
        YtttOverlayPlacement::Top,
        theme,
        ui_style,
    )
}

/// The picker surface without its modal layer.
#[allow(clippy::too_many_arguments)]
pub fn picker_panel<H, F>(
    rows: Vec<PickerOverlayRow>,
    text: PickerText,
    query_input: &Entity<InputState>,
    scroll_handle: &ScrollHandle,
    window: &Window,
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
            window,
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
                                window,
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
            .child(picker_footer(footer, theme, ui_style))
            .on_mouse_down_out(on_dismiss),
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

#[allow(clippy::too_many_arguments)]
fn picker_items<H, F>(
    rows: Vec<PickerOverlayRow>,
    no_results: SharedString,
    scroll_handle: &ScrollHandle,
    window: &Window,
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
    let anchor = picker_scroll_anchor_index(selected_index);
    let list = div()
        .id("palette-list")
        .debug_selector(|| "palette-list".to_string())
        .flex()
        .flex_col()
        .px(ui_style.palette.list_padding_x)
        .py(ui_style.palette.list_padding_y)
        .max_h(panel_style.body_max_height)
        .overflow_y_scroll()
        .track_scroll(scroll_handle);

    let kinds = rows
        .iter()
        .map(|row| row_kind(row.show_subtitle, row.item.subtitle.as_deref()))
        .collect::<Vec<_>>();
    let uniform = kinds.windows(2).all(|pair| pair[0] == pair[1]);
    if rows.len() < VIRTUAL_ROW_THRESHOLD || !uniform {
        if let Some(index) = anchor {
            scroll_handle.scroll_to_top_of_item(index);
        }
        return rows
            .into_iter()
            .enumerate()
            .fold(list.gap(ui_style.palette.list_gap), |list, (index, row)| {
                list.child(picker_item(
                    row,
                    index,
                    theme,
                    ui_style,
                    on_confirm_item(index),
                ))
            })
            .into_any_element();
    }

    // Long lists render only the rows in view, padded to their full height.
    let rem_size = window.rem_size();
    let row_height = yttt_row_style(kinds[0], SelectableState::Inactive, true, theme, ui_style)
        .height
        .to_pixels(rem_size);
    let gap = ui_style.palette.list_gap.to_pixels(rem_size);
    let padding_top = ui_style.palette.list_padding_y.to_pixels(rem_size);
    let pitch = row_height + gap;
    if let Some(index) = anchor {
        scroll_handle.set_offset(point(px(0.), -(padding_top + pitch * index as f32)));
    }
    let scrolled = (-scroll_handle.offset().y - padding_top).max(px(0.));
    let count = rows.len();
    let range = visible_row_range(count, scrolled, pitch, panel_style.body_max_height);
    let after = count - range.end;
    let mut list = list.child(div().flex_none().h(pitch * range.start as f32));
    for (index, row) in rows
        .into_iter()
        .enumerate()
        .skip(range.start)
        .take(range.len())
    {
        list = list.child(
            picker_item(row, index, theme, ui_style, on_confirm_item(index))
                .flex_none()
                .when(index + 1 < count, |row| row.mb(gap)),
        );
    }
    list.when(after > 0, |list| {
        list.child(div().flex_none().h(pitch * after as f32 - gap))
    })
    .into_any_element()
}

/// Lists shorter than this render every row.
const VIRTUAL_ROW_THRESHOLD: usize = 64;
/// Rows rendered past each edge of the viewport.
const VIRTUAL_OVERSCAN: usize = 8;

/// The rows a list of `count` rows spaced `pitch` apart shows after scrolling
/// `scrolled` into a `viewport`, plus [`VIRTUAL_OVERSCAN`] on each side.
pub fn visible_row_range(
    count: usize,
    scrolled: Pixels,
    pitch: Pixels,
    viewport: Pixels,
) -> Range<usize> {
    if count == 0 || pitch <= px(0.) {
        return 0..count;
    }
    let first = (scrolled / pitch).floor().max(0.) as usize;
    let visible = (viewport / pitch).ceil() as usize + 1;
    let start = first.saturating_sub(VIRTUAL_OVERSCAN).min(count);
    let end = (first + visible + VIRTUAL_OVERSCAN).min(count);
    start..end
}

fn row_kind(show_subtitle: bool, subtitle: Option<&str>) -> YtttRowKind {
    if show_subtitle && subtitle.is_some_and(|subtitle| !subtitle.trim().is_empty()) {
        YtttRowKind::Palette
    } else {
        YtttRowKind::PaletteCompact
    }
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
        .debug_selector(move || format!("palette-item-{index}"))
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
    let row_kind = row_kind(show_subtitle, item.subtitle.as_deref());
    let show_subtitle = row_kind == YtttRowKind::Palette;
    let subtitle = item.subtitle.unwrap_or_default();
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

#[cfg(test)]
mod tests {
    use gpui::px;

    use super::{VIRTUAL_OVERSCAN, visible_row_range};

    #[test]
    fn visible_rows_cover_the_viewport_and_overscan() {
        let range = visible_row_range(2000, px(0.), px(32.), px(384.));
        assert_eq!(range, 0..(13 + VIRTUAL_OVERSCAN));

        let range = visible_row_range(2000, px(32. * 1000.), px(32.), px(384.));
        assert_eq!(
            range,
            (1000 - VIRTUAL_OVERSCAN)..(1000 + 13 + VIRTUAL_OVERSCAN)
        );
    }

    #[test]
    fn visible_rows_stay_inside_the_list() {
        assert_eq!(
            visible_row_range(2000, px(32. * 5000.), px(32.), px(384.)),
            2000..2000
        );
        assert_eq!(visible_row_range(10, px(0.), px(32.), px(384.)), 0..10);
        assert_eq!(visible_row_range(0, px(0.), px(32.), px(384.)), 0..0);
    }
}
