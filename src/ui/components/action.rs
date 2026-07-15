use super::*;

pub fn workbench_action_button(
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    shortcut: Option<&'static str>,
    emphasis: ActionEmphasis,
) -> Button {
    let mut button = Button::new(id).label(label).compact().rounded(px(6.0));
    if let Some(shortcut) = shortcut {
        button = button.child(Kbd::new(shortcut_keystroke(shortcut)));
    }

    match emphasis {
        ActionEmphasis::Primary => button.primary(),
        ActionEmphasis::Secondary => button.outline(),
    }
}

pub fn workbench_icon_button<H>(
    id: impl Into<ElementId>,
    icon: IconName,
    kind: YtttIconButtonKind,
    theme: WorkbenchTheme,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = yttt_icon_button_style(kind, theme);

    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .size(style.size)
        .rounded(style.radius)
        .border_l(style.border_width)
        .border_color(style.border)
        .bg(style.background)
        .text_color(style.text)
        .hover(move |this| this.bg(style.hover_background).text_color(style.hover_text))
        .on_click(on_click)
        .child(Icon::new(icon).size(style.icon_size))
}

fn shortcut_keystroke(shortcut: &str) -> Keystroke {
    Keystroke::parse(shortcut).expect("workbench shortcut should be a valid GPUI keystroke")
}
