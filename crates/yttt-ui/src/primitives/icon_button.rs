use gpui::{Pixels, Rems, Rgba, px, rems, rgba};

use crate::theme::WorkbenchTheme;

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
) -> YtttIconButtonStyle {
    let transparent = rgba(0x00000000);
    let (size, radius, border_width, text) = match kind {
        YtttIconButtonKind::Toolbar => (rems(1.75), px(0.0), px(1.0), theme.text_muted),
        YtttIconButtonKind::SidebarHeader => (rems(1.5), px(4.0), px(0.0), theme.text_subtle),
        YtttIconButtonKind::TabClose => (rems(1.0), px(4.0), px(0.0), theme.text_subtle),
        YtttIconButtonKind::OverlayClose => (rems(1.75), px(6.0), px(0.0), theme.text_muted),
    };

    YtttIconButtonStyle {
        size,
        icon_size: rems(0.75),
        radius,
        border_width,
        background: transparent,
        hover_background: theme.hover_surface,
        border: if border_width == px(0.0) {
            transparent
        } else {
            theme.border
        },
        text,
        hover_text: theme.text,
    }
}
