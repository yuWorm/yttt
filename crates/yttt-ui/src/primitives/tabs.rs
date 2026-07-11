use gpui::{Pixels, Rgba, px};

use crate::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttTabBarStyle {
    pub height: Pixels,
    pub item_height: Pixels,
    pub border_width: Pixels,
    pub active_background: Rgba,
    pub inactive_background: Rgba,
    pub hover_background: Rgba,
}

pub fn yttt_tabbar_style(theme: WorkbenchTheme) -> YtttTabBarStyle {
    YtttTabBarStyle {
        height: px(32.0),
        item_height: px(32.0),
        border_width: px(1.0),
        active_background: theme.surface,
        inactive_background: theme.app_background,
        hover_background: theme.hover_surface,
    }
}
