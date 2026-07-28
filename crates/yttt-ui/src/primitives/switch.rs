use gpui::{App, Div, ElementId, Pixels, Rems, Rgba, SharedString, Window, div, prelude::*};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSwitchStyle {
    pub width: Rems,
    pub height: Rems,
    pub track_width: Rems,
    pub track_height: Rems,
    pub track_padding: Rems,
    pub thumb_size: Rems,
    pub control_height: Rems,
    pub outer_border_width: Pixels,
    pub track_border_width: Pixels,
    pub active_background: Rgba,
    pub inactive_background: Rgba,
    pub active_border: Rgba,
    pub inactive_border: Rgba,
    pub active_thumb: Rgba,
    pub inactive_thumb: Rgba,
}

pub fn yttt_switch_style(theme: WorkbenchTheme, ui_style: UiStyle) -> YtttSwitchStyle {
    YtttSwitchStyle {
        width: ui_style.switches.width,
        height: ui_style.switches.height,
        track_width: ui_style.switches.track_width,
        track_height: ui_style.switches.track_height,
        track_padding: ui_style.switches.track_padding,
        thumb_size: ui_style.switches.thumb_size,
        control_height: ui_style.switches.control_height,
        outer_border_width: ui_style.border.emphasized,
        track_border_width: ui_style.border.hairline,
        active_background: theme.accent,
        inactive_background: ui_style.active_background(theme),
        active_border: theme.focus_ring,
        inactive_border: theme.border_strong,
        active_thumb: theme.text,
        inactive_thumb: theme.text_subtle,
    }
}

pub fn yttt_switch<H>(
    id: impl Into<ElementId>,
    checked: bool,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_change: H,
) -> Div
where
    H: Fn(&bool, &mut Window, &mut App) + 'static,
{
    let style = yttt_switch_style(theme, ui_style);
    let next_checked = !checked;
    let track_background = if checked {
        style.active_background
    } else {
        style.inactive_background
    };
    let border = if checked {
        style.active_border
    } else {
        style.inactive_border
    };
    let thumb = if checked {
        style.active_thumb
    } else {
        style.inactive_thumb
    };

    div()
        .h(style.control_height)
        .flex()
        .items_center()
        .justify_end()
        .child(
            div()
                .id(id)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .w(style.width)
                .h(style.height)
                .rounded_full()
                .border(style.outer_border_width)
                .border_color(border)
                .hover(move |this| this.border_color(style.active_border))
                .on_click(move |_, window, cx| on_change(&next_checked, window, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .when(checked, |this| this.justify_end())
                        .when(!checked, |this| this.justify_start())
                        .w(style.track_width)
                        .h(style.track_height)
                        .px(style.track_padding)
                        .rounded_full()
                        .border(style.track_border_width)
                        .border_color(border)
                        .bg(track_background)
                        .child(
                            div()
                                .size(style.thumb_size)
                                .rounded_full()
                                .bg(thumb)
                                .shadow_xs(),
                        ),
                ),
        )
}

pub fn yttt_labeled_switch<H>(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    checked: bool,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_change: H,
) -> Div
where
    H: Fn(&bool, &mut Window, &mut App) + 'static,
{
    div()
        .flex()
        .items_center()
        .gap(ui_style.spacing.sm)
        .text_sm()
        .child(label.into())
        .child(yttt_switch(id, checked, theme, ui_style, on_change))
}
