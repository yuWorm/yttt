use gpui::{Pixels, Rems, Rgba, px, rems, rgba};

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
) -> YtttRowStyle {
    let (height, padding_x, padding_y, radius, border_width) = match kind {
        YtttRowKind::Palette => (rems(3.375), rems(0.75), rems(0.0), px(6.0), px(1.0)),
        YtttRowKind::Settings => (rems(4.5), rems(0.0), rems(0.75), px(0.0), px(1.0)),
        YtttRowKind::Sidebar => (rems(1.75), rems(0.5), rems(0.0), px(6.0), px(0.0)),
        YtttRowKind::Tab => (rems(2.0), rems(0.5), rems(0.0), px(0.0), px(1.0)),
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
            background: theme.active_surface,
            hover_background: theme.active_surface,
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
            background: transparent,
            hover_background: theme.hover_surface,
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
