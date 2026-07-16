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
    pub border: Rgba,
    pub text: Rgba,
}

pub fn yttt_button_style(
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttButtonStyle {
    let active_background = ui_style.active_background(theme);
    let hover_background = ui_style.hover_background(theme);
    let (background, hover_background, border, text) = match variant {
        YtttButtonVariant::Primary => (
            active_background,
            hover_background,
            theme.border,
            theme.text,
        ),
        YtttButtonVariant::Secondary => (
            theme.surface_elevated,
            hover_background,
            theme.border,
            theme.text_muted,
        ),
        YtttButtonVariant::Ghost => (
            theme.app_background.blend(gpui::rgba(0x00000000)),
            hover_background,
            gpui::rgba(0x00000000),
            theme.text_muted,
        ),
        YtttButtonVariant::Danger => {
            let hover_background = theme.surface.blend(Rgba {
                a: 0.3,
                ..theme.danger
            });
            (
                theme.danger.blend(gpui::rgba(0x00000022)),
                hover_background,
                theme.danger,
                theme.text,
            )
        }
    };

    YtttButtonStyle {
        radius: ui_style.radius.control,
        background,
        hover_background,
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
        .active(style.background.into())
        .shadow(ui_style.component.shadow)
}

pub fn yttt_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &App,
) -> Button {
    let style = yttt_button_style(variant, theme, ui_style);
    Button::new(id)
        .label(label)
        .xsmall()
        .compact()
        .h(ui_style.controls.button_height)
        .px(ui_style.controls.button_padding_x)
        .rounded(style.radius)
        .outline()
        .border_color(style.border)
        .custom(yttt_button_variant(variant, theme, ui_style, cx))
        .text_color(style.text)
}
