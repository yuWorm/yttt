use gpui::{Div, InteractiveElement as _, MouseButton};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardCapture {
    None,
    ScopeOnly,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayEventPolicy {
    pub keyboard: KeyboardCapture,
    pub mouse: bool,
    pub scroll: bool,
    pub dismiss_on_escape: bool,
    pub dismiss_on_click_outside: bool,
}

pub fn modal_overlay_event_policy() -> OverlayEventPolicy {
    OverlayEventPolicy {
        keyboard: KeyboardCapture::ScopeOnly,
        mouse: true,
        scroll: true,
        dismiss_on_escape: true,
        dismiss_on_click_outside: false,
    }
}

pub fn popover_overlay_event_policy() -> OverlayEventPolicy {
    OverlayEventPolicy {
        keyboard: KeyboardCapture::ScopeOnly,
        mouse: true,
        scroll: true,
        dismiss_on_escape: true,
        dismiss_on_click_outside: true,
    }
}

pub fn capture_overlay_input_with_policy(layer: Div, policy: OverlayEventPolicy) -> Div {
    let mut layer = layer;

    if policy.mouse {
        for button in MouseButton::all() {
            layer = layer
                .on_mouse_down(button, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(button, |_, _, cx| cx.stop_propagation());
        }
        layer = layer.on_mouse_move(|_, _, cx| cx.stop_propagation());
    }

    if policy.scroll {
        layer = layer.on_scroll_wheel(|_, _, cx| cx.stop_propagation());
    }

    layer
}

pub fn overlay_input_capture_policy() -> OverlayEventPolicy {
    modal_overlay_event_policy()
}

pub fn capture_overlay_input(layer: Div) -> Div {
    capture_overlay_input_with_policy(layer, overlay_input_capture_policy())
}
