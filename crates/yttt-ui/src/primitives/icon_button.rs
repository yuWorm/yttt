use gpui::{Pixels, Rems, Rgba, px, rgba};

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
