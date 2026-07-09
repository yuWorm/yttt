use super::*;

pub(super) fn tab_rename_dialog(
    cx: &mut Context<RootView>,
    ui_text: &UiText,
    input: &Entity<InputState>,
    theme: WorkbenchTheme,
) -> Div {
    let dialog = yttt_dialog_style(theme);
    capture_overlay_input(
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_start()
            .justify_center()
            .pt_16()
            .bg(dialog.overlay)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w(dialog.max_width)
                    .rounded(dialog.radius)
                    .border_1()
                    .border_color(dialog.border)
                    .bg(dialog.background)
                    .p(dialog.padding)
                    .text_color(dialog.text)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(ui_text.get(UiTextKey::RenameTabTitle)),
                    )
                    .child(yttt_dialog_input(input, theme))
                    .child(
                        div()
                            .text_xs()
                            .text_color(dialog.hint)
                            .child(ui_text.get(UiTextKey::RenameTabHint)),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(yttt_dialog_button(
                                cx,
                                "cancel-tab-rename",
                                ui_text.get(UiTextKey::Cancel),
                                YtttButtonVariant::Secondary,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    this.cancel_tab_rename_dialog();
                                    cx.notify();
                                }),
                            ))
                            .child(yttt_dialog_button(
                                cx,
                                "confirm-tab-rename",
                                ui_text.get(UiTextKey::RenameTabAction),
                                YtttButtonVariant::Primary,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.confirm_tab_rename_dialog_from_input(cx);
                                    cx.notify();
                                }),
                            )),
                    ),
            ),
    )
}

pub(super) fn keybinding_edit_dialog(
    cx: &mut Context<RootView>,
    input: &Entity<InputState>,
    theme: WorkbenchTheme,
) -> Div {
    let dialog = yttt_dialog_style(theme);
    capture_overlay_input(
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_start()
            .justify_center()
            .pt_16()
            .bg(dialog.overlay)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w(dialog.max_width)
                    .rounded(dialog.radius)
                    .border_1()
                    .border_color(dialog.border)
                    .bg(dialog.background)
                    .p(dialog.padding)
                    .text_color(dialog.text)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child("Edit keybinding"),
                    )
                    .child(yttt_dialog_input(input, theme))
                    .child(
                        div()
                            .text_xs()
                            .text_color(dialog.hint)
                            .child("Separate multiple bindings with commas."),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(yttt_dialog_button(
                                cx,
                                "cancel-keybinding-edit",
                                "Cancel",
                                YtttButtonVariant::Secondary,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    this.cancel_keybinding_edit_dialog();
                                    cx.notify();
                                }),
                            ))
                            .child(yttt_dialog_button(
                                cx,
                                "confirm-keybinding-edit",
                                "Save",
                                YtttButtonVariant::Primary,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.confirm_keybinding_edit_dialog_from_input(cx);
                                    cx.notify();
                                }),
                            )),
                    ),
            ),
    )
}

pub(super) fn close_project_dialog(
    cx: &mut Context<RootView>,
    ui_text: &UiText,
    theme: WorkbenchTheme,
) -> Div {
    let dialog = yttt_dialog_style(theme);
    capture_overlay_input(
        div()
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(dialog.overlay)
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap_3()
                    .w(dialog.max_width)
                    .rounded(dialog.radius)
                    .border_1()
                    .border_color(dialog.border)
                    .bg(dialog.background)
                    .p(dialog.padding)
                    .text_color(dialog.text)
                    .child(
                        div()
                            .text_sm()
                            .font_weight(gpui::FontWeight::SEMIBOLD)
                            .child(ui_text.get(UiTextKey::CloseProjectTitle)),
                    )
                    .child(
                        Alert::warning(
                            "close-project-warning",
                            ui_text.get(UiTextKey::CloseProjectBody),
                        )
                        .banner(),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(dialog.hint)
                            .child("Enter to close, Escape to cancel"),
                    )
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(yttt_dialog_button(
                                cx,
                                "cancel-close-project",
                                ui_text.get(UiTextKey::Cancel),
                                YtttButtonVariant::Secondary,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    this.cancel_pending_project_close();
                                    cx.notify();
                                }),
                            ))
                            .child(yttt_dialog_button(
                                cx,
                                "confirm-close-project",
                                ui_text.get(UiTextKey::CloseProjectAction),
                                YtttButtonVariant::Danger,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.confirm_pending_project_close();
                                    cx.notify();
                                }),
                            )),
                    ),
            ),
    )
}

fn yttt_dialog_input(input: &Entity<InputState>, theme: WorkbenchTheme) -> Div {
    let style = yttt_input_style(YtttInputKind::Dialog, theme);
    div()
        .flex()
        .items_center()
        .h(style.height)
        .rounded(style.radius)
        .border_1()
        .border_color(style.border)
        .bg(style.background)
        .px_2()
        .text_color(style.text)
        .child(Input::new(input).cleanable(false).appearance(false))
}

fn yttt_dialog_button<H>(
    cx: &mut Context<RootView>,
    id: &'static str,
    label: &'static str,
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    on_click: H,
) -> Button
where
    H: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    let style = yttt_button_style(variant, theme);
    let variant = ButtonCustomVariant::new(cx)
        .color(style.background.into())
        .foreground(style.text.into())
        .border(style.border.into())
        .hover(style.hover_background.into())
        .active(style.background.into())
        .shadow(false);

    Button::new(id)
        .label(label)
        .compact()
        .rounded(style.radius)
        .custom(variant)
        .on_click(on_click)
}

pub(super) fn empty_workspace(
    cx: &mut Context<RootView>,
    ui_text: &UiText,
    theme: &WorkbenchTheme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .flex_1()
        .w_full()
        .relative()
        .justify_center()
        .items_center()
        .bg(theme.app_background)
        .text_color(theme.text)
        .child(div().text_xl().child(ui_text.get(UiTextKey::AppName)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .items_center()
                .text_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.text_muted)
                        .child(ui_text.get(UiTextKey::EmptySubtitle)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(ui_text.get(UiTextKey::EmptySidebarNote)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .justify_center()
                .child(
                    workbench_action_button(
                        "empty-open-directory",
                        ui_text.get(UiTextKey::OpenDirectory),
                        "secondary-o",
                        ActionEmphasis::Primary,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_open_project(&OpenProject, window, cx);
                    })),
                )
                .child(
                    workbench_action_button(
                        "empty-open-recent",
                        ui_text.get(UiTextKey::OpenRecent),
                        "secondary-shift-o",
                        ActionEmphasis::Secondary,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_open_project_palette(&OpenProjectPalette, window, cx);
                    })),
                )
                .child(
                    workbench_action_button(
                        "empty-command-palette",
                        ui_text.get(UiTextKey::CommandPalette),
                        "secondary-p",
                        ActionEmphasis::Secondary,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_open_command_palette(&OpenCommandPalette, window, cx);
                    })),
                ),
        )
}

pub(super) fn project_empty_terminal_state(
    cx: &mut Context<RootView>,
    ui_text: &UiText,
    theme: &WorkbenchTheme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .flex_1()
        .w_full()
        .justify_center()
        .items_center()
        .bg(theme.terminal_background)
        .text_color(theme.text)
        .child(
            div()
                .text_sm()
                .text_color(theme.text_muted)
                .child(ui_text.get(UiTextKey::NoTerminalTabs)),
        )
        .child(
            workbench_action_button(
                "project-empty-new-tab",
                ui_text.get(UiTextKey::NewTab),
                "secondary-t",
                ActionEmphasis::Primary,
            )
            .on_click(cx.listener(|this, _, _window, cx| {
                let _ = this.run_command(CommandId::TabNew);
                cx.notify();
            })),
        )
}
