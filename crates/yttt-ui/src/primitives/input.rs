use gpui::{Pixels, Rems, Rgba};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttInputKind {
    Dialog,
    Palette,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttInputStyle {
    pub height: Rems,
    pub radius: Pixels,
    pub background: Rgba,
    pub border: Rgba,
    pub focused_border: Rgba,
    pub text: Rgba,
    pub placeholder: Rgba,
}

pub fn yttt_input_style(
    kind: YtttInputKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttInputStyle {
    YtttInputStyle {
        height: match kind {
            YtttInputKind::Dialog => ui_style.controls.dialog_input_height,
            YtttInputKind::Palette => ui_style.controls.palette_input_height,
            YtttInputKind::Settings => ui_style.controls.settings_height,
        },
        radius: match kind {
            YtttInputKind::Settings => ui_style.radius.control,
            YtttInputKind::Dialog | YtttInputKind::Palette => ui_style.radius.input,
        },
        background: theme.surface_elevated,
        border: theme.border,
        focused_border: theme.focus_ring,
        text: theme.text,
        placeholder: theme.text_subtle,
    }
}
