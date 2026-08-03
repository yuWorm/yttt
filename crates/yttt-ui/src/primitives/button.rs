use gpui::{App, ElementId, Pixels, Rgba, SharedString, prelude::*};
use gpui_component::{
    Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants},
};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttButtonVariant {
    Primary,
    Secondary,
    Ghost,
    Danger,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttButtonStyle {
    pub radius: Pixels,
    pub background: Rgba,
    pub hover_background: Rgba,
    pub active_background: Rgba,
    pub border: Rgba,
    pub text: Rgba,
}

pub fn yttt_button_style(
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttButtonStyle {
    let (background, hover_background, active_background, border, text) = match variant {
        YtttButtonVariant::Primary => (
            theme.element_background,
            theme.element_hover,
            theme.element_active,
            theme.border_variant,
            theme.text,
        ),
        YtttButtonVariant::Secondary => (
            theme.ghost_element_background,
            theme.ghost_element_hover,
            theme.ghost_element_active,
            theme.border_variant,
            theme.text,
        ),
        YtttButtonVariant::Ghost => (
            theme.ghost_element_background,
            theme.ghost_element_hover,
            theme.ghost_element_active,
            gpui::rgba(0x00000000),
            theme.text_muted,
        ),
        YtttButtonVariant::Danger => (
            theme.danger.alpha(0.22),
            theme.danger.alpha(0.30),
            theme.danger.alpha(0.38),
            theme.danger.alpha(0.7),
            theme.text,
        ),
    };

    YtttButtonStyle {
        radius: ui_style.radius.action,
        background,
        hover_background,
        active_background,
        border,
        text,
    }
}

pub fn yttt_button_variant(
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &App,
) -> ButtonCustomVariant {
    let style = yttt_button_style(variant, theme, ui_style);
    ButtonCustomVariant::new(cx)
        .color(style.background.into())
        .foreground(style.text.into())
        .hover(style.hover_background.into())
        .active(style.active_background.into())
        .shadow(ui_style.component.shadow)
}

pub fn yttt_button_base(
    id: impl Into<ElementId>,
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &App,
) -> Button {
    let style = yttt_button_style(variant, theme, ui_style);
    Button::new(id)
        .xsmall()
        .compact()
        .h(ui_style.controls.button_height)
        .px(ui_style.controls.button_padding_x)
        .rounded(style.radius)
        .outline()
        .border_color(style.border)
        .custom(yttt_button_variant(variant, theme, ui_style, cx))
        .bg(style.background)
        .text_color(style.text)
}

pub fn yttt_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &App,
) -> Button {
    yttt_button_base(id, variant, theme, ui_style, cx).label(label)
}
