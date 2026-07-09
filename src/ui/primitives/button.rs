use gpui::{App, ElementId, Pixels, Rgba, SharedString, prelude::*, px};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants},
};

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

pub fn yttt_button_variant(
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    cx: &App,
) -> ButtonCustomVariant {
    let style = yttt_button_style(variant, theme);
    ButtonCustomVariant::new(cx)
        .color(style.background.into())
        .foreground(style.text.into())
        .border(style.border.into())
        .hover(style.hover_background.into())
        .active(style.background.into())
        .shadow(false)
}

pub fn yttt_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    cx: &App,
) -> Button {
    let style = yttt_button_style(variant, theme);
    Button::new(id)
        .label(label)
        .xsmall()
        .compact()
        .h(style.height)
        .px(style.padding_x)
        .rounded(style.radius)
        .custom(yttt_button_variant(variant, theme, cx))
        .text_color(style.text)
}
