use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttStatusTone {
    Neutral,
    Running,
    Success,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttStatusDotStyle {
    pub size: Pixels,
    pub color: Rgba,
}

pub fn yttt_status_dot_style(tone: YtttStatusTone, theme: WorkbenchTheme) -> YtttStatusDotStyle {
    let color = match tone {
        YtttStatusTone::Neutral => theme.text_subtle,
        YtttStatusTone::Running => theme.accent,
        YtttStatusTone::Success => theme.success,
        YtttStatusTone::Error => theme.danger,
    };

    YtttStatusDotStyle {
        size: px(6.0),
        color,
    }
}
