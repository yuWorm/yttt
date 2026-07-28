use gpui::{Entity, Pixels, Rems, Rgba, prelude::*};
use gpui_component::{
    Sizable as _,
    input::{Input, InputState, NumberInput},
};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YtttInputKind {
    Dialog,
    Search,
    Palette,
    Settings,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct YtttInputStyle {
    pub height: Rems,
    pub radius: Pixels,
    pub background: Rgba,
    pub border: Rgba,
    pub focused_border: Rgba,
    pub text: Rgba,
    pub placeholder: Rgba,
}

pub fn yttt_input_style(
    kind: YtttInputKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> YtttInputStyle {
    YtttInputStyle {
        height: match kind {
            YtttInputKind::Dialog => ui_style.controls.dialog_input_height,
            YtttInputKind::Palette => ui_style.controls.palette_input_height,
            YtttInputKind::Search => ui_style.controls.search_height,
            YtttInputKind::Settings => ui_style.controls.settings_height,
        },
        radius: match kind {
            YtttInputKind::Settings => ui_style.radius.control,
            YtttInputKind::Dialog | YtttInputKind::Palette | YtttInputKind::Search => {
                ui_style.radius.input
            }
        },
        background: theme.surface_elevated,
        border: theme.border,
        focused_border: theme.focus_ring,
        text: theme.text,
        placeholder: theme.text_subtle,
    }
}

pub fn yttt_input(
    state: &Entity<InputState>,
    kind: YtttInputKind,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Input {
    let style = yttt_input_style(kind, theme, ui_style);
    Input::new(state)
        .appearance(true)
        .h(style.height)
        .rounded(style.radius)
        .border_color(style.border)
        .bg(style.background)
        .text_color(style.text)
}

pub fn yttt_borderless_input(state: &Entity<InputState>) -> Input {
    Input::new(state).appearance(false)
}

pub fn yttt_number_input(
    state: &Entity<InputState>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> NumberInput {
    let style = yttt_input_style(YtttInputKind::Settings, theme, ui_style);
    NumberInput::new(state)
        .small()
        .appearance(true)
        .h(style.height)
        .rounded(style.radius)
        .border_color(style.border)
        .bg(style.background)
        .text_color(style.text)
}
