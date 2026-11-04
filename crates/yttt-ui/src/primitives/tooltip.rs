use gpui::{AnyView, App, IntoElement, Styled as _, Window, px, rems};
use gpui_component::{text::Text, tooltip::Tooltip};

use crate::{
    style::{UiStyle, UiStyleId},
    surface::{UiElevation, YtttSurface as _},
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
        .m(px(0.))
        .ml(rems(0.5))
        .mt(rems(0.625))
        .bg(theme.surface_elevated)
        .text_color(theme.text)
        .border(ui_style.border.hairline)
        .border_color(theme.border)
        .rounded(ui_style.radius.compact)
        .yttt_elevation(UiElevation::Floating)
        .px(rems(0.5))
        .py(rems(0.25))
        .gap(rems(1.0))
        .text_size(ui_style.text.default)
}

/// A tooltip whose content `builder` draws itself, unstyled.
pub fn yttt_tooltip_view<E, F>(builder: F, window: &mut Window, cx: &mut App) -> AnyView
where
    E: IntoElement,
    F: Fn(&mut Window, &mut App) -> E + 'static,
{
    Tooltip::element(builder).build(window, cx)
}
