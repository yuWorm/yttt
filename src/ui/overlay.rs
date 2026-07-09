pub use crate::ui::interaction::overlay_policy::{
    KeyboardCapture, OverlayEventPolicy, capture_overlay_input_with_policy,
    modal_overlay_event_policy, popover_overlay_event_policy,
};

use gpui::Div;

pub fn overlay_input_capture_policy() -> OverlayEventPolicy {
    modal_overlay_event_policy()
}

pub fn capture_overlay_input(layer: Div) -> Div {
    capture_overlay_input_with_policy(layer, overlay_input_capture_policy())
}
