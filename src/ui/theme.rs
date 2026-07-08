use gpui::{Pixels, Rgba, px, rgb};

#[derive(Clone, Copy, Debug)]
pub struct WorkbenchTheme {
    pub app_background: Rgba,
    pub surface: Rgba,
    pub surface_elevated: Rgba,
    pub sidebar_background: Rgba,
    pub terminal_background: Rgba,
    pub border: Rgba,
    pub border_strong: Rgba,
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
    pub terminal_font_size: Pixels,
}

impl WorkbenchTheme {
    pub fn dark() -> Self {
        Self {
            app_background: rgb(0x0f0f0f),
            surface: rgb(0x151515),
            surface_elevated: rgb(0x1d1d1d),
            sidebar_background: rgb(0x171717),
            terminal_background: rgb(0x0b0b0b),
            border: rgb(0x2a2a2a),
            border_strong: rgb(0x3a3a3a),
            text: rgb(0xf5f5f5),
            text_muted: rgb(0xa3a3a3),
            text_subtle: rgb(0x737373),
            accent: rgb(0x7dd3fc),
            active_surface: rgb(0x242424),
            hover_surface: rgb(0x262626),
            danger: rgb(0xef4444),
            success: rgb(0x22c55e),
            warning: rgb(0xf59e0b),
            focus_ring: rgb(0x38bdf8),
            terminal_font_size: px(13.0),
        }
    }
}
