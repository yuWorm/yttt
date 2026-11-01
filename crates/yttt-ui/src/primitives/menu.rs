use gpui_component::menu::PopupMenu;

use crate::{style::UiStyle, theme::WorkbenchTheme};

// SPIKE: PopupMenuStyle is not replayed yet, so menus use the upstream look.
pub fn yttt_popup_menu(menu: PopupMenu, _theme: WorkbenchTheme, _ui_style: UiStyle) -> PopupMenu {
    menu
}
