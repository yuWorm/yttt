use super::*;
use yttt_ui::primitives::kbd::yttt_kbd;

pub fn workbench_keybinding_badge(
    keybinding: impl Into<String>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> AnyElement {
    let keybinding = keybinding.into();
    if let Some(keystroke) = parse_keybinding_for_display(&keybinding) {
        yttt_kbd(keystroke, theme, ui_style).into_any_element()
    } else if !ui_style.palette.bordered_shortcuts {
        div()
            .text_sm()
            .text_color(theme.text_muted)
            .child(keybinding)
            .into_any_element()
    } else {
        div()
            .rounded(ui_style.radius.compact)
            .border(ui_style.border.hairline)
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .px(ui_style.spacing.xs)
            .py(ui_style.spacing.xxs)
            .text_xs()
            .text_color(theme.text_muted)
            .child(keybinding)
            .into_any_element()
    }
}
