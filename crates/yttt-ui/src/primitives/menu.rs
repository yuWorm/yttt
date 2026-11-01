use gpui::{hsla, px, rems};
use gpui_component::{
    box_shadow,
    menu::{PopupMenu, PopupMenuStyle},
};

use crate::{
    style::{UiStyle, UiStyleId},
    theme::WorkbenchTheme,
};

/// Applies Zed's dense context-menu presentation without changing menu behavior.
///
/// The returned [`PopupMenu`] retains its item builders, focus restoration,
/// keyboard navigation, disabled-item handling, and event dispatch APIs.
pub fn yttt_popup_menu(menu: PopupMenu, theme: WorkbenchTheme, ui_style: UiStyle) -> PopupMenu {
    if ui_style.id == UiStyleId::Rounded {
        return menu;
    }

    menu.style(
        PopupMenuStyle::new()
            .background(theme.surface_elevated)
            .border_color(theme.border)
            .foreground(theme.text)
            .muted_foreground(theme.icon_muted)
            .hover(ui_style.hover_background(theme), theme.text)
            .selected(ui_style.active_background(theme), theme.text)
            .disabled_foreground(theme.text_disabled)
            .shadows([
                box_shadow(px(0.), px(2.), px(3.), px(0.), hsla(0., 0., 0., 0.12)),
                box_shadow(px(0.), px(1.), px(0.), px(0.), hsla(0., 0., 0., 0.06)),
            ])
            .radius(ui_style.radius.compact)
            .min_width(px(200.))
            .padding(rems(0.25))
            .gap(px(0.))
            .item_height(rems(1.5))
            .item_padding_x(rems(0.375))
            .item_gap(rems(0.375))
            .end_slot_gap(rems(1.0))
            .text_size(rems(0.875))
            .separator(rems(0.25), ui_style.border.hairline, theme.border)
            .reserve_icon_column(false),
    )
}
