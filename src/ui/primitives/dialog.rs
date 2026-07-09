use gpui::{Pixels, Rgba};

use crate::ui::{
    primitives::panel::{YtttPanelKind, yttt_panel_style},
    theme::WorkbenchTheme,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttDialogStyle {
    pub max_width: Pixels,
    pub radius: Pixels,
    pub padding: Pixels,
    pub overlay: Rgba,
    pub background: Rgba,
    pub border: Rgba,
    pub text: Rgba,
    pub hint: Rgba,
}

pub fn yttt_dialog_style(theme: WorkbenchTheme) -> YtttDialogStyle {
    let panel = yttt_panel_style(YtttPanelKind::Dialog, theme);
    YtttDialogStyle {
        max_width: panel.max_width,
        radius: panel.radius,
        padding: panel.padding,
        overlay: panel.overlay,
        background: panel.background,
        border: panel.border,
        text: theme.text,
        hint: theme.text_subtle,
    }
}
