use gpui::{
    AnyElement, App, ClickEvent, Div, Entity, InteractiveElement as _, IntoElement, ScrollHandle,
    StatefulInteractiveElement as _, Window, div, prelude::*, px, relative,
};
use gpui_component::{IconName, input::InputState};

use crate::palette::PaletteKind;

use crate::ui::{
    components::{SelectableState, workbench_palette_item},
    i18n::{UiText, UiTextKey},
    palette::picker::PickerItem,
    palette::surface::{PaletteFooterAction, palette_footer_actions, palette_scroll_anchor_index},
    primitives::{
        input::{YtttInputKind, yttt_input},
        panel::{
            YtttOverlayPlacement, YtttPanelKind, yttt_panel, yttt_panel_overlay, yttt_panel_style,
        },
        row::{YtttRowKind, yttt_row_style},
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
    kind: PaletteKind,
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
    yttt_panel_overlay(
        yttt_panel(YtttPanelKind::Palette, theme, ui_style)
            .p_0()
            .overflow_hidden()
            .child(picker_header(query_input, theme, ui_style))
            .child(picker_items(
                rows,
                kind,
                ui_text,
                scroll_handle,
                theme,
                ui_style,
                on_confirm_item,
            ))
            .child(picker_footer(ui_text, theme, ui_style)),
        YtttPanelKind::Palette,
        YtttOverlayPlacement::Top,
        theme,
        ui_style,
    )
}

pub fn picker_overlay_with_preview<H, F>(
    rows: Vec<PickerOverlayRow>,
    kind: PaletteKind,
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
                                kind,
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
        YtttPanelKind::Palette,
        YtttOverlayPlacement::Top,
        theme,
        ui_style,
    )
}

fn picker_header(
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
    kind: PaletteKind,
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
                    kind,
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
    kind: PaletteKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_click: H,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let status = if !row.item.enabled {
        row.item.disabled_reason.clone().unwrap_or_default()
    } else if matches!(kind, PaletteKind::Project | PaletteKind::RecentProject) {
        String::new()
    } else {
        row.item.status.clone().unwrap_or_default()
    };
    let subtitle = row.item.subtitle.unwrap_or_default();
    let show_subtitle = !subtitle.trim().is_empty()
        && (kind != PaletteKind::Command || ui_style.palette.show_command_subtitles);
    let row_kind = if show_subtitle {
        YtttRowKind::Palette
    } else {
        YtttRowKind::PaletteCompact
    };
    let leading_icon = picker_item_icon(kind, &row.item.id);

    workbench_palette_item(
        ("palette-item", index),
        row.item.title,
        subtitle,
        status,
        row.item.keybinding,
        leading_icon,
        row_kind,
        row.state,
        row.item.enabled,
        theme,
        ui_style,
        on_click,
    )
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
    }
}

fn picker_footer(ui_text: &UiText, theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    div()
        .flex()
        .items_center()
        .justify_end()
        .gap(ui_style.spacing.xl)
        .h(ui_style.controls.palette_footer_height)
        .border_t(ui_style.border.hairline)
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
        .gap(ui_style.palette.footer_action_gap)
        .child(div().text_color(theme.text).child(action.label))
        .child(
            div()
                .text_color(theme.text_subtle)
                .when(ui_style.palette.bordered_shortcuts, |this| {
                    this.rounded(ui_style.radius.compact)
                        .border(ui_style.border.hairline)
                        .border_color(theme.border)
                        .bg(theme.surface_elevated)
                        .px(ui_style.spacing.xs)
                })
                .child(action.key),
        )
}
