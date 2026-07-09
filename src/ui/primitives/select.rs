use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSelectStyle {
    pub width: Pixels,
    pub height: Pixels,
    pub radius: Pixels,
    pub menu_width: Pixels,
    pub background: Rgba,
    pub border: Rgba,
    pub text: Rgba,
}

pub fn yttt_select_style(theme: WorkbenchTheme) -> YtttSelectStyle {
    YtttSelectStyle {
        width: px(220.0),
        height: px(32.0),
        radius: px(6.0),
        menu_width: px(280.0),
        background: theme.surface_elevated,
        border: theme.border,
        text: theme.text,
    }
}
