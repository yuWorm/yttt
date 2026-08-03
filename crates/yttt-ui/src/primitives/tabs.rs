use gpui::{Div, Pixels, Rems, Rgba, div, prelude::*, px};

use crate::{
    SelectableState,
    primitives::row::{YtttRowKind, yttt_row_style},
    style::UiStyle,
    theme::WorkbenchTheme,
};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttTabBarStyle {
    pub height: Rems,
    pub item_height: Rems,
    pub border_width: Pixels,
    pub min_width: Pixels,
    pub max_width: Pixels,
    pub close_slot_size: Rems,
    pub active_background: Rgba,
    pub inactive_background: Rgba,
    pub hover_background: Rgba,
}

pub fn yttt_tabbar_style(theme: WorkbenchTheme, ui_style: UiStyle) -> YtttTabBarStyle {
    YtttTabBarStyle {
        height: ui_style.rows.tab_height,
        item_height: ui_style.rows.tab_height,
        border_width: ui_style.rows.tab_border_width,
        min_width: px(128.0),
        max_width: px(220.0),
        close_slot_size: ui_style.icon_buttons.tab_close_size,
        active_background: theme.tab_active_background,
        inactive_background: theme.tab_inactive_background,
        hover_background: theme.ghost_element_hover,
    }
}

pub fn yttt_tab(state: SelectableState, theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    let row = yttt_row_style(YtttRowKind::Tab, state, true, theme, ui_style);
    let tabbar = yttt_tabbar_style(theme, ui_style);
    div()
        .h(row.height)
        .min_w(tabbar.min_width)
        .max_w(tabbar.max_width)
        .rounded(row.radius)
        .border_r(row.border_width)
        .border_color(row.border)
        .when(state == SelectableState::Active, |this| {
            this.pb(row.border_width)
        })
        .when(state == SelectableState::Inactive, |this| {
            this.border_b(row.border_width)
        })
        .bg(row.background)
        .px(row.padding_x)
        .text_color(row.title)
        .hover(move |this| this.bg(row.hover_background))
}
