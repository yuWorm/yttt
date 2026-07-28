use gpui::{Div, IntoElement, Pixels, Rgba};

use crate::{
    primitives::panel::{
        YtttOverlayPlacement, YtttPanelKind, yttt_panel, yttt_panel_overlay, yttt_panel_style,
    },
    style::UiStyle,
    theme::WorkbenchTheme,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttDialogStyle {
    pub max_width: Pixels,
    pub radius: Pixels,
    pub padding: Pixels,
    pub border_width: Pixels,
    pub shadow: bool,
    pub overlay: Rgba,

    pub background: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub hint: Rgba,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum YtttDialogPlacement {
    #[default]
    Top,
    Center,
}

pub fn yttt_dialog_style(theme: WorkbenchTheme, ui_style: UiStyle) -> YtttDialogStyle {
    let panel = yttt_panel_style(YtttPanelKind::Dialog, theme, ui_style);
    YtttDialogStyle {
        max_width: panel.max_width,
        radius: panel.radius,
        padding: panel.padding,
        overlay: panel.overlay,
        border_width: panel.border_width,
        shadow: panel.shadow,
        background: panel.background,
        border: panel.border,
        text: theme.text,
        hint: theme.text_subtle,
    }
}

pub fn yttt_dialog_surface(theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    yttt_panel(YtttPanelKind::Dialog, theme, ui_style)
}

pub fn yttt_dialog_overlay(
    content: impl IntoElement,
    placement: YtttDialogPlacement,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let placement = match placement {
        YtttDialogPlacement::Top => YtttOverlayPlacement::Top,
        YtttDialogPlacement::Center => YtttOverlayPlacement::Center,
    };
    yttt_panel_overlay(content, YtttPanelKind::Dialog, placement, theme, ui_style)
}
