#![allow(clippy::disallowed_methods)]

use gpui::{Entity, Pixels, Rems, Rgba, prelude::*};
use gpui_component::{
    Sizable as _,
    select::{Select, SelectDelegate, SelectItem, SelectState},
};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSelectStyle {
    pub width: Pixels,
    pub height: Rems,
    pub radius: Pixels,
    pub menu_width: Pixels,
    pub background: Rgba,
    pub hover_background: Rgba,
    pub active_background: Rgba,
    pub border: Rgba,
    pub focused_border: Rgba,
    pub text: Rgba,
}

pub fn yttt_select_style(theme: WorkbenchTheme, ui_style: UiStyle) -> YtttSelectStyle {
    YtttSelectStyle {
        width: ui_style.controls.settings_control_width,
        height: ui_style.controls.settings_height,
        radius: ui_style.radius.control,
        menu_width: ui_style.controls.select_menu_width,
        background: theme.element_background,
        hover_background: theme.element_hover,
        active_background: theme.element_active,
        border: theme.border_variant,
        focused_border: theme.border_focused,
        text: theme.text,
    }
}

pub fn yttt_select<D>(
    state: &Entity<SelectState<D>>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Select<D>
where
    D: SelectDelegate + 'static,
    <D::Item as SelectItem>::Value: PartialEq + Clone,
{
    let style = yttt_select_style(theme, ui_style);
    Select::new(state)
        .small()
        .appearance(true)
        .w(style.width)
        .h(style.height)
        .menu_width(style.menu_width)
        .rounded(style.radius)
        .border_color(style.border)
        .bg(style.background)
        .text_color(style.text)
}
