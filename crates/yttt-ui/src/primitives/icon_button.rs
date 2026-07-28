use gpui::{
    App, ClickEvent, Div, ElementId, Pixels, Rems, Rgba, Stateful, Window, div, prelude::*, px,
    rgba,
};
use gpui_component::{
    Icon, IconName, Sizable as _,
    button::{Button, ButtonVariants as _},
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
    pub border_width: Pixels,
    pub background: Rgba,
    pub hover_background: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub hover_text: Rgba,
}

pub fn yttt_icon_button_style(
    kind: YtttIconButtonKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttIconButtonStyle {
    let transparent = rgba(0x00000000);
    let (size, radius, border_width, text) = match kind {
        YtttIconButtonKind::Toolbar => (
            ui_style.icon_buttons.toolbar_size,
            ui_style.icon_buttons.toolbar_radius,
            ui_style.icon_buttons.toolbar_border_width,
            theme.text_muted,
        ),
        YtttIconButtonKind::SidebarHeader => (
            ui_style.icon_buttons.sidebar_header_size,
            ui_style.icon_buttons.sidebar_header_radius,
            px(0.0),
            theme.text_subtle,
        ),
        YtttIconButtonKind::TabClose => (
            ui_style.icon_buttons.tab_close_size,
            ui_style.icon_buttons.tab_close_radius,
            px(0.0),
            theme.text_subtle,
        ),
        YtttIconButtonKind::OverlayClose => (
            ui_style.icon_buttons.overlay_close_size,
            ui_style.icon_buttons.overlay_close_radius,
            px(0.0),
            theme.text_muted,
        ),
    };

    YtttIconButtonStyle {
        size,
        icon_size: ui_style.icon_buttons.icon_size,
        radius,
        border_width,
        background: transparent,
        hover_background: ui_style.hover_background(theme),
        border: if border_width == px(0.0) {
            transparent
        } else {
            theme.border
        },
        text,
        hover_text: theme.text,
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
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .text_color(style.text)
        .hover(move |this| this.bg(style.hover_background).text_color(style.hover_text))
        .on_click(on_click)
        .child(Icon::new(icon).size(style.icon_size))
}

pub fn yttt_menu_icon_button(
    id: impl Into<ElementId>,
    icon: IconName,
    kind: YtttIconButtonKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
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
        .border(style.border_width)
        .border_color(style.border)
        .text_color(style.text)
}
