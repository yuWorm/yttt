use gpui::{Pixels, Rgba, px, rgb};

#[derive(Clone, Copy, Debug)]
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
    pub focused_pane_border: Rgba,
    pub split_line_width: Pixels,
    pub split_hit_area_width: Pixels,
    pub terminal_font_size: Pixels,
}

impl WorkbenchTheme {
    pub fn dark() -> Self {
        Self {
            app_background: rgb(0x1f242d),
            surface: rgb(0x252b34),
            surface_elevated: rgb(0x2b323d),
            titlebar_background: rgb(0x202630),
            sidebar_background: rgb(0x222832),
            tabbar_background: rgb(0x242a34),
            terminal_background: rgb(0x171b22),
            border: rgb(0x343b46),
            border_strong: rgb(0x46505f),
            split_line: rgb(0x343b46),
            split_line_active: rgb(0x59687a),
            text: rgb(0xe7edf4),
            text_muted: rgb(0xaab4c0),
            text_subtle: rgb(0x778391),
            accent: rgb(0x8ab4f8),
            active_surface: rgb(0x303844),
            hover_surface: rgb(0x2b323d),
            danger: rgb(0xef4444),
            success: rgb(0x22c55e),
            warning: rgb(0xf59e0b),
            focus_ring: rgb(0x7aa2f7),
            focused_pane_border: rgb(0x5f789c),
            split_line_width: px(1.0),
            split_hit_area_width: px(7.0),
            terminal_font_size: px(13.0),
        }
    }
}
