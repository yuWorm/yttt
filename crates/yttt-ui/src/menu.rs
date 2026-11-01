use std::rc::Rc;

use gpui::{
    Action, App, ClickEvent, Context, Entity, FocusHandle, IntoElement, SharedString, Window, hsla,
    px, rems,
};
use gpui_component::{
    box_shadow,
    button::Button,
    menu::{ContextMenu, ContextMenuExt, DropdownMenu, PopupMenu, PopupMenuItem, PopupMenuStyle},
};

use crate::{
    style::{UiStyle, UiStyleId},
    theme::WorkbenchTheme,
};

type MenuHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;

/// Reads the theme and style a menu opens with.
pub type MenuAppearance = fn(&App) -> (WorkbenchTheme, UiStyle);

enum MenuTarget {
    None,
    Action(Box<dyn Action>),
    Handler(MenuHandler),
}

/// One menu row, described as data so menus can be tested without a window.
pub struct YtttMenuItem {
    label: Option<SharedString>,
    disabled: bool,
    checked: bool,
    target: MenuTarget,
}

impl YtttMenuItem {
    pub fn new(label: impl Into<SharedString>) -> Self {
        Self {
            label: Some(label.into()),
            disabled: false,
            checked: false,
            target: MenuTarget::None,
        }
    }

    pub fn separator() -> Self {
        Self {
            label: None,
            disabled: false,
            checked: false,
            target: MenuTarget::None,
        }
    }

    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    pub fn checked(mut self, checked: bool) -> Self {
        self.checked = checked;
        self
    }

    pub fn action(mut self, action: Box<dyn Action>) -> Self {
        self.target = MenuTarget::Action(action);
        self
    }

    pub fn on_click(
        mut self,
        handler: impl Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    ) -> Self {
        self.target = MenuTarget::Handler(Rc::new(handler));
        self
    }

    pub fn label(&self) -> Option<&SharedString> {
        self.label.as_ref()
    }

    pub fn is_separator(&self) -> bool {
        self.label.is_none()
    }

    pub fn is_disabled(&self) -> bool {
        self.disabled
    }

    fn into_popup_item(self) -> PopupMenuItem {
        let Some(label) = self.label else {
            return PopupMenuItem::separator();
        };
        let item = PopupMenuItem::new(label)
            .disabled(self.disabled)
            .checked(self.checked);
        match self.target {
            MenuTarget::None => item,
            MenuTarget::Action(action) => item.action(action),
            MenuTarget::Handler(handler) => {
                item.on_click(move |event, window, cx| handler(event, window, cx))
            }
        }
    }
}

/// The contents of a context or dropdown menu.
#[derive(Default)]
pub struct YtttMenu {
    action_context: Option<FocusHandle>,
    items: Vec<YtttMenuItem>,
}

impl YtttMenu {
    pub fn item(mut self, item: YtttMenuItem) -> Self {
        self.items.push(item);
        self
    }

    /// Dispatches `action` items to this focus handle instead of the menu.
    pub fn action_context(mut self, context: FocusHandle) -> Self {
        self.action_context = Some(context);
        self
    }

    pub fn items(&self) -> &[YtttMenuItem] {
        &self.items
    }

    /// Fills a popup menu owned by another component, such as the file tree.
    pub fn into_popup_menu(
        self,
        menu: PopupMenu,
        theme: WorkbenchTheme,
        ui_style: UiStyle,
    ) -> PopupMenu {
        let menu = match self.action_context {
            Some(context) => menu.action_context(context),
            None => menu,
        };
        self.items
            .into_iter()
            .fold(styled_popup_menu(menu, theme, ui_style), |menu, item| {
                menu.item(item.into_popup_item())
            })
    }
}

fn build_popup_menu(
    menu: PopupMenu,
    appearance: MenuAppearance,
    build: &dyn Fn(YtttMenu, &mut Window, &mut App) -> YtttMenu,
    window: &mut Window,
    cx: &mut Context<PopupMenu>,
) -> PopupMenu {
    let (theme, ui_style) = appearance(cx);
    build(YtttMenu::default(), window, cx).into_popup_menu(menu, theme, ui_style)
}

pub trait YtttContextMenu: ContextMenuExt + Sized {
    #[track_caller]
    fn yttt_context_menu(
        self,
        appearance: MenuAppearance,
        build: impl Fn(YtttMenu, &mut Window, &mut App) -> YtttMenu + 'static,
    ) -> ContextMenu<Self> {
        self.context_menu(move |menu, window, cx| {
            build_popup_menu(menu, appearance, &build, window, cx)
        })
    }
}

impl<T: ContextMenuExt + Sized> YtttContextMenu for T {}

pub trait YtttDropdownMenu: Sized {
    fn yttt_dropdown_menu(
        self,
        appearance: MenuAppearance,
        build: impl Fn(YtttMenu, &mut Window, &mut App) -> YtttMenu + 'static,
    ) -> impl IntoElement;
}

impl YtttDropdownMenu for Button {
    fn yttt_dropdown_menu(
        self,
        appearance: MenuAppearance,
        build: impl Fn(YtttMenu, &mut Window, &mut App) -> YtttMenu + 'static,
    ) -> impl IntoElement {
        self.dropdown_menu(move |menu, window, cx| {
            build_popup_menu(menu, appearance, &build, window, cx)
        })
    }
}

/// Builds a menu to render in place, for galleries and previews.
pub fn yttt_menu_preview(
    menu: YtttMenu,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    window: &mut Window,
    cx: &mut App,
) -> Entity<PopupMenu> {
    let menu = std::cell::Cell::new(Some(menu));
    PopupMenu::build(window, cx, move |popup, _, _| {
        menu.take()
            .unwrap_or_default()
            .into_popup_menu(popup, theme, ui_style)
    })
}

/// Applies Zed's dense context-menu presentation without changing menu behavior.
fn styled_popup_menu(menu: PopupMenu, theme: WorkbenchTheme, ui_style: UiStyle) -> PopupMenu {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn items_keep_their_order_and_state() {
        let menu = YtttMenu::default()
            .item(YtttMenuItem::new("Rename"))
            .item(YtttMenuItem::separator())
            .item(YtttMenuItem::new("Delete").disabled(true));

        let labels: Vec<_> = menu.items().iter().map(YtttMenuItem::label).collect();
        assert_eq!(
            labels,
            [
                Some(&SharedString::from("Rename")),
                None,
                Some(&SharedString::from("Delete")),
            ]
        );
        assert!(menu.items()[1].is_separator());
        assert!(menu.items()[2].is_disabled());
        assert!(!menu.items()[0].is_disabled());
    }
}
