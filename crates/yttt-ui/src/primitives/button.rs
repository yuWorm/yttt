use gpui::{ElementId, Pixels, Rgba, SharedString, prelude::*};
use gpui_base::Button;
use gpui_component::{Icon, IconName, Sizable as _, spinner::Spinner, tooltip::Tooltip};

use crate::{
    style::{UiStyle, UiStyleId},
    theme::WorkbenchTheme,
};

pub type YtttButton = Button;

pub trait YtttButtonExt: Sized {
    fn icon(self, icon: IconName) -> Self;
    fn tooltip_text(self, text: impl Into<SharedString>) -> Self;
    fn loading(self, loading: bool) -> Self;
}

impl YtttButtonExt for Button {
    fn icon(self, icon: IconName) -> Self {
        self.child(Icon::new(icon).small())
    }

    fn tooltip_text(self, text: impl Into<SharedString>) -> Self {
        let text = text.into();
        self.tooltip(move |window, cx| Tooltip::new(text.clone()).build(window, cx))
    }

    fn loading(self, loading: bool) -> Self {
        self.when(loading, |button| button.child(Spinner::new().xsmall()))
    }
}

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

pub fn yttt_button_base(
    id: impl Into<ElementId>,
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Button {
    let style = yttt_button_style(variant, theme, ui_style);
    let outlined = ui_style.id != UiStyleId::Zed
        || variant == YtttButtonVariant::Secondary
        || variant == YtttButtonVariant::Danger;
    Button::new(id)
        .h(ui_style.controls.button_height)
        .px(ui_style.controls.button_padding_x)
        .gap_1()
        .text_xs()
        .cursor_pointer()
        .rounded(style.radius)
        .when(outlined, |button| button.border_1())
        .border_color(style.border)
        .bg(style.background)
        .text_color(style.text)
        .enabled_hover(move |this| this.bg(style.hover_background))
        .enabled_active(move |this| this.bg(style.active_background))
        .styles(|styles| styles.disabled(|disabled| disabled.cursor_default()))
}

pub fn yttt_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Button {
    yttt_button_base(id, variant, theme, ui_style).child(label.into())
}
