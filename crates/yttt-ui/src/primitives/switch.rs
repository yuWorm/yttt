use gpui::{Pixels, Rems, Rgba};

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
