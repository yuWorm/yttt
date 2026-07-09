use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSwitchStyle {
    pub width: Pixels,
    pub height: Pixels,
    pub track_width: Pixels,
    pub track_height: Pixels,
    pub track_padding: Pixels,
    pub thumb_size: Pixels,
    pub control_height: Pixels,
    pub active_background: Rgba,
    pub inactive_background: Rgba,
    pub active_border: Rgba,
    pub inactive_border: Rgba,
    pub active_thumb: Rgba,
    pub inactive_thumb: Rgba,
}

pub fn yttt_switch_style(theme: WorkbenchTheme) -> YtttSwitchStyle {
    YtttSwitchStyle {
        width: px(42.0),
        height: px(26.0),
        track_width: px(34.0),
        track_height: px(20.0),
        track_padding: px(2.0),
        thumb_size: px(14.0),
        control_height: px(32.0),
        active_background: theme.accent,
        inactive_background: theme.active_surface,
        active_border: theme.focus_ring,
        inactive_border: theme.border_strong,
        active_thumb: theme.text,
        inactive_thumb: theme.text_subtle,
    }
}
