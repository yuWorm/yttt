use gpui::{Pixels, Rems, Rgba, px, rems};

use crate::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSelectStyle {
    pub width: Pixels,
    pub height: Rems,
    pub radius: Pixels,
    pub menu_width: Pixels,
    pub background: Rgba,
    pub border: Rgba,
    pub text: Rgba,
}

pub fn yttt_select_style(theme: WorkbenchTheme) -> YtttSelectStyle {
    YtttSelectStyle {
        width: px(220.0),
        height: rems(2.0),
        radius: px(6.0),
        menu_width: px(280.0),
        background: theme.surface_elevated,
        border: theme.border,
        text: theme.text,
    }
}
