use gpui::{Keystroke, Styled as _};
use gpui_component::kbd::Kbd;

use crate::{
    style::{UiStyle, UiStyleId},
    theme::WorkbenchTheme,
};

/// Shows a keystroke. Zed draws these as muted text without a key cap.
#[allow(clippy::disallowed_methods)]
pub fn yttt_kbd(keystroke: Keystroke, theme: WorkbenchTheme, ui_style: UiStyle) -> Kbd {
    let kbd = Kbd::new(keystroke);
    if ui_style.id == UiStyleId::Rounded {
        return kbd
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .text_color(theme.text_muted);
    }
    // `appearance(false)` renders bare text and drops every style, so keep the
    // chip's small type and strip its chrome instead.
    kbd.bg(gpui::transparent_black())
        .p_0()
        .min_w(gpui::px(0.))
        .text_color(theme.text_muted)
}
