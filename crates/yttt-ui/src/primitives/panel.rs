use gpui::{Pixels, Rgba, px};

use crate::{style::UiStyle, theme::WorkbenchTheme};

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
    pub border_width: Pixels,
    pub background: Rgba,
    pub border: Rgba,
    pub shadow: bool,
}

pub fn yttt_panel_style(
    kind: YtttPanelKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttPanelStyle {
    let overlay = match kind {
        YtttPanelKind::Dialog => ui_style.panels.dialog_overlay,
        YtttPanelKind::Palette | YtttPanelKind::Settings | YtttPanelKind::Editor => {
            ui_style.panels.panel_overlay
        }
    };
    let (width, height, max_width, max_height, padding) = match kind {
        YtttPanelKind::Palette => (px(760.0), None, px(900.0), px(480.0), px(0.0)),
        YtttPanelKind::Settings => (px(900.0), Some(px(560.0)), px(940.0), px(600.0), px(0.0)),
        YtttPanelKind::Dialog => (
            px(420.0),
            None,
            px(420.0),
            px(420.0),
            ui_style.panels.dialog_padding,
        ),
        YtttPanelKind::Editor => (px(860.0), Some(px(560.0)), px(960.0), px(620.0), px(0.0)),
    };

    YtttPanelStyle {
        width,
        height,
        max_width,
        max_height,
        radius: ui_style.panels.radius,
        padding,
        overlay,
        background: theme.surface.alpha(1.0),
        border_width: ui_style.border.hairline,
        border: theme.border_strong,
        shadow: ui_style.panels.shadow,
    }
}
