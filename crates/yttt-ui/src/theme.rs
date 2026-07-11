use gpui::{Pixels, Rgba, px, rgb};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WorkbenchTheme {
    pub app_background: Rgba,
    pub surface: Rgba,
    pub surface_elevated: Rgba,
    pub titlebar_background: Rgba,
    pub sidebar_background: Rgba,
    pub tabbar_background: Rgba,
    pub terminal_background: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
    pub split_line: Rgba,
    pub split_line_active: Rgba,
    pub text: Rgba,
    pub text_muted: Rgba,
    pub text_subtle: Rgba,
    pub accent: Rgba,
    pub active_surface: Rgba,
    pub hover_surface: Rgba,
    pub danger: Rgba,
    pub success: Rgba,
    pub warning: Rgba,
    pub focus_ring: Rgba,
    pub selection: Rgba,
    pub focused_pane_border: Rgba,
    pub split_line_width: Pixels,
    pub split_hit_area_width: Pixels,
}

impl WorkbenchTheme {
    pub fn one_dark() -> Self {
        Self {
            app_background: rgb(0x23272e),
            surface: rgb(0x23272e),
            surface_elevated: rgb(0x1e2227),
            titlebar_background: rgb(0x23272e),
            sidebar_background: rgb(0x23272e),
            tabbar_background: rgb(0x1e2227),
            terminal_background: rgb(0x23272e),
            border: rgb(0x3e4452),
            border_strong: rgb(0x4e5666),
            split_line: rgb(0x3e4452),
            split_line_active: rgb(0x5a6375),
            text: rgb(0xabb2bf),
            text_muted: rgb(0x7f838c),
            text_subtle: rgb(0x495162),
            accent: rgb(0x61afef),
            active_surface: rgb(0x2c313a),
            hover_surface: rgb(0x2c313a),
            danger: rgb(0xc24038),
            success: rgb(0xa5e075),
            warning: rgb(0xd19a66),
            focus_ring: rgb(0x3e4452),
            selection: gpui::rgba(0x67769640),
            focused_pane_border: rgb(0x3e4452),
            split_line_width: px(1.0),
            split_hit_area_width: px(7.0),
        }
    }
}
