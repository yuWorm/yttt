use gpui::{App, Div, ElementId, Pixels, Rems, Rgba, SharedString, Window, div, prelude::*, px};

use gpui_base::Switch;

use crate::{
    style::{UiStyle, UiStyleId},
    theme::WorkbenchTheme,
};

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
    pub hover_border: Rgba,
    pub active_thumb: Rgba,
    pub inactive_thumb: Rgba,
}

pub fn yttt_switch_style(theme: WorkbenchTheme, ui_style: UiStyle) -> YtttSwitchStyle {
    let zed = ui_style.id == UiStyleId::Zed;
    YtttSwitchStyle {
        width: ui_style.switches.width,
        height: ui_style.switches.height,
        track_width: ui_style.switches.track_width,
        track_height: ui_style.switches.track_height,
        track_padding: ui_style.switches.track_padding,
        thumb_size: ui_style.switches.thumb_size,
        control_height: ui_style.switches.control_height,
        outer_border_width: if zed {
            px(0.0)
        } else {
            ui_style.border.hairline
        },
        track_border_width: ui_style.border.hairline,
        active_background: if zed {
            theme.accent.alpha(0.4)
        } else {
            theme.accent
        },
        inactive_background: theme.element_background,
        active_border: if zed {
            theme.accent.alpha(0.2)
        } else {
            theme.border_variant
        },
        inactive_border: if zed {
            theme.border
        } else {
            theme.border_variant
        },
        hover_border: theme.border_focused,
        active_thumb: theme.text,
        inactive_thumb: if zed {
            theme.icon_disabled
        } else {
            theme.text_muted
        },
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

    let switch = Switch::new(id)
        .checked(checked)
        .on_change(move |next, _, window, cx| on_change(&next, window, cx))
        .cursor_pointer()
        .flex()
        .items_center()
        .rounded_full();
    let thumb = div()
        .size(style.thumb_size)
        .rounded_full()
        .bg(thumb)
        .shadow_xs();
    let align = move |track: Div| {
        track
            .flex()
            .items_center()
            .when(checked, |this| this.justify_end())
            .when(!checked, |this| this.justify_start())
    };
    // Zed's switch is a single track, so the track itself shows focus and hover.
    let control = if ui_style.id == UiStyleId::Zed {
        switch
            .when(checked, |this| this.justify_end())
            .when(!checked, |this| this.justify_start())
            .w(style.track_width)
            .h(style.track_height)
            .px(style.track_padding)
            .border(style.track_border_width)
            .border_color(border)
            .bg(track_background)
            .hover(move |this| this.border_color(style.hover_border))
            .focus_visible(move |this| this.border_color(style.hover_border))
            .child(thumb)
    } else {
        switch
            .justify_center()
            .w(style.width)
            .h(style.height)
            .border(style.outer_border_width)
            .border_color(border)
            .hover(move |this| this.border_color(style.hover_border))
            .focus_visible(move |this| this.border_color(style.hover_border))
            .child(
                align(div())
                    .w(style.track_width)
                    .h(style.track_height)
                    .px(style.track_padding)
                    .rounded_full()
                    .border(style.track_border_width)
                    .border_color(border)
                    .bg(track_background)
                    .child(thumb),
            )
    };

    div()
        .h(style.control_height)
        .flex()
        .items_center()
        .justify_end()
        .child(control)
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
