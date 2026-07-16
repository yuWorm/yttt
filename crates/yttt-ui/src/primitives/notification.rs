use gpui::{Pixels, Rems, Rgba, px};

use crate::{style::UiStyle, theme::WorkbenchTheme};

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
    pub action_radius: Pixels,
    pub action_padding_x: Rems,
    pub action_padding_y: Rems,
    pub shadow: bool,
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
    ui_style: UiStyle,
) -> YtttNotificationStyle {
    let tone = match tone {
        YtttNotificationTone::Success => theme.success,
        YtttNotificationTone::Warning => theme.warning,
        YtttNotificationTone::Error => theme.danger,
    };

    YtttNotificationStyle {
        width: px(360.0),
        min_height: ui_style.notifications.min_height,
        padding_x: ui_style.notifications.padding_x,
        padding_y: ui_style.notifications.padding_y,
        gap: ui_style.notifications.gap,
        radius: ui_style.notifications.radius,
        border_width: ui_style.notifications.border_width,
        icon_size: ui_style.notifications.icon_size,
        action_radius: ui_style.notifications.action_radius,
        action_padding_x: ui_style.notifications.action_padding_x,
        action_padding_y: ui_style.notifications.action_padding_y,
        shadow: ui_style.component.shadow,
        background: theme.surface,
        border: theme.border,
        title: theme.text,
        context: theme.text_subtle,
        action: theme.text_muted,
        action_background: ui_style.hover_background(theme),
        tone,
    }
}
