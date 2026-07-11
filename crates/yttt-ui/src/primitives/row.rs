use gpui::{Pixels, Rgba, px};

use crate::{SelectableState, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttRowKind {
    Palette,
    Settings,
    Sidebar,
    Tab,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttRowStyle {
    pub height: Pixels,
    pub padding_x: Pixels,
    pub padding_y: Pixels,
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
) -> YtttRowStyle {
    let (height, padding_x, padding_y, radius, border_width) = match kind {
        YtttRowKind::Palette => (px(54.0), px(12.0), px(0.0), px(6.0), px(1.0)),
        YtttRowKind::Settings => (px(72.0), px(0.0), px(12.0), px(0.0), px(1.0)),
        YtttRowKind::Sidebar => (px(28.0), px(8.0), px(0.0), px(6.0), px(0.0)),
        YtttRowKind::Tab => (px(32.0), px(8.0), px(0.0), px(0.0), px(1.0)),
    };

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
            YtttRowKind::Sidebar | YtttRowKind::Tab => theme.app_background,
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
            background: theme.surface,
            hover_background: theme.surface,
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
            background: theme.active_surface,
            hover_background: theme.active_surface,
            border: theme.active_surface,
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
            background: theme.app_background,
            hover_background: theme.hover_surface,
            border: theme.app_background,
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
            background: theme.app_background,
            hover_background: theme.hover_surface,
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
            hover_background: theme.hover_surface,
            border: theme.surface_elevated,
            title: theme.text_muted,
            subtitle: theme.text_subtle,
            status: theme.text_muted,
        },
    }
}
