use gpui::{Pixels, Rgba, px};

use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttNotificationTone {
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttNotificationStyle {
    pub width: Pixels,
    pub min_height: Pixels,
    pub padding_x: Pixels,
    pub padding_y: Pixels,
    pub gap: Pixels,
    pub radius: Pixels,
    pub border_width: Pixels,
    pub icon_size: Pixels,
    pub background: Rgba,
    pub border: Rgba,
    pub title: Rgba,
    pub context: Rgba,
    pub action: Rgba,
    pub action_background: Rgba,
    pub tone: Rgba,
}

pub fn yttt_notification_style(
    tone: YtttNotificationTone,
    theme: WorkbenchTheme,
) -> YtttNotificationStyle {
    let tone = match tone {
        YtttNotificationTone::Success => theme.success,
        YtttNotificationTone::Warning => theme.warning,
        YtttNotificationTone::Error => theme.danger,
    };

    YtttNotificationStyle {
        width: px(360.0),
        min_height: px(44.0),
        padding_x: px(12.0),
        padding_y: px(8.0),
        gap: px(8.0),
        radius: px(8.0),
        border_width: px(1.0),
        icon_size: px(14.0),
        background: theme.surface,
        border: theme.border,
        title: theme.text,
        context: theme.text_subtle,
        action: theme.text_muted,
        action_background: theme.hover_surface,
        tone,
    }
}
