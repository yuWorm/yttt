use gpui::{AnyElement, Div, Pixels, Rems, Rgba, div, prelude::*, rgba};

use crate::{SelectableState, style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttRowKind {
    Palette,
    Settings,
    Sidebar,
    Tab,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttRowStyle {
    pub height: Rems,
    pub padding_x: Rems,
    pub padding_y: Rems,
    pub radius: Pixels,
    pub border_width: Pixels,
    pub background: Rgba,
    pub hover_background: Rgba,
    pub border: Rgba,
    pub title: Rgba,
    pub subtitle: Rgba,
    pub status: Rgba,
}

pub fn yttt_row_style(
    kind: YtttRowKind,
    state: SelectableState,
    enabled: bool,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttRowStyle {
    let (height, padding_x, padding_y, radius, border_width) = match kind {
        YtttRowKind::Palette => (
            ui_style.rows.palette_height,
            ui_style.rows.palette_padding_x,
            ui_style.spacing.xxs,
            ui_style.rows.palette_radius,
            ui_style.rows.palette_border_width,
        ),
        YtttRowKind::Settings => (
            ui_style.rows.settings_height,
            ui_style.spacing.xxs,
            ui_style.rows.settings_padding_y,
            ui_style.rows.settings_radius,
            ui_style.rows.settings_border_width,
        ),
        YtttRowKind::Sidebar => (
            ui_style.rows.sidebar_height,
            ui_style.rows.sidebar_padding_x,
            ui_style.spacing.xxs,
            ui_style.rows.sidebar_radius,
            ui_style.rows.sidebar_border_width,
        ),
        YtttRowKind::Tab => (
            ui_style.rows.tab_height,
            ui_style.rows.tab_padding_x,
            ui_style.spacing.xxs,
            ui_style.rows.tab_radius,
            ui_style.rows.tab_border_width,
        ),
    };
    let transparent = rgba(0x00000000);

    if kind == YtttRowKind::Settings {
        return YtttRowStyle {
            height,
            padding_x,
            padding_y,
            radius,
            border_width,
            background: theme.ghost_element_background,
            hover_background: theme.ghost_element_background,
            border: transparent,
            title: theme.text,
            subtitle: theme.text_subtle,
            status: theme.text_muted,
        };
    }

    if !enabled {
        let background = match kind {
            YtttRowKind::Palette => theme.element_disabled,
            YtttRowKind::Settings | YtttRowKind::Sidebar | YtttRowKind::Tab => transparent,
        };

        return YtttRowStyle {
            height,
            padding_x,
            padding_y,
            radius,
            border_width,
            background,
            hover_background: background,
            border: background,
            title: theme.text_subtle,
            subtitle: theme.text_subtle,
            status: theme.text_subtle,
        };
    }

    match state {
        SelectableState::Active if kind == YtttRowKind::Tab => YtttRowStyle {
            height,
            padding_x,
            padding_y,
            radius,
            border_width,
            background: theme.tab_active_background,
            hover_background: theme.tab_active_background,
            border: theme.border_variant,
            title: theme.text,
            subtitle: theme.text_muted,
            status: theme.text_muted,
        },
        SelectableState::Active => YtttRowStyle {
            height,
            padding_x,
            padding_y,
            radius,
            border_width,
            background: theme.ghost_element_selected,
            hover_background: theme.ghost_element_selected,
            border: theme.ghost_element_selected,
            title: theme.text,
            subtitle: theme.text_muted,
            status: theme.text_muted,
        },
        SelectableState::Inactive if kind == YtttRowKind::Sidebar => YtttRowStyle {
            height,
            padding_x,
            padding_y,
            radius,
            border_width,
            background: transparent,
            hover_background: theme.ghost_element_hover,
            border: transparent,
            title: theme.text_muted,
            subtitle: theme.text_subtle,
            status: theme.text_muted,
        },
        SelectableState::Inactive if kind == YtttRowKind::Tab => YtttRowStyle {
            height,
            padding_x,
            padding_y,
            radius,
            border_width,
            background: theme.tab_inactive_background,
            hover_background: theme.ghost_element_hover,
            border: theme.border_variant,
            title: theme.text_muted,
            subtitle: theme.text_subtle,
            status: theme.text_muted,
        },
        SelectableState::Inactive => YtttRowStyle {
            height,
            padding_x,
            padding_y,
            radius,
            border_width,
            background: theme.element_background,
            hover_background: theme.element_hover,
            border: theme.element_background,
            title: theme.text_muted,
            subtitle: theme.text_subtle,
            status: theme.text_muted,
        },
    }
}

pub fn yttt_row(
    kind: YtttRowKind,
    state: SelectableState,
    enabled: bool,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let style = yttt_row_style(kind, state, enabled, theme, ui_style);
    div()
        .min_h(style.height)
        .px(style.padding_x)
        .py(style.padding_y)
        .rounded(style.radius)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .text_color(style.title)
        .when(enabled, |this| {
            this.hover(move |this| this.bg(style.hover_background))
        })
}

pub fn yttt_settings_row(
    control_width: Pixels,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    let title = title.into();
    let description = description.into();
    let style = yttt_row_style(
        YtttRowKind::Settings,
        SelectableState::Inactive,
        true,
        theme,
        ui_style,
    );

    yttt_row(
        YtttRowKind::Settings,
        SelectableState::Inactive,
        true,
        theme,
        ui_style,
    )
    .flex()
    .items_center()
    .justify_between()
    .gap(ui_style.spacing.xxl)
    .child(
        div()
            .flex()
            .flex_col()
            .gap(ui_style.spacing.xs)
            .min_w_0()
            .flex_1()
            .child(div().text_sm().text_color(style.title).child(title))
            .child(
                div()
                    .text_xs()
                    .text_color(style.subtitle)
                    .child(description),
            ),
    )
    .child(
        div()
            .flex()
            .justify_end()
            .items_center()
            .w(control_width)
            .flex_none()
            .child(control),
    )
}
