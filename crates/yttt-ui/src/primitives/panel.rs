use gpui::{Pixels, Rgba, px, rgba};

use crate::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttPanelKind {
    Palette,
    Settings,
    Dialog,
    Editor,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttPanelStyle {
    pub width: Pixels,
    pub height: Option<Pixels>,
    pub max_width: Pixels,
    pub max_height: Pixels,
    pub radius: Pixels,
    pub padding: Pixels,
    pub overlay: Rgba,
    pub background: Rgba,
    pub border: Rgba,
}

pub fn yttt_panel_style(kind: YtttPanelKind, theme: WorkbenchTheme) -> YtttPanelStyle {
    let overlay = match kind {
        YtttPanelKind::Dialog => rgba(0x00000073),
        YtttPanelKind::Palette | YtttPanelKind::Settings | YtttPanelKind::Editor => {
            rgba(0x00000066)
        }
    };
    let (width, height, max_width, max_height, radius, padding) = match kind {
        YtttPanelKind::Palette => (px(760.0), None, px(900.0), px(480.0), px(8.0), px(0.0)),
        YtttPanelKind::Settings => (
            px(900.0),
            Some(px(560.0)),
            px(940.0),
            px(600.0),
            px(8.0),
            px(0.0),
        ),
        YtttPanelKind::Dialog => (px(420.0), None, px(420.0), px(420.0), px(8.0), px(16.0)),
        YtttPanelKind::Editor => (
            px(860.0),
            Some(px(560.0)),
            px(960.0),
            px(620.0),
            px(8.0),
            px(0.0),
        ),
    };

    YtttPanelStyle {
        width,
        height,
        max_width,
        max_height,
        radius,
        padding,
        overlay,
        background: theme.surface,
        border: theme.border_strong,
    }
}
