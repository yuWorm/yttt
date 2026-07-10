use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttInputKind {
    Dialog,
    Palette,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttInputStyle {
    pub height: Pixels,
    pub radius: Pixels,
    pub background: Rgba,
    pub border: Rgba,
    pub focused_border: Rgba,
    pub text: Rgba,
    pub placeholder: Rgba,
}

pub fn yttt_input_style(kind: YtttInputKind, theme: WorkbenchTheme) -> YtttInputStyle {
    YtttInputStyle {
        height: match kind {
            YtttInputKind::Dialog => px(34.0),
            YtttInputKind::Palette => px(42.0),
            YtttInputKind::Settings => px(32.0),
        },
        radius: match kind {
            YtttInputKind::Settings => px(6.0),
            YtttInputKind::Dialog | YtttInputKind::Palette => px(7.0),
        },
        background: theme.surface_elevated,
        border: theme.border,
        focused_border: theme.focus_ring,
        text: theme.text,
        placeholder: theme.text_subtle,
    }
}
