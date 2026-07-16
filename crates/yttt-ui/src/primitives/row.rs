use gpui::{Pixels, Rems, Rgba, rgba};

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
            background: theme.surface,
            hover_background: theme.surface,
            border: theme.border,
            title: theme.text,
            subtitle: theme.text_subtle,
            status: theme.text_muted,
        };
    }

    if !enabled {
        let background = match kind {
            YtttRowKind::Palette | YtttRowKind::Settings => theme.surface_elevated,
            YtttRowKind::Sidebar | YtttRowKind::Tab => transparent,
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
            background: ui_style.active_background(theme),
            hover_background: ui_style.active_background(theme),
            border: theme.border,
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
            background: ui_style.active_background(theme),
            hover_background: ui_style.active_background(theme),
            border: ui_style.active_background(theme),
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
            hover_background: ui_style.hover_background(theme),
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
            background: transparent,
            hover_background: ui_style.hover_background(theme),
            border: theme.border,
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
            background: theme.surface_elevated,
            hover_background: ui_style.hover_background(theme),
            border: theme.surface_elevated,
            title: theme.text_muted,
            subtitle: theme.text_subtle,
            status: theme.text_muted,
        },
    }
}
