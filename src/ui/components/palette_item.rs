use super::*;

pub fn workbench_palette_item<H>(
    id: impl Into<ElementId>,
    title: impl Into<String>,
    subtitle: impl Into<String>,
    status: impl Into<String>,
    keybinding: Option<String>,
    state: SelectableState,
    enabled: bool,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = palette_row_style(state, enabled, theme, ui_style);
    let title = title.into();
    let subtitle = subtitle.into();
    let status = status.into();
    let keybinding = keybinding.filter(|keybinding| !keybinding.trim().is_empty());

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_between()
        .gap(ui_style.spacing.xl)
        .h(style.height)
        .rounded(style.radius)
        .border(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .px(style.padding_x)
        .hover(move |this| this.bg(style.hover_background))
        .on_click(on_click)
        .child(
            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.xs)
                .overflow_hidden()
                .child(
                    div()
                        .text_sm()
                        .text_color(style.title)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(style.subtitle)
                        .truncate()
                        .child(subtitle),
                ),
        )
        .child(palette_item_trailing(
            status,
            keybinding,
            style.status,
            theme,
            ui_style,
        ))
}

pub fn workbench_keybinding_badge(
    keybinding: impl Into<String>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> AnyElement {
    let keybinding = keybinding.into();
    if let Some(keystroke) = parse_keybinding_for_display(&keybinding) {
        Kbd::new(keystroke)
            .border_color(theme.border)
            .bg(theme.surface_elevated)
            .text_color(theme.text_muted)
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

fn palette_item_trailing(
    status: String,
    keybinding: Option<String>,
    status_color: gpui::Rgba,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let mut trailing = div()
        .flex_none()
        .flex()
        .items_center()
        .justify_end()
        .gap(ui_style.spacing.md);

    if !status.is_empty() {
        trailing = trailing.child(
            div()
                .text_xs()
                .text_color(status_color)
                .truncate()
                .child(status),
        );
    }

    if let Some(keybinding) = keybinding {
        trailing = trailing.child(workbench_keybinding_badge(keybinding, theme, ui_style));
    }

    trailing
}
