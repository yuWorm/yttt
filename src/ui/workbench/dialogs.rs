use super::*;

pub(super) fn tab_rename_dialog(
    cx: &mut Context<WorkbenchView>,
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
                    .child(yttt_dialog_header(
                        "close-tab-rename-dialog",
                        ui_text.get(UiTextKey::RenameTabTitle),
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.cancel_tab_rename_dialog();
                            cx.notify();
                        }),
                    ))
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
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    command: CommandId,
    keybindings: &[String],
    error: Option<&str>,
    theme: WorkbenchTheme,
) -> Div {
    let dialog = yttt_dialog_style(theme);
    let recorded = if keybindings.is_empty() {
        div()
            .text_sm()
            .text_color(dialog.hint)
            .child(ui_text.get(UiTextKey::SettingsKeybindingRecorderPrompt))
    } else {
        keybindings.iter().fold(
            div()
                .flex()
                .flex_wrap()
                .items_center()
                .justify_center()
                .gap_2(),
            |bindings, keybinding| {
                bindings.child(workbench_keybinding_badge(keybinding.clone(), theme))
            },
        )
    };

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
                    .child(yttt_dialog_header(
                        "close-keybinding-edit-dialog",
                        ui_text.get(UiTextKey::SettingsKeybindingDialogTitle),
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.cancel_keybinding_edit_dialog();
                            cx.notify();
                        }),
                    ))
                    .child(
                        div()
                            .text_xs()
                            .text_color(dialog.hint)
                            .child(command_title_with_text(command, ui_text)),
                    )
                    .child(
                        div()
                            .id(SharedString::from("keybinding-recorder"))
                            .debug_selector(|| "keybinding-recorder".to_string())
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_h_16()
                            .rounded_md()
                            .border_1()
                            .border_color(theme.focus_ring)
                            .bg(theme.surface_elevated)
                            .px_4()
                            .py_3()
                            .child(recorded),
                    )
                    .child(
                        div()
                            .text_xs()
                            .text_color(dialog.hint)
                            .child(ui_text.get(UiTextKey::SettingsKeybindingRecorderHint)),
                    )
                    .when_some(error.map(str::to_string), |dialog, error| {
                        dialog.child(div().text_xs().text_color(theme.danger).child(error))
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap_2()
                            .child(yttt_dialog_button(
                                cx,
                                "clear-keybinding-edit",
                                ui_text.get(UiTextKey::SettingsClearKeybindings),
                                YtttButtonVariant::Secondary,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    this.clear_keybinding_edit_keys();
                                    cx.notify();
                                }),
                            ))
                            .child(yttt_dialog_button(
                                cx,
                                "cancel-keybinding-edit",
                                ui_text.get(UiTextKey::Cancel),
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
                                ui_text.get(UiTextKey::SettingsSave),
                                YtttButtonVariant::Primary,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.confirm_keybinding_edit_dialog();
                                    cx.notify();
                                }),
                            )),
                    ),
            ),
    )
}

pub(super) fn file_conflict_dialog(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    path: String,
    missing: bool,
) -> Div {
    let dialog = yttt_dialog_style(theme);
    let title = ui_text.get(if missing {
        UiTextKey::FileDeletedOnDisk
    } else {
        UiTextKey::FileChangedOnDisk
    });
    let overwrite_label = ui_text.get(if missing {
        UiTextKey::FileRecreate
    } else {
        UiTextKey::FileOverwrite
    });
    let mut actions = div().flex().justify_end().gap_2().child(yttt_dialog_button(
        cx,
        "cancel-file-conflict",
        ui_text.get(UiTextKey::Cancel),
        YtttButtonVariant::Secondary,
        theme,
        cx.listener(|this, _, _window, cx| {
            this.cancel_pending_file_conflict(cx);
            cx.notify();
        }),
    ));
    if !missing {
        actions = actions.child(yttt_dialog_button(
            cx,
            "reload-file-conflict",
            ui_text.get(UiTextKey::FileReload),
            YtttButtonVariant::Secondary,
            theme,
            cx.listener(|this, _, window, cx| {
                this.reload_pending_file_conflict(window, cx);
            }),
        ));
    }
    actions = actions.child(yttt_dialog_button(
        cx,
        "overwrite-file-conflict",
        overwrite_label,
        YtttButtonVariant::Danger,
        theme,
        cx.listener(|this, _, window, cx| {
            this.overwrite_pending_file_conflict(window, cx);
        }),
    ));

    capture_overlay_input(
        div()
            .debug_selector(|| "file-conflict-dialog".to_string())
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
                    .child(yttt_dialog_header(
                        "close-file-conflict-dialog",
                        title,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.cancel_pending_file_conflict(cx);
                            cx.notify();
                        }),
                    ))
                    .child(workbench_inline_notification(
                        ToastItem {
                            title: path,
                            context: ui_text.get(UiTextKey::StatusWarningContext).to_string(),
                            tone: ToastTone::Warning,
                        },
                        theme,
                    ))
                    .child(actions),
            ),
    )
}

pub(super) fn dirty_close_dialog(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    title: String,
    details: Vec<String>,
    file_intent: bool,
    has_save_error: bool,
) -> Div {
    let dialog = yttt_dialog_style(theme);
    let save_label = ui_text.get(if file_intent {
        UiTextKey::FileSaveAction
    } else {
        UiTextKey::SaveAllAndContinue
    });
    let discard_label = ui_text.get(if file_intent {
        UiTextKey::Discard
    } else {
        UiTextKey::DiscardAndContinue
    });
    let summary = details.join("\n");
    let mut content = div()
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
        .child(yttt_dialog_header(
            "close-dirty-file-dialog",
            title,
            theme,
            cx.listener(|this, _, _window, cx| {
                this.cancel_pending_dirty_close();
                cx.notify();
            }),
        ));
    if !summary.is_empty() {
        content = content.child(workbench_inline_notification(
            ToastItem {
                title: summary,
                context: ui_text.get(UiTextKey::StatusWarningContext).to_string(),
                tone: ToastTone::Warning,
            },
            theme,
        ));
    }
    if has_save_error {
        content = content.child(
            div()
                .text_xs()
                .text_color(dialog.hint)
                .child(ui_text.get(UiTextKey::CloseSaveFailureGuidance)),
        );
    }
    content = content.child(
        div()
            .flex()
            .justify_end()
            .gap_2()
            .child(yttt_dialog_button(
                cx,
                "cancel-dirty-close",
                ui_text.get(UiTextKey::Cancel),
                YtttButtonVariant::Secondary,
                theme,
                cx.listener(|this, _, _window, cx| {
                    this.cancel_pending_dirty_close();
                    cx.notify();
                }),
            ))
            .child(yttt_dialog_button(
                cx,
                "discard-dirty-close",
                discard_label,
                YtttButtonVariant::Danger,
                theme,
                cx.listener(|this, _, window, cx| {
                    this.discard_pending_dirty_close(window, cx);
                }),
            ))
            .child(yttt_dialog_button(
                cx,
                "save-dirty-close",
                save_label,
                YtttButtonVariant::Primary,
                theme,
                cx.listener(|this, _, window, cx| {
                    this.save_pending_dirty_close(window, cx);
                }),
            )),
    );

    capture_overlay_input(
        div()
            .debug_selector(|| "dirty-close-dialog".to_string())
            .absolute()
            .top_0()
            .left_0()
            .right_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(dialog.overlay)
            .child(content),
    )
}

pub(super) fn close_project_dialog(
    cx: &mut Context<WorkbenchView>,
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
                    .child(yttt_dialog_header(
                        "close-project-dialog",
                        ui_text.get(UiTextKey::CloseProjectTitle),
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            this.cancel_pending_project_close();
                            cx.notify();
                        }),
                    ))
                    .child(workbench_inline_notification(
                        ToastItem {
                            title: ui_text.get(UiTextKey::CloseProjectBody).to_string(),
                            context: ui_text.get(UiTextKey::StatusWarningContext).to_string(),
                            tone: ToastTone::Warning,
                        },
                        theme,
                    ))
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

fn yttt_dialog_header<H>(
    id: &'static str,
    title: impl Into<SharedString>,
    theme: WorkbenchTheme,
    on_close: H,
) -> Div
where
    H: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_3()
        .w_full()
        .child(
            div()
                .min_w_0()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(workbench_icon_button(
            id,
            IconName::Close,
            YtttIconButtonKind::OverlayClose,
            theme,
            on_close,
        ))
}

fn yttt_dialog_input(input: &Entity<InputState>, theme: WorkbenchTheme) -> Div {
    let style = yttt_input_style(YtttInputKind::Dialog, theme);
    div()
        .flex()
        .items_center()
        .h(style.height)
        .rounded(style.radius)
        .bg(style.background)
        .overflow_hidden()
        .text_color(style.text)
        .child(
            Input::new(input)
                .cleanable(false)
                .appearance(true)
                .rounded(style.radius)
                .bg(style.background),
        )
}

fn yttt_dialog_button<H>(
    cx: &mut Context<WorkbenchView>,
    id: &'static str,
    label: &'static str,
    variant: YtttButtonVariant,
    theme: WorkbenchTheme,
    on_click: H,
) -> Button
where
    H: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    yttt_button(id, label, variant, theme, cx).on_click(on_click)
}

pub(super) fn empty_workspace(
    cx: &mut Context<WorkbenchView>,
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
    cx: &mut Context<WorkbenchView>,
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
