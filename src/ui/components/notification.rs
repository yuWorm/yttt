use super::*;

pub fn notification_tone_for_toast(tone: ToastTone) -> YtttNotificationTone {
    match tone {
        ToastTone::Success => YtttNotificationTone::Success,
        ToastTone::Warning => YtttNotificationTone::Warning,
        ToastTone::Error => YtttNotificationTone::Error,
    }
}

pub fn workbench_agent_notification(
    item: ToastItem,
    action_label: impl Into<SharedString>,
    theme: WorkbenchTheme,
) -> Notification {
    workbench_notification(item, Some(action_label.into()), theme)
}

pub fn workbench_status_notification(item: ToastItem, theme: WorkbenchTheme) -> Notification {
    workbench_notification(item, None, theme)
}

pub fn workbench_inline_notification(item: ToastItem, theme: WorkbenchTheme) -> Div {
    let tone = notification_tone_for_toast(item.tone);
    let style = yttt_notification_style(tone, theme);
    let icon = notification_icon(tone);
    let title = SharedString::from(item.title);
    let context = SharedString::from(item.context);

    div()
        .w(style.width)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .rounded(style.radius)
        .px(style.padding_x)
        .py(style.padding_y)
        .child(notification_content(title, context, None, icon, style))
}

fn workbench_notification(
    item: ToastItem,
    action_label: Option<SharedString>,
    theme: WorkbenchTheme,
) -> Notification {
    let tone = notification_tone_for_toast(item.tone);
    let style = yttt_notification_style(tone, theme);
    let icon = notification_icon(tone);
    let title = SharedString::from(item.title);
    let context = SharedString::from(item.context);

    Notification::new()
        .w(style.width)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .rounded(style.radius)
        .px(style.padding_x)
        .py(style.padding_y)
        .content(move |_, _, _| {
            notification_content(
                title.clone(),
                context.clone(),
                action_label.clone(),
                icon.clone(),
                style,
            )
            .into_any_element()
        })
}

fn notification_icon(tone: YtttNotificationTone) -> IconName {
    match tone {
        YtttNotificationTone::Success => IconName::CircleCheck,
        YtttNotificationTone::Warning => IconName::TriangleAlert,
        YtttNotificationTone::Error => IconName::CircleX,
    }
}

fn notification_content(
    title: SharedString,
    context: SharedString,
    action_label: Option<SharedString>,
    icon: IconName,
    style: crate::ui::primitives::notification::YtttNotificationStyle,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(style.gap)
        .min_h(style.min_height)
        .w_full()
        .child(Icon::new(icon).size(style.icon_size).text_color(style.tone))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_0()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(style.title)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(style.context)
                        .truncate()
                        .child(context),
                ),
        )
        .when_some(action_label, |this, action_label| {
            this.child(
                div()
                    .flex_none()
                    .rounded(px(5.0))
                    .bg(style.action_background)
                    .px(px(6.0))
                    .py(px(2.0))
                    .text_xs()
                    .text_color(style.action)
                    .child(action_label),
            )
        })
}
