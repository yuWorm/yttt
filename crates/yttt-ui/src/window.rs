use gpui::{Entity, Render, TitlebarOptions, Window};
/// Opens dialogs and notifications on a window.
pub use gpui_component::WindowExt;
use gpui_component::{Root, TitleBar};

/// Wraps a window's view in the component root.
///
/// Every window goes through here so window-wide behavior has one home. The
/// root takes its rem size from the component theme, so callers must not set it.
#[allow(clippy::disallowed_methods)]
pub fn yttt_root<V: Render + 'static>(
    view: Entity<V>,
    window: &mut Window,
    cx: &mut gpui::Context<Root>,
) -> Root {
    Root::new(view, window, cx)
}

/// The window title bar, with platform window controls.
pub fn yttt_titlebar() -> TitleBar {
    TitleBar::new()
}

/// Window options that hand the title bar area to [`yttt_titlebar`].
pub fn yttt_titlebar_options() -> TitlebarOptions {
    TitleBar::title_bar_options()
}
