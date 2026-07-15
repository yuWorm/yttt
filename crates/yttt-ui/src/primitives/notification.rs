use gpui::{Pixels, Rems, Rgba, px, rems};

use crate::theme::WorkbenchTheme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttNotificationTone {
    Success,
    Warning,
    Error,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttNotificationStyle {
    pub width: Pixels,
    pub min_height: Rems,
    pub padding_x: Rems,
    pub padding_y: Rems,
    pub gap: Rems,
    pub radius: Pixels,
    pub border_width: Pixels,
    pub icon_size: Rems,
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
        min_height: rems(2.75),
        padding_x: rems(0.75),
        padding_y: rems(0.5),
        gap: rems(0.5),
        radius: px(8.0),
        border_width: px(1.0),
        icon_size: rems(0.875),
        background: theme.surface,
        border: theme.border,
        title: theme.text,
        context: theme.text_subtle,
        action: theme.text_muted,
        action_background: theme.hover_surface,
        tone,
    }
}
