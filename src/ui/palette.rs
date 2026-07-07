use gpui::{Div, IntoElement, div, prelude::*, px, rgb, rgba};

use crate::palette::{ActivePalette, PaletteItem, PaletteKind};

pub fn palette_overlay(active_palette: &ActivePalette, items: &[PaletteItem]) -> impl IntoElement {
    let filtered_items = active_palette.filtered_items(items);

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
                .child(palette_header(active_palette))
                .child(palette_items(filtered_items, active_palette.selected_index)),
        )
}

fn palette_header(active_palette: &ActivePalette) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_1()
        .border_b_1()
        .border_color(rgb(0x2a2a2a))
        .p_3()
        .child(div().text_sm().text_color(rgb(0xd4d4d4)).child(palette_title(
            active_palette.kind,
        )))
        .child(
            div()
                .text_sm()
                .text_color(rgb(0x737373))
                .child(if active_palette.query.is_empty() {
                    "Type to filter".to_string()
                } else {
                    active_palette.query.clone()
                }),
        )
}

fn palette_items(items: Vec<&PaletteItem>, selected_index: usize) -> Div {
    if items.is_empty() {
        return div()
            .p_4()
            .text_sm()
            .text_color(rgb(0x737373))
            .child("No results");
    }

    items
        .into_iter()
        .enumerate()
        .fold(div().flex().flex_col().p_2(), |list, (index, item)| {
            list.child(palette_item(item, index == selected_index))
        })
}

fn palette_item(item: &PaletteItem, selected: bool) -> Div {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .rounded_sm()
        .px_2()
        .py_2()
        .bg(if selected { rgb(0x263238) } else { rgb(0x151515) })
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .overflow_hidden()
                .child(div().text_sm().truncate().child(item.title.clone()))
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x8a8a8a))
                        .truncate()
                        .child(item.subtitle.clone().unwrap_or_default()),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(rgb(0xa3a3a3))
                .child(item.status.clone().unwrap_or_default()),
        )
}

fn palette_title(kind: PaletteKind) -> &'static str {
    match kind {
        PaletteKind::Command => "Command Palette",
        PaletteKind::Project => "Project Palette",
        PaletteKind::Tab => "Tab Palette",
        PaletteKind::Pane => "Pane Palette",
    }
}
