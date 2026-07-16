use gpui::{Pixels, Rgba};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttStatusTone {
    Neutral,
    Running,
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttStatusDotStyle {
    pub size: Pixels,
    pub color: Rgba,
}

pub fn yttt_status_dot_style(
    tone: YtttStatusTone,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttStatusDotStyle {
    let color = match tone {
        YtttStatusTone::Neutral => theme.text_subtle,
        YtttStatusTone::Running => theme.accent,
        YtttStatusTone::Success => theme.success,
        YtttStatusTone::Warning => theme.warning,
        YtttStatusTone::Error => theme.danger,
    };

    YtttStatusDotStyle {
        size: ui_style.status_dot_size,
        color,
    }
}
