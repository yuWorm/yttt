use gpui::{Rems, Rgba, rems};

use crate::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSwitchStyle {
    pub width: Rems,
    pub height: Rems,
    pub track_width: Rems,
    pub track_height: Rems,
    pub track_padding: Rems,
    pub thumb_size: Rems,
    pub control_height: Rems,
    pub active_background: Rgba,
    pub inactive_background: Rgba,
    pub active_border: Rgba,
    pub inactive_border: Rgba,
    pub active_thumb: Rgba,
    pub inactive_thumb: Rgba,
}

pub fn yttt_switch_style(theme: WorkbenchTheme) -> YtttSwitchStyle {
    YtttSwitchStyle {
        width: rems(2.625),
        height: rems(1.625),
        track_width: rems(2.125),
        track_height: rems(1.25),
        track_padding: rems(0.125),
        thumb_size: rems(0.875),
        control_height: rems(2.0),
        active_background: theme.accent,
        inactive_background: theme.active_surface,
        active_border: theme.focus_ring,
        inactive_border: theme.border_strong,
        active_thumb: theme.text,
        inactive_thumb: theme.text_subtle,
    }
}
