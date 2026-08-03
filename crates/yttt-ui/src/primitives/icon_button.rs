use gpui::{
    App, ClickEvent, Div, ElementId, Pixels, Rems, Rgba, Stateful, Window, div, prelude::*,
};
use gpui_component::{
    Icon, IconName, Sizable as _,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttIconButtonKind {
    Toolbar,
    SidebarHeader,
    TabClose,
    OverlayClose,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttIconButtonStyle {
    pub size: Rems,
    pub icon_size: Rems,
    pub radius: Pixels,
    pub background: Rgba,
    pub hover_background: Rgba,
    pub active_background: Rgba,
    pub text: Rgba,
    pub hover_text: Rgba,
    pub active_text: Rgba,
}

pub fn yttt_icon_button_style(
    kind: YtttIconButtonKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttIconButtonStyle {
    let (size, radius, text) = match kind {
        YtttIconButtonKind::Toolbar => (
            ui_style.icon_buttons.toolbar_size,
            ui_style.icon_buttons.toolbar_radius,
            theme.text_muted,
        ),
        YtttIconButtonKind::SidebarHeader => (
            ui_style.icon_buttons.sidebar_header_size,
            ui_style.icon_buttons.sidebar_header_radius,
            theme.text_subtle,
        ),
        YtttIconButtonKind::TabClose => (
            ui_style.icon_buttons.tab_close_size,
            ui_style.icon_buttons.tab_close_radius,
            theme.text_subtle,
        ),
        YtttIconButtonKind::OverlayClose => (
            ui_style.icon_buttons.overlay_close_size,
            ui_style.icon_buttons.overlay_close_radius,
            theme.text_muted,
        ),
    };

    YtttIconButtonStyle {
        size,
        icon_size: ui_style.icon_buttons.icon_size,
        radius,
        background: theme.ghost_element_background,
        hover_background: theme.ghost_element_hover,
        active_background: theme.ghost_element_active,
        text,
        hover_text: theme.text,
        active_text: theme.accent,
    }
}

pub fn yttt_icon_button<H>(
    id: impl Into<ElementId>,
    icon: IconName,
    kind: YtttIconButtonKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = yttt_icon_button_style(kind, theme, ui_style);

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(style.size)
        .rounded(style.radius)
        .bg(style.background)
        .text_color(style.text)
        .hover(move |this| this.bg(style.hover_background).text_color(style.hover_text))
        .active(move |this| {
            this.bg(style.active_background)
                .text_color(style.active_text)
        })
        .on_click(on_click)
        .child(Icon::new(icon).size(style.icon_size))
}

pub fn yttt_menu_icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    kind: YtttIconButtonKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    cx: &App,
) -> Button {
    let style = yttt_icon_button_style(kind, theme, ui_style);
    Button::new(id)
        .ghost()
        .xsmall()
        .compact()
        .icon(icon)
        .w(style.size)
        .h(style.size)
        .p_0()
        .rounded(style.radius)
        .text_color(style.text)
        .custom(
            ButtonCustomVariant::new(cx)
                .color(style.background.into())
                .foreground(style.text.into())
                .hover(style.hover_background.into())
                .active(style.active_background.into()),
        )
        .bg(style.background)
}
