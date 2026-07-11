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
    pub fn dark() -> Self {
        Self {
            app_background: rgb(0x1f2329),
            surface: rgb(0x23272e),
            surface_elevated: rgb(0x23272e),
            titlebar_background: rgb(0x1f2329),
            sidebar_background: rgb(0x1f2329),
            tabbar_background: rgb(0x1f2329),
            terminal_background: rgb(0x1b1e23),
            border: rgb(0x343a43),
            border_strong: rgb(0x414852),
            split_line: rgb(0x343a43),
            split_line_active: rgb(0x414852),
            text: rgb(0xe6e8eb),
            text_muted: rgb(0xb8bcc2),
            text_subtle: rgb(0x7d8794),
            accent: rgb(0x7aa2f7),
            active_surface: rgb(0x2e333b),
            hover_surface: rgb(0x292e36),
            danger: rgb(0xef4444),
            success: rgb(0x22c55e),
            warning: rgb(0xf59e0b),
            focus_ring: rgb(0x7aa2f7),
            selection: rgb(0x7aa2f7),
            focused_pane_border: rgb(0x6f7785),
            split_line_width: px(1.0),
            split_hit_area_width: px(7.0),
        }
    }
}
