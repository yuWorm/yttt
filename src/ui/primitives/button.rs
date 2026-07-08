use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttButtonVariant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttButtonStyle {
    pub height: Pixels,
    pub radius: Pixels,
    pub padding_x: Pixels,
    pub background: Rgba,
    pub hover_background: Rgba,
    pub border: Rgba,
    pub text: Rgba,
}

pub fn yttt_button_style(variant: YtttButtonVariant, theme: WorkbenchTheme) -> YtttButtonStyle {
    let (background, hover_background, border, text) = match variant {
        YtttButtonVariant::Primary => (
            theme.active_surface,
            theme.hover_surface,
            theme.border,
            theme.text,
        ),
        YtttButtonVariant::Secondary => (
            theme.surface_elevated,
            theme.hover_surface,
            theme.border,
            theme.text_muted,
        ),
        YtttButtonVariant::Ghost => (
            theme.app_background.blend(gpui::rgba(0x00000000)),
            theme.hover_surface,
            gpui::rgba(0x00000000),
            theme.text_muted,
        ),
        YtttButtonVariant::Danger => (
            theme.danger.blend(gpui::rgba(0x00000022)),
            theme.danger,
            theme.danger,
            theme.text,
        ),
    };

    YtttButtonStyle {
        height: px(28.0),
        radius: px(6.0),
        padding_x: px(12.0),
        background,
        hover_background,
        border,
        text,
    }
}
