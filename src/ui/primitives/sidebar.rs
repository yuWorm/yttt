use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSidebarStyle {
    pub width: Pixels,
    pub collapsed_width: Pixels,
    pub border_width: Pixels,
    pub item_height: Pixels,
    pub item_padding_x: Pixels,
    pub background: Rgba,
    pub active_background: Rgba,
    pub hover_background: Rgba,
}

pub fn yttt_sidebar_style(theme: WorkbenchTheme) -> YtttSidebarStyle {
    YtttSidebarStyle {
        width: px(216.0),
        collapsed_width: px(46.0),
        border_width: px(1.0),
        item_height: px(28.0),
        item_padding_x: px(8.0),
        background: theme.app_background,
        active_background: theme.active_surface,
        hover_background: theme.hover_surface,
    }
}
