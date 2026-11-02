use std::{cell::Cell, rc::Rc};

use gpui::{
    Action, Anchor, AnyElement, App, Bounds, ClickEvent, Context, Div, FocusHandle, IntoElement,
    MouseButton, MouseDownEvent, Pixels, Point, Render, SharedString, Window, canvas, deferred,
    div, point, prelude::*, px, rems,
};
use gpui_base::{
    Button, Positioner, Root, RootPlugin,
    actions::{Cancel, Confirm, SelectDown, SelectUp},
};
use gpui_component::{
    Icon, IconName, Sizable as _,
    menu::{PopupMenu, PopupMenuItem, PopupMenuStyle},
};

use crate::{
    style::{UiStyle, UiStyleId},
    surface::{UiElevation, YtttSurface as _, floating_shadows},
    theme::WorkbenchTheme,
};

type MenuHandler = Rc<dyn Fn(&ClickEvent, &mut Window, &mut App)>;
type MenuBuilder = dyn Fn(YtttMenu, &mut Window, &mut App) -> YtttMenu;

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

    #[allow(clippy::disallowed_methods)]
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

/// The key context gpui-component binds menu navigation keys to.
const MENU_KEY_CONTEXT: &str = "PopupMenu";
const MENU_MIN_WIDTH: Pixels = px(200.);
const MENU_WINDOW_MARGIN: Pixels = px(8.);
/// Deferred paint priority that keeps a menu above dialogs.
const MENU_PRIORITY: usize = 100;

/// Measured from Zed 1.26.2's project panel menu.
fn menu_row_height() -> gpui::Rems {
    rems(1.40625)
}

fn menu_row(item: &YtttMenuItem, theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    let Some(label) = item.label.clone() else {
        return div()
            .my(rems(0.375))
            .h(ui_style.border.hairline)
            .bg(theme.border);
    };
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(rems(1.0))
        .h(menu_row_height())
        .px(rems(0.375))
        .rounded(ui_style.radius.compact)
        .text_color(if item.disabled {
            theme.text_disabled
        } else {
            theme.text
        })
        .child(label)
        .when(item.checked, |row| {
            row.child(Icon::new(IconName::Check).xsmall())
        })
}

fn menu_surface(theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    div()
        .min_w(MENU_MIN_WIDTH)
        .p(rems(0.25))
        .flex()
        .flex_col()
        .bg(theme.surface_elevated)
        .border(ui_style.border.hairline)
        .border_color(theme.border)
        .rounded(ui_style.radius.compact)
        .text_size(ui_style.text.default)
        .yttt_elevation(UiElevation::Floating)
}

struct OpenMenu {
    position: Point<Pixels>,
    action_context: Option<FocusHandle>,
    items: Vec<YtttMenuItem>,
    highlighted: Option<usize>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    focus_handle: FocusHandle,
    previous_focus: Option<FocusHandle>,
}

/// Index of the next row a keyboard move lands on, wrapping past either end.
fn next_enabled(items: &[YtttMenuItem], from: Option<usize>, forward: bool) -> Option<usize> {
    let count = items.len();
    let start = from.unwrap_or(if forward { count.wrapping_sub(1) } else { 0 });
    (1..=count)
        .map(|step| {
            if forward {
                (start + step) % count
            } else {
                (start + count - step % count) % count
            }
        })
        .find(|&index| !items[index].is_separator() && !items[index].disabled)
}

/// The window's single open menu. Only one menu can be open at a time, so it
/// lives on the root rather than on every trigger.
#[derive(Default)]
pub struct MenuLayer {
    open: Option<OpenMenu>,
    requested: Option<MenuRequest>,
}

struct MenuRequest {
    position: Point<Pixels>,
    appearance: MenuAppearance,
    build: Rc<MenuBuilder>,
}

impl MenuLayer {
    fn dismiss(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(open) = self.open.take() {
            if let Some(previous) = open.previous_focus {
                previous.focus(window, cx);
            }
            cx.notify();
        }
    }

    fn move_highlight(&mut self, forward: bool, cx: &mut Context<Self>) {
        if let Some(open) = &mut self.open {
            open.highlighted = next_enabled(&open.items, open.highlighted, forward);
            cx.notify();
        }
    }

    fn activate(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut open) = self.open.take() else {
            return;
        };
        cx.notify();
        if let Some(previous) = open.previous_focus.take() {
            previous.focus(window, cx);
        }
        let item = open.items.swap_remove(index);
        if item.disabled {
            return;
        }
        match item.target {
            MenuTarget::None => {}
            MenuTarget::Handler(handler) => handler(&ClickEvent::default(), window, cx),
            MenuTarget::Action(action) => {
                if let Some(context) = open.action_context {
                    context.focus(window, cx);
                }
                window.dispatch_action(action, cx);
            }
        }
    }
}

impl RootPlugin for MenuLayer {}

impl Render for MenuLayer {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(open) = &self.open else {
            return div().into_any_element();
        };
        let (theme, ui_style) = (open.theme, open.ui_style);
        let hover = ui_style.hover_background(theme);
        let selected = ui_style.active_background(theme);
        let rows = open.items.iter().enumerate().map(|(index, item)| {
            let enabled = !item.is_separator() && !item.disabled;
            menu_row(item, theme, ui_style)
                .id(index)
                .when(open.highlighted == Some(index) && enabled, |row| {
                    row.bg(selected)
                })
                .when(enabled, |row| {
                    row.hover(move |row| row.bg(hover)).on_click(cx.listener(
                        move |layer, _, window, cx| {
                            layer.activate(index, window, cx);
                        },
                    ))
                })
        });
        deferred(
            Positioner::corner(Anchor::TopLeft, open.position)
                .margin(MENU_WINDOW_MARGIN)
                .occlude()
                .child(
                    menu_surface(theme, ui_style)
                        .id("yttt-menu")
                        .debug_selector(|| "yttt-menu".to_string())
                        .key_context(MENU_KEY_CONTEXT)
                        .track_focus(&open.focus_handle)
                        .on_action(cx.listener(|layer, _: &SelectUp, _, cx| {
                            layer.move_highlight(false, cx);
                        }))
                        .on_action(cx.listener(|layer, _: &SelectDown, _, cx| {
                            layer.move_highlight(true, cx);
                        }))
                        .on_action(cx.listener(|layer, _: &Cancel, window, cx| {
                            layer.dismiss(window, cx);
                        }))
                        .on_action(cx.listener(|layer, _: &Confirm, window, cx| {
                            if let Some(index) =
                                layer.open.as_ref().and_then(|open| open.highlighted)
                            {
                                layer.activate(index, window, cx);
                            }
                        }))
                        .on_mouse_down_out(cx.listener(|layer, _, window, cx| {
                            layer.dismiss(window, cx);
                        }))
                        .children(rows),
                ),
        )
        .with_priority(MENU_PRIORITY)
        .into_any_element()
    }
}

fn menu_layer(window: &Window, cx: &App) -> Option<gpui::Entity<MenuLayer>> {
    window
        .root::<Root>()
        .flatten()?
        .read(cx)
        .plugin::<MenuLayer>()
}

/// Whether this window has an open menu. The menu renders outside the
/// workbench's tree, so focus recovery must leave it alone while it is open.
pub fn menu_is_open(window: &Window, cx: &App) -> bool {
    menu_layer(window, cx).is_some_and(|layer| layer.read(cx).open.is_some())
}

fn open_menu(
    position: Point<Pixels>,
    appearance: MenuAppearance,
    build: &MenuBuilder,
    window: &mut Window,
    cx: &mut App,
) {
    let menu = build(YtttMenu::default(), window, cx);
    if menu.items.is_empty() {
        return;
    }
    let Some(layer) = menu_layer(window, cx) else {
        return;
    };
    let (theme, ui_style) = appearance(cx);
    let focus_handle = cx.focus_handle();
    let previous_focus = window.focused(cx);
    focus_handle.focus(window, cx);
    layer.update(cx, |layer, cx| {
        layer.open = Some(OpenMenu {
            position,
            action_context: menu.action_context,
            items: menu.items,
            highlighted: None,
            theme,
            ui_style,
            focus_handle,
            previous_focus,
        });
        cx.notify();
    });
}

/// Opens a context menu once the mouse event has finished. Other listeners on
/// the element still run, and nested regions resolve to the innermost one,
/// which bubbles first.
fn request_menu(request: MenuRequest, window: &mut Window, cx: &mut App) {
    let Some(layer) = menu_layer(window, cx) else {
        return;
    };
    let first = layer.update(cx, |layer, _| {
        let first = layer.requested.is_none();
        layer.requested.get_or_insert(request);
        first
    });
    if first {
        window.defer(cx, move |window, cx| {
            if let Some(request) = layer.update(cx, |layer, _| layer.requested.take()) {
                open_menu(
                    request.position,
                    request.appearance,
                    &*request.build,
                    window,
                    cx,
                );
            }
        });
    }
}

pub trait YtttContextMenu: InteractiveElement + Sized {
    fn yttt_context_menu(
        self,
        appearance: MenuAppearance,
        build: impl Fn(YtttMenu, &mut Window, &mut App) -> YtttMenu + 'static,
    ) -> Self {
        let build: Rc<MenuBuilder> = Rc::new(build);
        self.on_mouse_down(
            MouseButton::Right,
            move |event: &MouseDownEvent, window, cx| {
                let request = MenuRequest {
                    position: event.position,
                    appearance,
                    build: build.clone(),
                };
                request_menu(request, window, cx);
            },
        )
    }
}

impl<T: InteractiveElement + Sized> YtttContextMenu for T {}

pub trait YtttDropdownMenu: Sized {
    fn yttt_dropdown_menu(
        self,
        appearance: MenuAppearance,
        build: impl Fn(YtttMenu, &mut Window, &mut App) -> YtttMenu + 'static,
    ) -> Self;
}

impl YtttDropdownMenu for Button {
    fn yttt_dropdown_menu(
        self,
        appearance: MenuAppearance,
        build: impl Fn(YtttMenu, &mut Window, &mut App) -> YtttMenu + 'static,
    ) -> Self {
        let bounds = Rc::new(Cell::new(Bounds::<Pixels>::default()));
        let trigger_bounds = bounds.clone();
        self.relative()
            .child(
                canvas(
                    move |bounds, _, _| trigger_bounds.set(bounds),
                    |_, _, _, _| {},
                )
                .absolute()
                .top_0()
                .left_0()
                .size_full(),
            )
            .on_click(move |_, window, cx| {
                let trigger = bounds.get();
                let position = point(trigger.left(), trigger.bottom() + px(4.));
                open_menu(position, appearance, &build, window, cx);
            })
    }
}

/// Renders a menu in place, for galleries and previews.
pub fn yttt_menu_preview(menu: &YtttMenu, theme: WorkbenchTheme, ui_style: UiStyle) -> AnyElement {
    menu_surface(theme, ui_style)
        .children(
            menu.items
                .iter()
                .map(|item| menu_row(item, theme, ui_style)),
        )
        .into_any_element()
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
            .shadows(floating_shadows())
            .radius(ui_style.radius.compact)
            .min_width(MENU_MIN_WIDTH)
            .padding(rems(0.25))
            .gap(px(0.))
            .item_height(menu_row_height())
            .item_padding_x(rems(0.375))
            .item_gap(rems(0.375))
            .end_slot_gap(rems(1.0))
            .text_size(ui_style.text.default)
            .separator(rems(0.375), ui_style.border.hairline, theme.border)
            .reserve_icon_column(false),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyboard_moves_skip_separators_and_disabled_rows() {
        let items = [
            YtttMenuItem::new("Open"),
            YtttMenuItem::separator(),
            YtttMenuItem::new("Paste").disabled(true),
            YtttMenuItem::new("Delete"),
        ];

        assert_eq!(next_enabled(&items, None, true), Some(0));
        assert_eq!(next_enabled(&items, Some(0), true), Some(3));
        assert_eq!(next_enabled(&items, Some(3), true), Some(0));
        assert_eq!(next_enabled(&items, None, false), Some(3));
        assert_eq!(next_enabled(&items, Some(3), false), Some(0));
    }

    #[test]
    fn keyboard_moves_find_nothing_without_enabled_rows() {
        let items = [
            YtttMenuItem::separator(),
            YtttMenuItem::new("Off").disabled(true),
        ];

        assert_eq!(next_enabled(&items, None, true), None);
    }

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
