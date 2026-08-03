use gpui::{Div, IntoElement, MouseButton, Pixels, Rgba, div, prelude::*, px, relative};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttPanelKind {
    Palette,
    Settings,
    Dialog,
    Editor,
    Fullscreen,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum YtttOverlayPlacement {
    Top,
    #[default]
    Center,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyboardCapture {
    None,
    ScopeOnly,
    All,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OverlayEventPolicy {
    pub keyboard: KeyboardCapture,
    pub mouse: bool,
    pub scroll: bool,
    pub dismiss_on_escape: bool,
    pub dismiss_on_click_outside: bool,
}

pub const fn modal_overlay_event_policy() -> OverlayEventPolicy {
    OverlayEventPolicy {
        keyboard: KeyboardCapture::ScopeOnly,
        mouse: true,
        scroll: true,
        dismiss_on_escape: true,
        dismiss_on_click_outside: false,
    }
}

pub const fn popover_overlay_event_policy() -> OverlayEventPolicy {
    OverlayEventPolicy {
        keyboard: KeyboardCapture::ScopeOnly,
        mouse: true,
        scroll: true,
        dismiss_on_escape: true,
        dismiss_on_click_outside: true,
    }
}

pub fn capture_overlay_input_with_policy(mut layer: Div, policy: OverlayEventPolicy) -> Div {
    if policy.mouse {
        for button in MouseButton::all() {
            layer = layer
                .on_mouse_down(button, |_, _, cx| cx.stop_propagation())
                .on_mouse_up(button, |_, _, cx| cx.stop_propagation());
        }
        layer = layer.on_mouse_move(|_, _, cx| cx.stop_propagation());
    }
    if policy.scroll {
        layer = layer.on_scroll_wheel(|_, _, cx| cx.stop_propagation());
    }
    layer
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttPanelStyle {
    pub width: Pixels,
    pub height: Option<Pixels>,
    pub max_width: Pixels,
    pub max_height: Pixels,
    pub body_max_height: Pixels,
    pub radius: Pixels,
    pub padding: Pixels,
    pub overlay: Rgba,
    pub border_width: Pixels,
    pub background: Rgba,
    pub border: Rgba,
    pub shadow: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSettingsLayout {
    pub sidebar_width: Pixels,
    pub control_width: Pixels,
    pub compact_control_width: Pixels,
    pub control_height: gpui::Rems,
    pub search_height: gpui::Rems,
    pub ui_style: UiStyle,
}

pub fn yttt_settings_layout(ui_style: UiStyle) -> YtttSettingsLayout {
    YtttSettingsLayout {
        sidebar_width: ui_style.settings.sidebar_width,
        control_width: ui_style.controls.settings_control_width,
        compact_control_width: ui_style.controls.settings_compact_control_width,
        control_height: ui_style.controls.settings_height,
        search_height: ui_style.controls.search_height,
        ui_style,
    }
}

pub fn yttt_panel_style(
    kind: YtttPanelKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttPanelStyle {
    let overlay = match kind {
        YtttPanelKind::Dialog => ui_style.panels.dialog_overlay,
        YtttPanelKind::Editor => ui_style.panels.editor_overlay,
        YtttPanelKind::Fullscreen => ui_style.panels.fullscreen_overlay,
        YtttPanelKind::Palette | YtttPanelKind::Settings => ui_style.panels.panel_overlay,
    };
    let (width, height, max_width, max_height, body_max_height, padding) = match kind {
        YtttPanelKind::Palette => (px(760.0), None, px(900.0), px(480.0), px(376.0), px(0.0)),
        YtttPanelKind::Settings => (
            px(900.0),
            Some(px(560.0)),
            px(940.0),
            px(600.0),
            px(600.0),
            px(0.0),
        ),
        YtttPanelKind::Dialog => (
            px(420.0),
            None,
            px(420.0),
            px(420.0),
            px(420.0),
            ui_style.panels.dialog_padding,
        ),
        YtttPanelKind::Editor => (
            px(860.0),
            Some(px(560.0)),
            px(960.0),
            px(620.0),
            px(620.0),
            px(0.0),
        ),
        YtttPanelKind::Fullscreen => (px(1280.0), None, px(1280.0), px(900.0), px(900.0), px(0.0)),
    };

    YtttPanelStyle {
        width,
        height,
        max_width,
        max_height,
        radius: ui_style.panels.radius,
        body_max_height,
        padding,
        overlay,
        background: if kind == YtttPanelKind::Settings {
            theme.editor_background.alpha(1.0)
        } else {
            theme.surface.alpha(1.0)
        },
        border_width: ui_style.border.hairline,
        border: theme.border_variant,
        shadow: ui_style.panels.shadow,
    }
}

pub fn yttt_panel(kind: YtttPanelKind, theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    let style = yttt_panel_style(kind, theme, ui_style);
    div()
        .flex()
        .flex_col()
        .w(style.width)
        .max_w(style.max_width)
        .max_h(style.max_height)
        .when_some(style.height, |this, height| this.h(height))
        .rounded(style.radius)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .text_color(theme.text)
        .p(style.padding)
        .when(style.shadow, |this| this.shadow_lg())
}

pub fn yttt_fullscreen_panel(theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    let style = yttt_panel_style(YtttPanelKind::Fullscreen, theme, ui_style);
    div()
        .flex()
        .flex_col()
        .w(relative(0.96))
        .h(relative(0.90))
        .min_h_0()
        .rounded(style.radius)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .text_color(theme.text)
        .when(style.shadow, |this| this.shadow_lg())
}

pub fn yttt_panel_overlay(
    content: impl IntoElement,
    kind: YtttPanelKind,
    placement: YtttOverlayPlacement,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let style = yttt_panel_style(kind, theme, ui_style);
    let layer = div()
        .absolute()
        .inset_0()
        .flex()
        .justify_center()
        .bg(style.overlay)
        .text_color(theme.text)
        .when(placement == YtttOverlayPlacement::Top, |this| {
            this.items_start().pt(ui_style.spacing.overlay_top)
        })
        .when(placement == YtttOverlayPlacement::Center, |this| {
            this.items_center()
        })
        .child(content);
    capture_overlay_input_with_policy(layer, modal_overlay_event_policy())
}
