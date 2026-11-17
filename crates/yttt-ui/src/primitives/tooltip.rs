use gpui::{
    AnyView, App, InteractiveElement as _, IntoElement, ParentElement as _, Rems, Styled as _,
    Window, div, px, rems,
};
use gpui_component::{text::Text, tooltip::Tooltip};

use crate::{
    style::{UiStyle, UiStyleId},
    surface::{UiElevation, YtttSurface as _},
    theme::WorkbenchTheme,
};

/// Widest a text tooltip's body may grow before its text wraps.
pub const TOOLTIP_MAX_WIDTH: Rems = Rems(24.0);
/// Tallest a text tooltip's body may grow; anything longer is clipped.
pub const TOOLTIP_MAX_HEIGHT: Rems = Rems(12.0);

/// Builds a tooltip with Zed's elevation, density, and trigger offset.
///
/// The returned [`Tooltip`] keeps its native `.action`, `.key_binding`, and
/// `.build` APIs. The outer margins deliberately replace the vendor default so
/// the visible body is offset from its trigger like Zed's tooltip container.
///
/// Every style caps the body at [`TOOLTIP_MAX_WIDTH`] by [`TOOLTIP_MAX_HEIGHT`],
/// so a long label (an agent's whole task prompt, a deep path) wraps and is
/// clipped instead of stretching across the window.
pub fn yttt_tooltip(text: impl Into<Text>, theme: WorkbenchTheme, ui_style: UiStyle) -> Tooltip {
    let tooltip = bounded_text_tooltip(text.into());
    if ui_style.id == UiStyleId::Rounded {
        return tooltip;
    }

    tooltip
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

/// The vendor tooltip puts its text in a flex item without a minimum width,
/// and GPUI lays text out unwrapped when asked for its min-content width, so a
/// cap on the outer container alone would clip one endless line. Capping the
/// text's own box gives it a definite width to wrap at.
fn bounded_text_tooltip(text: Text) -> Tooltip {
    Tooltip::element(move |_, _| {
        div()
            .id("yttt-tooltip-body")
            .debug_selector(|| "yttt-tooltip-body".to_string())
            .max_w(TOOLTIP_MAX_WIDTH)
            .max_h(TOOLTIP_MAX_HEIGHT)
            .overflow_hidden()
            .child(text.clone())
    })
}

/// A tooltip whose content `builder` draws itself, unstyled.
pub fn yttt_tooltip_view<E, F>(builder: F, window: &mut Window, cx: &mut App) -> AnyView
where
    E: IntoElement,
    F: Fn(&mut Window, &mut App) -> E + 'static,
{
    Tooltip::element(builder).build(window, cx)
}
