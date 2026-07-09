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
    switch::Switch,
};

use crate::ui::{
    palette_surface::palette_row_style,
    primitives::{
        icon_button::{YtttIconButtonKind, yttt_icon_button_style},
        notification::{YtttNotificationTone, yttt_notification_style},
        row::{YtttRowKind, yttt_row_style},
        switch::yttt_switch_style,
    },
    theme::WorkbenchTheme,
    toast::{ToastItem, ToastTone},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableState {
    Active,
    Inactive,
}

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
        .child(
            div()
                .flex_none()
                .text_xs()
                .text_color(style.status)
                .child(status),
        )
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

    div()
        .h(style.control_height)
        .flex()
        .items_center()
        .justify_end()
        .child(Switch::new(id).checked(checked).on_click(on_change))
}

pub fn notification_tone_for_toast(tone: ToastTone) -> YtttNotificationTone {
    match tone {
        ToastTone::Success => YtttNotificationTone::Success,
        ToastTone::Error => YtttNotificationTone::Error,
    }
}

pub fn workbench_agent_notification(
    item: ToastItem,
    action_label: impl Into<SharedString>,
    theme: WorkbenchTheme,
) -> Notification {
    let tone = notification_tone_for_toast(item.tone);
    let style = yttt_notification_style(tone, theme);
    let icon = match tone {
        YtttNotificationTone::Success => IconName::CircleCheck,
        YtttNotificationTone::Error => IconName::CircleX,
    };
    let title = SharedString::from(item.title);
    let context = SharedString::from(item.context);
    let action_label = action_label.into();

    Notification::new()
        .w(style.width)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .rounded(style.radius)
        .px(style.padding_x)
        .py(style.padding_y)
        .content(move |_, _, _| {
            div()
                .flex()
                .items_center()
                .gap(style.gap)
                .min_h(style.min_height)
                .w_full()
                .child(
                    Icon::new(icon.clone())
                        .size(style.icon_size)
                        .text_color(style.tone),
                )
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
                                .child(title.clone()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(style.context)
                                .truncate()
                                .child(context.clone()),
                        ),
                )
                .child(
                    div()
                        .flex_none()
                        .rounded(px(5.0))
                        .bg(style.action_background)
                        .px(px(6.0))
                        .py(px(2.0))
                        .text_xs()
                        .text_color(style.action)
                        .child(action_label.clone()),
                )
                .into_any_element()
        })
}

fn shortcut_keystroke(shortcut: &str) -> Keystroke {
    Keystroke::parse(shortcut).expect("workbench shortcut should be a valid GPUI keystroke")
}
