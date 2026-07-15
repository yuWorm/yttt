use super::icon_button::{YtttIconButtonKind, yttt_icon_button_style};
use gpui::{Pixels, Rems, Rgba, px, rems, rgba};

use crate::theme::WorkbenchTheme;

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

pub fn yttt_tabbar_style(theme: WorkbenchTheme) -> YtttTabBarStyle {
    YtttTabBarStyle {
        height: rems(2.0),
        item_height: rems(2.0),
        border_width: px(1.0),
        close_slot_size: yttt_icon_button_style(YtttIconButtonKind::TabClose, theme).size,
        active_background: theme.active_surface,
        inactive_background: rgba(0x00000000),
        hover_background: theme.hover_surface,
    }
}
