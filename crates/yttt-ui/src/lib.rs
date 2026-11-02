pub mod menu;
pub mod primitives;
pub mod style;
pub mod surface;
pub mod theme;
pub mod window;

pub use gpui_component::{Icon, IconName};

/// Installs the component layer every `yttt-ui` primitive depends on.
#[allow(clippy::disallowed_methods)]
pub fn init(cx: &mut gpui::App) {
    gpui_component::init(cx);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableState {
    Active,
    Inactive,
}
