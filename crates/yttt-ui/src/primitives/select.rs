use gpui::{Pixels, Rems, Rgba, px};

use crate::{style::UiStyle, theme::WorkbenchTheme};

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

pub fn yttt_select_style(theme: WorkbenchTheme, ui_style: UiStyle) -> YtttSelectStyle {
    YtttSelectStyle {
        width: px(220.0),
        height: ui_style.controls.settings_height,
        radius: ui_style.radius.control,
        menu_width: px(280.0),
        background: theme.surface_elevated,
        border: theme.border,
        text: theme.text,
    }
}
