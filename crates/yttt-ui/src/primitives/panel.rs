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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttSettingsWidthClass {
    Compact,
    Regular,
    Wide,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttSettingsHeightClass {
    Compact,
    Regular,
    Tall,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttSettingsLayout {
    pub panel_width: Pixels,
    pub panel_height: Pixels,
    pub sidebar_width: Pixels,
    pub control_width: Pixels,
    pub compact_control_width: Pixels,
    pub control_height: gpui::Rems,
    pub search_height: gpui::Rems,
    pub content_padding_x: gpui::Rems,
    pub content_padding_y: gpui::Rems,
    pub section_gap: gpui::Rems,
    pub width_class: YtttSettingsWidthClass,
    pub height_class: YtttSettingsHeightClass,
    pub stack_rows: bool,
    pub ui_style: UiStyle,
}

pub fn yttt_settings_layout(ui_style: UiStyle, viewport: gpui::Size<Pixels>) -> YtttSettingsLayout {
    let viewport_width = f32::from(viewport.width);
    let viewport_height = f32::from(viewport.height);
    let panel_width_value = (viewport_width * 0.92)
        .clamp(860.0, 1_240.0)
        .min((viewport_width - 32.0).max(0.0));
    let panel_height_value = (viewport_height * 0.90)
        .clamp(520.0, 820.0)
        .min((viewport_height - 24.0).max(0.0));
    let width_class = if panel_width_value < 980.0 {
        YtttSettingsWidthClass::Compact
    } else if panel_width_value < 1_180.0 {
        YtttSettingsWidthClass::Regular
    } else {
        YtttSettingsWidthClass::Wide
    };
    let height_class = if panel_height_value < 620.0 {
        YtttSettingsHeightClass::Compact
    } else if panel_height_value < 780.0 {
        YtttSettingsHeightClass::Regular
    } else {
        YtttSettingsHeightClass::Tall
    };
    let (sidebar_width, control_width, content_padding_x) = match width_class {
        YtttSettingsWidthClass::Compact => (
            px(192.0),
            px(f32::from(ui_style.controls.settings_control_width).min(200.0)),
            ui_style.spacing.xl,
        ),
        YtttSettingsWidthClass::Regular => (
            px(208.0),
            ui_style.controls.settings_control_width,
            ui_style.spacing.xxl,
        ),
        YtttSettingsWidthClass::Wide => (
            px(224.0),
            ui_style.controls.settings_control_width,
            ui_style.settings.content_padding_x,
        ),
    };
    let (content_padding_y, section_gap) = match height_class {
        YtttSettingsHeightClass::Compact => (ui_style.spacing.lg, ui_style.spacing.xxl),
        YtttSettingsHeightClass::Regular | YtttSettingsHeightClass::Tall => {
            (ui_style.settings.content_padding_y, ui_style.spacing.xxxl)
        }
    };

    YtttSettingsLayout {
        panel_width: px(panel_width_value),
        panel_height: px(panel_height_value),
        sidebar_width,
        control_width,
        compact_control_width: ui_style.controls.settings_compact_control_width,
        control_height: ui_style.controls.settings_height,
        search_height: ui_style.controls.search_height,
        content_padding_x,
        content_padding_y,
        section_gap,
        width_class,
        height_class,
        stack_rows: panel_width_value < 920.0,
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
        YtttPanelKind::Palette => (
            ui_style.palette.panel_width,
            None,
            ui_style.palette.panel_max_width,
            ui_style.palette.panel_max_height,
            ui_style.palette.body_max_height,
            px(0.0),
        ),
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
