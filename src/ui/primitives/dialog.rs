use gpui::{Pixels, Rgba, px, rgba};

use crate::ui::theme::WorkbenchTheme;

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
    YtttDialogStyle {
        max_width: px(420.0),
        radius: px(8.0),
        padding: px(16.0),
        overlay: rgba(0x00000073),
        background: theme.surface,
        border: theme.border_strong,
        text: theme.text,
        hint: theme.text_subtle,
    }
}
