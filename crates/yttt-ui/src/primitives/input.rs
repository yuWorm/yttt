use gpui::{Pixels, Rems, Rgba, px, rems};

use crate::theme::WorkbenchTheme;

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

pub fn yttt_input_style(kind: YtttInputKind, theme: WorkbenchTheme) -> YtttInputStyle {
    YtttInputStyle {
        height: match kind {
            YtttInputKind::Dialog => rems(2.125),
            YtttInputKind::Palette => rems(2.625),
            YtttInputKind::Settings => rems(2.0),
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
