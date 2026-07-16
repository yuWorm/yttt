use super::icon_button::{YtttIconButtonKind, yttt_icon_button_style};
use gpui::{Pixels, Rems, Rgba, rgba};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttTabBarStyle {
    pub height: Rems,
    pub item_height: Rems,
    pub border_width: Pixels,
    pub close_slot_size: Rems,
    pub active_background: Rgba,
    pub inactive_background: Rgba,
    pub hover_background: Rgba,
}

pub fn yttt_tabbar_style(theme: WorkbenchTheme, ui_style: UiStyle) -> YtttTabBarStyle {
    YtttTabBarStyle {
        height: ui_style.rows.tab_height,
        item_height: ui_style.rows.tab_height,
        border_width: ui_style.rows.tab_border_width,
        close_slot_size: yttt_icon_button_style(YtttIconButtonKind::TabClose, theme, ui_style).size,
        active_background: ui_style.active_background(theme),
        inactive_background: rgba(0x00000000),
        hover_background: ui_style.hover_background(theme),
    }
}
