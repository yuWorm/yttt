mod notification;
mod palette_item;

pub use notification::{
    notification_tone_for_toast, workbench_agent_notification, workbench_error_notification,
    workbench_inline_notification, workbench_status_notification,
};
pub use palette_item::{workbench_keybinding_badge, workbench_palette_item};

use gpui::{
    AnyElement, App, ClickEvent, Div, ElementId, FontWeight, InteractiveElement as _,
    ParentElement as _, SharedString, Stateful, StatefulInteractiveElement as _, Window, div,
    prelude::*,
};
use gpui_component::{Icon, IconName, kbd::Kbd, notification::Notification};

use crate::ui::{
    notifications::{ToastItem, ToastTone},
    primitives::{
        notification::{
            YtttNotificationTone, yttt_notification_style, yttt_notification_surface,
            yttt_toast_notification,
        },
        row::{YtttRowKind, yttt_row, yttt_row_style},
    },
    settings::keybinding_display::parse_keybinding_for_display,
    theme::{UiStyle, WorkbenchTheme},
};
pub use yttt_ui::SelectableState;

pub fn selectable_state_classes(state: SelectableState) -> &'static str {
    match state {
        SelectableState::Active => "selectable active",
        SelectableState::Inactive => "selectable inactive",
    }
}
