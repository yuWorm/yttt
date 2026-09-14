use gpui::{Styled as _, hsla, px, rems};
use gpui_component::{
    box_shadow,
    text::Text,
    tooltip::{Tooltip, TooltipAppearance},
};

use crate::{
    style::{UiStyle, UiStyleId},
    theme::WorkbenchTheme,
};

/// Builds a tooltip with Zed's elevation, density, and trigger offset.
///
/// The returned [`Tooltip`] keeps its native `.action`, `.key_binding`, and
/// `.build` APIs. The outer margins deliberately replace the vendor default so
/// the visible body is offset from its trigger like Zed's tooltip container.
pub fn yttt_tooltip(text: impl Into<Text>, theme: WorkbenchTheme, ui_style: UiStyle) -> Tooltip {
    if ui_style.id == UiStyleId::Rounded {
        return Tooltip::new(text);
    }

    Tooltip::new(text)
        .appearance(TooltipAppearance::new().key_binding_foreground(theme.text_muted))
        .m(px(0.))
        .ml(rems(0.5))
        .mt(rems(0.625))
        .bg(theme.surface_elevated)
        .text_color(theme.text)
        .border(ui_style.border.hairline)
        .border_color(theme.border)
        .rounded(ui_style.radius.compact)
        .shadow(vec![
            box_shadow(px(0.), px(2.), px(3.), px(0.), hsla(0., 0., 0., 0.12)),
            box_shadow(px(0.), px(1.), px(0.), px(0.), hsla(0., 0., 0., 0.06)),
        ])
        .px(rems(0.5))
        .py(rems(0.25))
        .gap(rems(1.0))
        .text_size(rems(0.875))
}
