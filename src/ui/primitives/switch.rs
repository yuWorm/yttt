use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSwitchStyle {
    pub width: Pixels,
    pub height: Pixels,
    pub control_height: Pixels,
    pub active_background: Rgba,
    pub inactive_background: Rgba,
    pub thumb: Rgba,
}

pub fn yttt_switch_style(theme: WorkbenchTheme) -> YtttSwitchStyle {
    YtttSwitchStyle {
        width: px(36.0),
        height: px(20.0),
        control_height: px(32.0),
        active_background: theme.active_surface,
        inactive_background: theme.surface_elevated,
        thumb: theme.text,
    }
}
