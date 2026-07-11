use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement as _, Keystroke,
    ParentElement as _, Pixels, SharedString, Stateful, StatefulInteractiveElement as _, Window,
    div, prelude::*, px,
};
use gpui_component::{
    Icon, IconName,
    button::{Button, ButtonVariants},
    kbd::Kbd,
    notification::Notification,
};

use crate::ui::{
    notifications::{ToastItem, ToastTone},
    palette::surface::palette_row_style,
    primitives::{
        icon_button::{YtttIconButtonKind, yttt_icon_button_style},
        notification::{YtttNotificationTone, yttt_notification_style},
        row::{YtttRowKind, yttt_row_style},
        switch::yttt_switch_style,
    },
    settings::keybinding_display::parse_keybinding_for_display,
    theme::WorkbenchTheme,
};
pub use yttt_ui::SelectableState;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActionEmphasis {
    Primary,
    Secondary,
}

pub fn selectable_state_classes(state: SelectableState) -> &'static str {
    match state {
        SelectableState::Active => "selectable active",
        SelectableState::Inactive => "selectable inactive",
    }
}

pub fn workbench_action_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    shortcut: &'static str,
    emphasis: ActionEmphasis,
) -> Button {
    let button = Button::new(id)
        .label(label)
        .compact()
        .rounded(px(6.0))
        .child(Kbd::new(shortcut_keystroke(shortcut)));

    match emphasis {
        ActionEmphasis::Primary => button.primary(),
        ActionEmphasis::Secondary => button.outline(),
    }
}

pub fn workbench_icon_button<H>(
    id: impl Into<ElementId>,
    icon: IconName,
    kind: YtttIconButtonKind,
    theme: WorkbenchTheme,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = yttt_icon_button_style(kind, theme);

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(style.size)
        .rounded(style.radius)
        .border_l(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .text_color(style.text)
        .hover(move |this| this.bg(style.hover_background).text_color(style.hover_text))
        .on_click(on_click)
        .child(Icon::new(icon).size(style.icon_size))
}

pub fn workbench_palette_item<H>(
    id: impl Into<ElementId>,
    title: impl Into<String>,
    subtitle: impl Into<String>,
    status: impl Into<String>,
    keybinding: Option<String>,
    state: SelectableState,
    enabled: bool,
    theme: WorkbenchTheme,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = palette_row_style(state, enabled, theme);
    let title = title.into();
    let subtitle = subtitle.into();
    let status = status.into();
    let keybinding = keybinding.filter(|keybinding| !keybinding.trim().is_empty());

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap_4()
        .h(style.height)
        .rounded(style.radius)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .px(style.padding_x)
        .hover(move |this| this.bg(style.hover_background))
        .on_click(on_click)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .overflow_hidden()
                .child(
                    div()
                        .text_sm()
                        .text_color(style.title)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(style.subtitle)
                        .truncate()
                        .child(subtitle),
                ),
        )
        .child(palette_item_trailing(
            status,
            keybinding,
            style.status,
            theme,
        ))
}

pub fn workbench_keybinding_badge(
    keybinding: impl Into<String>,
    theme: WorkbenchTheme,
) -> AnyElement {
    let keybinding = keybinding.into();
    if let Some(keystroke) = parse_keybinding_for_display(&keybinding) {
        Kbd::new(keystroke)
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .text_color(theme.text_muted)
            .into_any_element()
    } else {
        div()
            .rounded_sm()
            .border_1()
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .px_1()
            .py_0p5()
            .text_xs()
            .text_color(theme.text_muted)
            .child(keybinding)
            .into_any_element()
    }
}

fn palette_item_trailing(
    status: String,
    keybinding: Option<String>,
    status_color: gpui::Rgba,
    theme: WorkbenchTheme,
) -> Div {
    let mut trailing = div()
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .gap_2();

    if !status.is_empty() {
        trailing = trailing.child(
            div()
                .text_xs()
                .text_color(status_color)
                .truncate()
                .child(status),
        );
    }

    if let Some(keybinding) = keybinding {
        trailing = trailing.child(workbench_keybinding_badge(keybinding, theme));
    }

    trailing
}

pub fn workbench_settings_row(
    control_width: Pixels,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    let title = title.into();
    let description = description.into();
    let row_style = yttt_row_style(
        YtttRowKind::Settings,
        SelectableState::Inactive,
        true,
        theme,
    );

    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
        .min_h(row_style.height)
        .border_b(row_style.border_width)
        .border_color(row_style.border)
        .bg(row_style.background)
        .py(row_style.padding_y)
        .child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .min_w_0()
                .flex_1()
                .child(
                    div()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(row_style.title)
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(row_style.subtitle)
                        .child(description),
                ),
        )
        .child(
            div()
                .flex()
                .justify_end()
                .items_center()
                .w(control_width)
                .flex_none()
                .child(control),
        )
}

pub fn workbench_switch<H>(
    id: impl Into<ElementId>,
    checked: bool,
    theme: WorkbenchTheme,
    on_change: H,
) -> Div
where
    H: Fn(&bool, &mut Window, &mut App) + 'static,
{
    let style = yttt_switch_style(theme);
    let id = id.into();
    let next_checked = !checked;
    let track_background = if checked {
        style.active_background
    } else {
        style.inactive_background
    };
    let border = if checked {
        style.active_border
    } else {
        style.inactive_border
    };
    let thumb = if checked {
        style.active_thumb
    } else {
        style.inactive_thumb
    };

    div()
        .h(style.control_height)
        .flex()
        .items_center()
        .justify_end()
        .child(
            div()
                .id(id)
                .cursor_pointer()
                .flex()
                .items_center()
                .justify_center()
                .w(style.width)
                .h(style.height)
                .rounded_full()
                .border_2()
                .border_color(border)
                .hover(move |this| this.border_color(style.active_border))
                .on_click(move |_, window, cx| on_change(&next_checked, window, cx))
                .child(
                    div()
                        .flex()
                        .items_center()
                        .when(checked, |this| this.justify_end())
                        .when(!checked, |this| this.justify_start())
                        .w(style.track_width)
                        .h(style.track_height)
                        .px(style.track_padding)
                        .rounded_full()
                        .border_1()
                        .border_color(border)
                        .bg(track_background)
                        .child(
                            div()
                                .size(style.thumb_size)
                                .rounded_full()
                                .bg(thumb)
                                .shadow_xs(),
                        ),
                ),
        )
}

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

fn shortcut_keystroke(shortcut: &str) -> Keystroke {
    Keystroke::parse(shortcut).expect("workbench shortcut should be a valid GPUI keystroke")
}
