pub mod menu;
pub mod picker;
pub mod primitives;
pub mod style;
pub mod surface;
pub mod theme;
pub mod tree;
pub mod window;

pub use gpui_component::{Icon, IconName};

/// Installs the component layer every `yttt-ui` primitive depends on.
#[allow(clippy::disallowed_methods)]
pub fn init(cx: &mut gpui::App) {
    gpui_component::init(cx);
    gpui_base::Root::register_plugin::<menu::MenuLayer>(cx, |_, _| menu::MenuLayer::default());
}

/// Re-registers the component keybindings after `cx.clear_key_bindings()`.
pub fn rebind_keybindings(cx: &mut gpui::App) {
    gpui_component::rebind_keybindings(cx);
}

/// Sets the locale of strings the component layer renders itself.
pub fn set_locale(locale: &str) {
    gpui_component::set_locale(locale);
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SelectableState {
    Active,
    Inactive,
}
