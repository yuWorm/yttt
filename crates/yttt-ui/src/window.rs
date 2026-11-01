use gpui::{Entity, Render, Window};
use gpui_component::Root;

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
