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
    ui_style: UiStyle,
) -> Notification {
    workbench_notification(item, Some(action_label.into()), theme, ui_style)
}

pub fn workbench_status_notification(
    item: ToastItem,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Notification {
    workbench_notification(item, None, theme, ui_style)
}

pub fn workbench_error_notification(
    item: ToastItem,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Notification {
    workbench_notification(item, None, theme, ui_style).autohide(true)
}

pub fn workbench_inline_notification(
    item: ToastItem,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let tone = notification_tone_for_toast(item.tone);
    let style = yttt_notification_style(tone, theme, ui_style);
    let icon = notification_icon(tone);
    let title = SharedString::from(item.title);
    let context = SharedString::from(item.context);

    div()
        .w(style.width)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .rounded(style.radius)
        .when(style.shadow, |this| this.shadow_lg())
        .px(style.padding_x)
        .py(style.padding_y)
        .child(notification_content(
            title, context, None, icon, style, None,
        ))
}

fn workbench_notification(
    item: ToastItem,
    action_label: Option<SharedString>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Notification {
    let tone = notification_tone_for_toast(item.tone);
    let style = yttt_notification_style(tone, theme, ui_style);
    let icon = notification_icon(tone);
    let title = SharedString::from(item.title);
    let context = SharedString::from(item.context);

    Notification::new()
        .autohide(true)
        .always_show_close_button(true)
        .w(style.width)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .rounded(style.radius)
        .when(style.shadow, |this| this.shadow_lg())
        .px(style.padding_x)
        .py(style.padding_y)
        .content(move |_, _, _| {
            notification_content(
                title.clone(),
                context.clone(),
                action_label.clone(),
                icon.clone(),
                style,
                None,
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
    close_button: Option<Stateful<Div>>,
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
                        .child(title),
                )
                .child(div().text_xs().text_color(style.context).child(context)),
        )
        .when_some(action_label, |this, action_label| {
            this.child(
                div()
                    .flex_none()
                    .rounded(style.action_radius)
                    .bg(style.action_background)
                    .px(style.action_padding_x)
                    .py(style.action_padding_y)
                    .text_xs()
                    .text_color(style.action)
                    .child(action_label),
            )
        })
        .when_some(close_button, |this, close_button| this.child(close_button))
}
