mod action;
mod notification;
mod palette_item;
mod settings;

pub use action::{workbench_action_button, workbench_icon_button};
pub use notification::{
    notification_tone_for_toast, workbench_agent_notification, workbench_error_notification,
    workbench_inline_notification, workbench_status_notification,
};
pub use palette_item::{workbench_keybinding_badge, workbench_palette_item};
pub use settings::{workbench_settings_row, workbench_switch};

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
