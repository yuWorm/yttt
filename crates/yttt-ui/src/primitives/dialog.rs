use gpui::{App, Div, IntoElement, Pixels, RenderOnce, Rgba, Window};
use gpui_base::FocusTrapElement as _;

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

/// A modal layer that keeps Tab navigation inside its content.
#[derive(IntoElement)]
pub struct YtttDialogOverlay {
    layer: Div,
}

impl RenderOnce for YtttDialogOverlay {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let focus = window
            .use_keyed_state("yttt-dialog-focus", cx, |_, cx| cx.focus_handle())
            .read(cx)
            .clone();
        self.layer.focus_trap("yttt-dialog-trap", &focus)
    }
}

pub fn yttt_dialog_overlay(
    content: impl IntoElement,
    placement: YtttDialogPlacement,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttDialogOverlay {
    let placement = match placement {
        YtttDialogPlacement::Top => YtttOverlayPlacement::Top,
        YtttDialogPlacement::Center => YtttOverlayPlacement::Center,
    };
    YtttDialogOverlay {
        layer: yttt_panel_overlay(content, YtttPanelKind::Dialog, placement, theme, ui_style),
    }
}
