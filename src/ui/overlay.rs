use gpui::{Div, InteractiveElement as _, MouseButton};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayInputCapturePolicy {
    pub keyboard: bool,
    pub mouse: bool,
    pub scroll: bool,
}

pub fn overlay_input_capture_policy() -> OverlayInputCapturePolicy {
    OverlayInputCapturePolicy {
        keyboard: true,
        mouse: true,
        scroll: true,
    }
}

pub fn capture_overlay_input(layer: Div) -> Div {
    let policy = overlay_input_capture_policy();
    let mut layer = layer;

    if policy.keyboard {
        layer = layer
            .on_key_down(|_, _, cx| cx.stop_propagation())
            .on_key_up(|_, _, cx| cx.stop_propagation());
    }

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
