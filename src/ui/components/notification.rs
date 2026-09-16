use super::*;
use crate::ui::primitives::button::{YtttButtonVariant, yttt_button};

pub fn notification_tone_for_toast(tone: ToastTone) -> YtttNotificationTone {
    match tone {
        ToastTone::Success => YtttNotificationTone::Success,
        ToastTone::Warning => YtttNotificationTone::Warning,
        ToastTone::Error => YtttNotificationTone::Error,
    }
}
type NotificationAction = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

pub fn workbench_agent_notification(
    item: ToastItem,
    action_label: impl Into<SharedString>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_action: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
) -> Notification {
    let on_action: NotificationAction = Rc::new(on_action);
    workbench_notification(
        item,
        Some((action_label.into(), on_action)),
        theme,
        ui_style,
    )
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
    let status = item.status.map(SharedString::from);
    let context = SharedString::from(item.context);

    yttt_notification_surface(tone, theme, ui_style).child(notification_content(
        status, title, context, None, icon, style,
    ))
}

fn workbench_notification(
    item: ToastItem,
    action: Option<(SharedString, NotificationAction)>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Notification {
    let tone = notification_tone_for_toast(item.tone);
    let style = yttt_notification_style(tone, theme, ui_style);
    let icon = notification_icon(tone);
    let title = SharedString::from(item.title);
    let status = item.status.map(SharedString::from);
    let context = SharedString::from(item.context);

    yttt_toast_notification(tone, theme, ui_style).content(move |_, _, cx| {
        let action = action.clone().map(|(label, on_action)| {
            yttt_button(
                "notification-action",
                label,
                YtttButtonVariant::Primary,
                WorkbenchTheme {
                    // Toasts are opaque even when the window material is translucent.
                    element_background: theme.element_background.alpha(1.0),
                    ..theme
                },
                ui_style,
                cx,
            )
            .debug_selector(|| "notification-action".into())
            .icon(IconName::ArrowRight)
            .on_click(cx.listener(move |notification, event, window, cx| {
                cx.stop_propagation();
                notification.dismiss(window, cx);
                on_action(event, window, cx);
            }))
        });

        notification_content(
            status.clone(),
            title.clone(),
            context.clone(),
            action,
            icon.clone(),
            style,
        )
        .into_any_element()
    })
}

fn notification_icon(tone: YtttNotificationTone) -> IconName {
    match tone {
        YtttNotificationTone::Info => IconName::Info,
        YtttNotificationTone::Success => IconName::CircleCheck,
        YtttNotificationTone::Warning => IconName::TriangleAlert,
        YtttNotificationTone::Error => IconName::CircleX,
    }
}

fn notification_content(
    status: Option<SharedString>,
    title: SharedString,
    context: SharedString,
    action: Option<Button>,
    icon: IconName,
    style: crate::ui::primitives::notification::YtttNotificationStyle,
) -> Div {
    div()
        .flex()
        .items_start()
        .gap(style.gap)
        .min_h(style.min_height)
        .w_full()
        .child(
            div()
                .debug_selector(|| "notification-icon".into())
                .flex_none()
                .flex()
                .items_center()
                .h(gpui::rems(1.25))
                .child(Icon::new(icon).size(style.icon_size).text_color(style.tone)),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .debug_selector(|| "notification-title".into())
                        .text_sm()
                        .line_height(gpui::rems(1.25))
                        .text_color(style.title)
                        .child(title),
                )
                .child(
                    div()
                        .debug_selector(|| "notification-body".into())
                        .text_xs()
                        .line_height(gpui::rems(1.25))
                        .text_color(style.context)
                        .child(context),
                )
                .when(status.is_some() || action.is_some(), |this| {
                    this.child(
                        div()
                            .flex()
                            .w_full()
                            .min_w_0()
                            .items_center()
                            .justify_between()
                            .gap(style.gap)
                            .pt_1()
                            .when_some(status, |this, status| {
                                this.child(
                                    div()
                                        .min_w_0()
                                        .flex_1()
                                        .text_xs()
                                        .text_color(style.tone)
                                        .truncate()
                                        .child(status),
                                )
                            })
                            .when_some(action, |this, action| {
                                this.child(div().flex_none().child(action))
                            }),
                    )
                }),
        )
}
