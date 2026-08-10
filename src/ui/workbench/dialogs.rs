use super::*;
use gpui::{ImageSource, Resource, img};
use gpui_component::{Icon, kbd::Kbd};

pub(super) fn tab_rename_dialog(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    input: &Entity<InputState>,
    theme: WorkbenchTheme,
) -> Div {
    let ui_style = current_ui_style(cx);
    let dialog = yttt_dialog_style(theme, ui_style);
    yttt_dialog_overlay(
        yttt_dialog_surface(theme, ui_style)
            .gap(ui_style.spacing.lg)
            .child(yttt_dialog_header(
                "close-tab-rename-dialog",
                ui_text.get(UiTextKey::RenameTabTitle),
                theme,
                ui_style,
                cx.listener(|this, _, _window, cx| {
                    this.cancel_tab_rename_dialog();
                    cx.notify();
                }),
            ))
            .child(yttt_dialog_input(input, theme, ui_style))
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
                    .gap(ui_style.spacing.md)
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
        YtttDialogPlacement::Top,
        theme,
        ui_style,
    )
}

pub(super) fn keybinding_edit_dialog(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    action: BindableActionId,
    profile: KeybindingProfile,
    keybindings: &[String],
    original_keybindings: &[String],
    is_recording: bool,
    recording_index: Option<usize>,
    error: Option<&str>,
    theme: WorkbenchTheme,
) -> Div {
    let ui_style = current_ui_style(cx);
    let dialog = yttt_dialog_style(theme, ui_style);
    let action_title = match action.command() {
        Some(command) => command_title_with_text(command, ui_text),
        None => action.title().unwrap_or(action.as_str()),
    };
    let profile_label = match profile {
        KeybindingProfile::Base => ui_text.get(UiTextKey::SettingsKeybindingProfileBase),
        KeybindingProfile::Vim => ui_text.get(UiTextKey::SettingsKeybindingProfileVim),
    };
    let dirty = keybindings != original_keybindings;
    let current_bindings = if keybindings.is_empty() {
        div()
            .debug_selector(|| "keybinding-current-bindings".to_string())
            .flex()
            .items_center()
            .justify_center()
            .min_h_12()
            .rounded(ui_style.radius.control)
            .border(ui_style.border.hairline)
            .border_color(theme.border_variant.alpha(0.65))
            .bg(theme.editor_background.alpha(0.45))
            .text_xs()
            .text_color(dialog.hint)
            .child(ui_text.get(UiTextKey::SettingsKeybindingNoBindings))
            .into_any_element()
    } else {
        keybindings
            .iter()
            .enumerate()
            .fold(
                div()
                    .debug_selector(|| "keybinding-current-bindings".to_string())
                    .flex()
                    .flex_col()
                    .gap(ui_style.spacing.xs)
                    .max_h(px(168.0))
                    .overflow_y_scrollbar(),
                |bindings, (index, keybinding)| {
                    bindings.child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap(ui_style.spacing.lg)
                            .rounded(ui_style.radius.control)
                            .border(ui_style.border.hairline)
                            .border_color(theme.border_variant.alpha(0.65))
                            .bg(theme.editor_background.alpha(0.45))
                            .px(ui_style.spacing.md)
                            .py(ui_style.spacing.sm)
                            .child(workbench_keybinding_badge(
                                keybinding.clone(),
                                theme,
                                ui_style,
                            ))
                            .child(
                                yttt_button(
                                    format!("remove-keybinding-{index}"),
                                    ui_text.get(UiTextKey::SettingsKeybindingRemove),
                                    YtttButtonVariant::Ghost,
                                    theme,
                                    ui_style,
                                    cx,
                                )
                                .on_click(cx.listener(
                                    move |this, _, _window, cx| {
                                        this.remove_keybinding_edit_key(index);
                                        cx.notify();
                                    },
                                )),
                            ),
                    )
                },
            )
            .into_any_element()
    };
    let recorder_value = recording_index
        .and_then(|index| keybindings.get(index))
        .map(|keybinding| workbench_keybinding_badge(keybinding.clone(), theme, ui_style));

    let mut body = div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.lg)
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(ui_style.spacing.lg)
                .rounded(ui_style.radius.control)
                .bg(theme.editor_background.alpha(0.45))
                .px(ui_style.spacing.md)
                .py(ui_style.spacing.sm)
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_sm()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(action_title),
                )
                .child(
                    div()
                        .flex_none()
                        .rounded_full()
                        .border(ui_style.border.hairline)
                        .border_color(theme.border_variant)
                        .px(ui_style.spacing.sm)
                        .py(ui_style.spacing.xxs)
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(profile_label),
                ),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .gap(ui_style.spacing.sm)
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text_muted)
                        .child(ui_text.get(UiTextKey::SettingsKeybindingCurrentBindings)),
                )
                .child(current_bindings),
        );

    if is_recording {
        body = body.child(
            div()
                .id(SharedString::from("keybinding-recorder"))
                .debug_selector(|| "keybinding-recorder".to_string())
                .flex()
                .flex_col()
                .items_center()
                .gap(ui_style.spacing.md)
                .min_h_24()
                .rounded(ui_style.radius.control)
                .border(ui_style.border.emphasized)
                .border_color(theme.accent)
                .bg(theme.active_surface.alpha(0.55))
                .px(ui_style.spacing.xl)
                .py(ui_style.spacing.lg)
                .child(
                    div()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.accent)
                        .child(ui_text.get(UiTextKey::SettingsKeybindingRecording)),
                )
                .child(recorder_value.unwrap_or_else(|| {
                    div()
                        .text_sm()
                        .text_color(dialog.hint)
                        .child(ui_text.get(UiTextKey::SettingsKeybindingRecorderPrompt))
                        .into_any_element()
                }))
                .child(
                    div()
                        .text_xs()
                        .text_color(dialog.hint)
                        .child(ui_text.get(UiTextKey::SettingsKeybindingRecorderHint)),
                )
                .child(
                    div()
                        .debug_selector(|| "finish-keybinding-recording".to_string())
                        .child(yttt_dialog_button(
                            cx,
                            "finish-keybinding-recording-button",
                            ui_text.get(UiTextKey::SettingsKeybindingFinishRecording),
                            YtttButtonVariant::Secondary,
                            theme,
                            cx.listener(|this, _, _window, cx| {
                                this.finish_keybinding_edit_recording();
                                cx.notify();
                            }),
                        )),
                ),
        );
    } else {
        body = body.child(
            div()
                .debug_selector(|| "keybinding-recording-actions".to_string())
                .flex()
                .items_center()
                .gap(ui_style.spacing.md)
                .child(
                    div()
                        .debug_selector(|| "replace-keybinding".to_string())
                        .child(yttt_dialog_button(
                            cx,
                            "replace-keybinding-button",
                            ui_text.get(UiTextKey::SettingsKeybindingReplace),
                            YtttButtonVariant::Primary,
                            theme,
                            cx.listener(|this, _, _window, cx| {
                                this.begin_keybinding_edit_replacement();
                                cx.notify();
                            }),
                        )),
                )
                .child(
                    div()
                        .id("add-keybinding-alternative")
                        .debug_selector(|| "add-keybinding-alternative".to_string())
                        .child(yttt_dialog_button(
                            cx,
                            "add-keybinding-alternative-button",
                            ui_text.get(UiTextKey::SettingsAddKeybindingAlternative),
                            YtttButtonVariant::Secondary,
                            theme,
                            cx.listener(|this, _, _window, cx| {
                                this.begin_keybinding_edit_alternative();
                                cx.notify();
                            }),
                        )),
                ),
        );
    }

    if let Some(error) = error {
        body = body.child(
            div()
                .debug_selector(|| "keybinding-edit-error".to_string())
                .rounded(ui_style.radius.control)
                .border(ui_style.border.hairline)
                .border_color(theme.danger.alpha(0.65))
                .bg(theme.danger.alpha(0.12))
                .px(ui_style.spacing.md)
                .py(ui_style.spacing.sm)
                .text_xs()
                .text_color(theme.danger)
                .child(error.to_string()),
        );
    }

    let footer = div()
        .debug_selector(|| "keybinding-edit-footer".to_string())
        .flex()
        .items_center()
        .justify_between()
        .gap(ui_style.spacing.lg)
        .pt(ui_style.spacing.md)
        .border_t(ui_style.border.hairline)
        .border_color(theme.border_variant.alpha(0.65))
        .child(
            div()
                .flex()
                .items_center()
                .gap(ui_style.spacing.xs)
                .child(yttt_dialog_button(
                    cx,
                    "reset-keybinding-edit",
                    ui_text.get(UiTextKey::SettingsReset),
                    YtttButtonVariant::Ghost,
                    theme,
                    cx.listener(|this, _, _window, cx| {
                        this.reset_keybinding_edit_keys();
                        cx.notify();
                    }),
                ))
                .child(yttt_dialog_button(
                    cx,
                    "clear-keybinding-edit",
                    ui_text.get(UiTextKey::SettingsClearKeybindings),
                    YtttButtonVariant::Ghost,
                    theme,
                    cx.listener(|this, _, _window, cx| {
                        this.clear_keybinding_edit_keys();
                        cx.notify();
                    }),
                )),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(ui_style.spacing.sm)
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
                .child(
                    yttt_dialog_button(
                        cx,
                        "confirm-keybinding-edit",
                        ui_text.get(UiTextKey::SettingsSave),
                        YtttButtonVariant::Primary,
                        theme,
                        cx.listener(|this, _, _window, cx| {
                            let _ = this.confirm_keybinding_edit_dialog();
                            cx.notify();
                        }),
                    )
                    .disabled(!dirty || is_recording || error.is_some()),
                ),
        );

    yttt_dialog_overlay(
        yttt_dialog_surface(theme, ui_style)
            .debug_selector(|| "keybinding-edit-dialog".to_string())
            .w(px(520.0))
            .max_w(px(560.0))
            .max_h(px(620.0))
            .gap(ui_style.spacing.lg)
            .child(yttt_dialog_header(
                "close-keybinding-edit-dialog",
                ui_text.get(UiTextKey::SettingsKeybindingDialogTitle),
                theme,
                ui_style,
                cx.listener(|this, _, _window, cx| {
                    this.cancel_keybinding_edit_dialog();
                    cx.notify();
                }),
            ))
            .child(body)
            .child(footer),
        YtttDialogPlacement::Center,
        theme,
        ui_style,
    )
}

pub(super) fn zed_theme_import_dialog(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    detection: &ZedThemeDetection,
    conflict_policy: ZedThemeImportConflictPolicy,
    config_paths: &AppConfigPaths,
    theme: WorkbenchTheme,
) -> Div {
    let ui_style = current_ui_style(cx);
    let dialog = yttt_dialog_style(theme, ui_style);
    let ui_output_dir = config_paths.themes_dir();
    let icon_output_dir = config_paths.icon_themes_dir();
    let existing_count =
        detected_zed_theme_existing_count(detection, &ui_output_dir, &icon_output_dir);

    yttt_dialog_overlay(
        yttt_dialog_surface(theme, ui_style)
            .debug_selector(|| "zed-theme-import-dialog".to_string())
            .gap(ui_style.spacing.lg)
            .max_h(px(640.0))
                    .child(yttt_dialog_header("close-zed-theme-import-dialog", ui_text.get(UiTextKey::SettingsImportZedThemes), theme, ui_style, cx.listener(|this, _, _window, cx| {
                        this.cancel_zed_theme_import_dialog();
                        cx.notify();
                    })))
                    .child(
                        div()
                            .text_xs()
                            .text_color(dialog.hint)
                            .child(ui_text.get(UiTextKey::SettingsImportZedThemesDescription)),
                    )
                    .when(detection.ui_theme_count() > 0, |this| {
                        this.child(zed_theme_import_panel(
                            ui_text.get(UiTextKey::OnboardingZedUiThemes),
                            &detection.extensions,
                            false,
                            &ui_output_dir,
                            &icon_output_dir,
                            ui_text,
                            theme,
                            ui_style,
                        ))
                    })
                    .when(detection.icon_theme_count() > 0, |this| {
                        this.child(zed_theme_import_panel(
                            ui_text.get(UiTextKey::OnboardingZedIconThemes),
                            &detection.extensions,
                            true,
                            &ui_output_dir,
                            &icon_output_dir,
                            ui_text,
                            theme,
                            ui_style,
                        ))
                    })
                    .when(!detection.warnings.is_empty(), |this| {
                        this.child(
                            div()
                                .rounded(ui_style.radius.control)
                                .border(ui_style.border.hairline)
                                .border_color(theme.warning)
                                .bg(theme.surface_elevated)
                                .px(ui_style.spacing.lg)
                                .py(ui_style.spacing.md)
                                .text_xs()
                                .text_color(theme.warning)
                                .child(format!(
                                    "{} ({})",
                                    ui_text.get(UiTextKey::OnboardingZedDetectionWarnings),
                                    detection.warnings.len()
                                )),
                        )
                    })
                    .when(existing_count > 0, |this| {
                        this.child(
                            div()
                                .flex()
                                .flex_col()
                                .gap(ui_style.spacing.md)
                                .rounded(ui_style.radius.control)
                                .border(ui_style.border.hairline)
                                .border_color(theme.border)
                                .bg(theme.surface_elevated)
                                .px(ui_style.spacing.lg)
                                .py(ui_style.spacing.md)
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(dialog.hint)
                                        .child(
                                            ui_text.get(
                                                UiTextKey::SettingsImportZedThemesConflictHint,
                                            ),
                                        ),
                                )
                                .child(
                                    div()
                                        .flex()
                                        .items_center()
                                        .gap(ui_style.spacing.xl)
                                        .child(
                                            div()
                                                .debug_selector(|| {
                                                    "zed-theme-import-policy-skip".to_string()
                                                })
                                                .child(
                                                    yttt_radio(
                                                        "zed-theme-import-policy-skip-radio",
                                                        ui_text.get(
                                                            UiTextKey::SettingsImportZedThemesSkipExisting,
                                                        ),
                                                        conflict_policy
                                                            == ZedThemeImportConflictPolicy::SkipExisting,
                                                    )
                                                        .on_click(cx.listener(
                                                            |this, checked, _window, cx| {
                                                                if *checked {
                                                                    this.set_zed_theme_import_conflict_policy(
                                                                        ZedThemeImportConflictPolicy::SkipExisting,
                                                                    );
                                                                    cx.notify();
                                                                }
                                                            },
                                                        )),
                                                ),
                                        )
                                        .child(
                                            div()
                                                .debug_selector(|| {
                                                    "zed-theme-import-policy-overwrite".to_string()
                                                })
                                                .child(
                                                    yttt_radio(
                                                        "zed-theme-import-policy-overwrite-radio",
                                                        ui_text.get(
                                                            UiTextKey::SettingsImportZedThemesOverwriteExisting,
                                                        ),
                                                        conflict_policy
                                                            == ZedThemeImportConflictPolicy::OverwriteExisting,
                                                    )
                                                    .on_click(cx.listener(
                                                        |this, checked, _window, cx| {
                                                            if *checked {
                                                                this.set_zed_theme_import_conflict_policy(
                                                                    ZedThemeImportConflictPolicy::OverwriteExisting,
                                                                );
                                                                cx.notify();
                                                            }
                                                        },
                                                    )),
                                                ),
                                        ),
                                ),
                        )
                    })
                    .child(
                        div()
                            .flex()
                            .justify_end()
                            .gap(ui_style.spacing.md)
                            .child(yttt_dialog_button(
                                cx,
                                "cancel-zed-theme-import",
                                ui_text.get(UiTextKey::Cancel),
                                YtttButtonVariant::Secondary,
                                theme,
                                cx.listener(|this, _, _window, cx| {
                                    this.cancel_zed_theme_import_dialog();
                                    cx.notify();
                                }),
                            )
                            .debug_selector(|| "cancel-zed-theme-import".to_string()),
                        )
                            .child(yttt_dialog_button(
                                cx,
                                "confirm-zed-theme-import",
                                ui_text.get(UiTextKey::SettingsImportZedThemesAction),
                                YtttButtonVariant::Primary,
                                theme,
                                cx.listener(|this, _, window, cx| {
                                    match this.confirm_zed_theme_import_dialog() {
                                        Ok((ui_theme_count, icon_theme_count)) => {
                                            let context = format!(
                                                "{}: {}; {}: {}",
                                                this.ui_text.get(UiTextKey::SettingsUiTheme),
                                                ui_theme_count,
                                                this.ui_text.get(UiTextKey::SettingsIconTheme),
                                                icon_theme_count
                                            );
                                            this.queue_status_notification(
                                                this.ui_text.get(
                                                    UiTextKey::SettingsImportZedThemesComplete,
                                                ),
                                                context,
                                            );
                                        }
                                        Err(error) => this.load_error = Some(error),
                                    }
                                    this.flush_pending_status_notifications(window, cx);
                                    cx.notify();
                                }),
                            )
                            .debug_selector(|| "confirm-zed-theme-import".to_string()),
                        )
                    ),
        YtttDialogPlacement::Top,
        theme,
        ui_style,
    )
}

fn detected_zed_theme_existing_count(
    detection: &ZedThemeDetection,
    ui_output_dir: &Path,
    icon_output_dir: &Path,
) -> usize {
    detection
        .extensions
        .iter()
        .map(|extension| {
            extension
                .ui_theme_names
                .iter()
                .filter(|name| {
                    zed_ui_theme_output_path(&extension.id, name, ui_output_dir).exists()
                })
                .count()
                + if zed_icon_theme_output_path(&extension.id, icon_output_dir).exists() {
                    extension.icon_theme_names.len()
                } else {
                    0
                }
        })
        .sum()
}

fn zed_theme_import_panel(
    title: &'static str,
    extensions: &[DetectedZedExtension],
    icon_themes: bool,
    ui_output_dir: &Path,
    icon_output_dir: &Path,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let id_prefix = if icon_themes {
        "zed-theme-import-icon-theme"
    } else {
        "zed-theme-import-ui-theme"
    };
    let mut rows = div().flex().flex_col().gap(ui_style.spacing.xs);
    let mut index = 0usize;
    for extension in extensions {
        let names = if icon_themes {
            &extension.icon_theme_names
        } else {
            &extension.ui_theme_names
        };
        for name in names {
            let row_index = index;
            index += 1;
            let imported = if icon_themes {
                zed_icon_theme_output_path(&extension.id, icon_output_dir).exists()
            } else {
                zed_ui_theme_output_path(&extension.id, name, ui_output_dir).exists()
            };
            let row_selector = format!("{id_prefix}-{row_index}");
            let imported_selector = format!("{id_prefix}-imported-{row_index}");
            rows = rows.child(
                div()
                    .id(SharedString::from(row_selector.clone()))
                    .debug_selector(move || row_selector.clone())
                    .flex()
                    .items_center()
                    .justify_between()
                    .gap(ui_style.spacing.lg)
                    .rounded(ui_style.radius.control)
                    .px(ui_style.spacing.lg)
                    .py(ui_style.spacing.md)
                    .bg(theme.surface_elevated)
                    .child(
                        div()
                            .min_w_0()
                            .flex_1()
                            .text_sm()
                            .text_color(theme.text)
                            .truncate()
                            .child(name.clone()),
                    )
                    .when(imported, |this| {
                        this.child(
                            div()
                                .id(SharedString::from(imported_selector.clone()))
                                .debug_selector(move || imported_selector.clone())
                                .flex_none()
                                .rounded_full()
                                .bg(ui_style.active_background(theme))
                                .px(ui_style.spacing.md)
                                .py(ui_style.spacing.xs)
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(ui_text.get(UiTextKey::SettingsImportZedThemesImported)),
                        )
                    })
                    .child(
                        div()
                            .max_w(px(120.0))
                            .flex_none()
                            .text_xs()
                            .text_color(theme.text_subtle)
                            .truncate()
                            .child(extension.name.clone()),
                    ),
            );
        }
    }

    div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.md)
        .child(
            div()
                .text_xs()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text_muted)
                .child(format!("{title} ({index})")),
        )
        .child(div().max_h(px(160.0)).overflow_y_scrollbar().child(rows))
}

pub(super) fn file_conflict_dialog(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    theme: WorkbenchTheme,
    path: String,
) -> Div {
    let ui_style = current_ui_style(cx);
    let title = ui_text.get(UiTextKey::FileChangedOnDisk);
    let actions = div()
        .flex()
        .justify_end()
        .gap(ui_style.spacing.md)
        .child(yttt_dialog_button(
            cx,
            "cancel-file-conflict",
            ui_text.get(UiTextKey::Cancel),
            YtttButtonVariant::Secondary,
            theme,
            cx.listener(|this, _, _window, cx| {
                this.cancel_pending_file_conflict(cx);
                cx.notify();
            }),
        ))
        .child(yttt_dialog_button(
            cx,
            "reload-file-conflict",
            ui_text.get(UiTextKey::FileReload),
            YtttButtonVariant::Secondary,
            theme,
            cx.listener(|this, _, window, cx| {
                this.reload_pending_file_conflict(window, cx);
            }),
        ))
        .child(yttt_dialog_button(
            cx,
            "overwrite-file-conflict",
            ui_text.get(UiTextKey::FileOverwrite),
            YtttButtonVariant::Danger,
            theme,
            cx.listener(|this, _, window, cx| {
                this.overwrite_pending_file_conflict(window, cx);
            }),
        ));

    yttt_dialog_overlay(
        yttt_dialog_surface(theme, ui_style)
            .debug_selector(|| "file-conflict-dialog".to_string())
            .gap(ui_style.spacing.lg)
            .child(yttt_dialog_header(
                "close-file-conflict-dialog",
                title,
                theme,
                ui_style,
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
                ui_style,
            ))
            .child(actions),
        YtttDialogPlacement::Center,
        theme,
        ui_style,
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
    let ui_style = current_ui_style(cx);
    let dialog = yttt_dialog_style(theme, ui_style);
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
    let mut content = yttt_dialog_surface(theme, ui_style)
        .gap(ui_style.spacing.lg)
        .child(yttt_dialog_header(
            "close-dirty-file-dialog",
            title,
            theme,
            ui_style,
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
            ui_style,
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
            .gap(ui_style.spacing.md)
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

    yttt_dialog_overlay(
        content.debug_selector(|| "dirty-close-dialog".to_string()),
        YtttDialogPlacement::Center,
        theme,
        ui_style,
    )
}

pub(super) fn close_project_dialog(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    theme: WorkbenchTheme,
) -> Div {
    let ui_style = current_ui_style(cx);
    let dialog = yttt_dialog_style(theme, ui_style);
    yttt_dialog_overlay(
        yttt_dialog_surface(theme, ui_style)
            .gap(ui_style.spacing.lg)
            .child(yttt_dialog_header(
                "close-project-dialog",
                ui_text.get(UiTextKey::CloseProjectTitle),
                theme,
                ui_style,
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
                ui_style,
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
                    .gap(ui_style.spacing.md)
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
        YtttDialogPlacement::Center,
        theme,
        ui_style,
    )
}

pub(super) fn yttt_dialog_header<H>(
    id: &'static str,
    title: impl Into<SharedString>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_close: H,
) -> Div
where
    H: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(ui_style.spacing.lg)
        .w_full()
        .child(
            div()
                .min_w_0()
                .text_sm()
                .font_weight(gpui::FontWeight::SEMIBOLD)
                .child(title.into()),
        )
        .child(yttt_icon_button(
            id,
            IconName::Close,
            YtttIconButtonKind::OverlayClose,
            theme,
            ui_style,
            on_close,
        ))
}

pub(super) fn yttt_dialog_input(
    input: &Entity<InputState>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Input {
    yttt_input(input, YtttInputKind::Dialog, theme, ui_style).cleanable(false)
}

pub(super) fn yttt_dialog_button<H>(
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
    let ui_style = current_ui_style(cx);
    yttt_button(id, label, variant, theme, ui_style, cx).on_click(on_click)
}

#[derive(Clone, Copy)]
struct EmptyWorkspaceScale {
    layout: f32,
    text: f32,
}

impl EmptyWorkspaceScale {
    fn for_window(window: &Window) -> Self {
        let viewport = window.viewport_size();
        let layout = (f32::from(viewport.width) / 1100.0)
            .min(f32::from(viewport.height) / 600.0)
            .clamp(1.0, 1.35);

        Self {
            layout,
            text: 1.0 + (layout - 1.0) * 0.6,
        }
    }

    fn pixels(self, value: f32) -> Pixels {
        px(value * self.layout)
    }

    fn text_pixels(self, value: f32) -> Pixels {
        px(value * self.text)
    }

    fn spacing(self, value: gpui::Rems) -> gpui::Rems {
        rems(value.0 * self.layout)
    }

    fn text_rems(self, value: f32) -> gpui::Rems {
        rems(value * self.text)
    }
}

fn empty_workspace_action(
    id: &'static str,
    icon: IconName,
    label: &'static str,
    shortcut: Option<&'static str>,
    featured: bool,
    theme: &WorkbenchTheme,
    ui_style: UiStyle,
    scale: EmptyWorkspaceScale,
    cx: &gpui::App,
) -> Button {
    let content = div()
        .flex()
        .items_center()
        .justify_between()
        .w_full()
        .child(
            div()
                .flex()
                .items_center()
                .gap(scale.spacing(ui_style.spacing.md))
                .child(
                    Icon::new(icon)
                        .size(scale.spacing(rems(1.0)))
                        .text_color(if featured {
                            theme.accent
                        } else {
                            theme.text_muted
                        }),
                )
                .child(
                    div()
                        .text_size(scale.text_rems(0.875))
                        .text_color(theme.text)
                        .when(featured, |this| this.font_weight(FontWeight::SEMIBOLD))
                        .child(label),
                ),
        )
        .children(shortcut.map(|shortcut| {
            Kbd::new(
                Keystroke::parse(shortcut)
                    .expect("empty workspace shortcut should be a valid GPUI keystroke"),
            )
            .text_size(scale.text_rems(0.75))
        }));

    yttt_button_base(id, YtttButtonVariant::Ghost, *theme, ui_style, cx)
        .w_full()
        .h(scale.spacing(rems(2.5)))
        .px(scale.spacing(ui_style.spacing.lg))
        .rounded(ui_style.radius.action)
        .child(content)
        .debug_selector(move || id.to_string())
        .when(featured, |this| this.outline())
}

pub(super) fn empty_workspace(
    window: &Window,
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    theme: &WorkbenchTheme,
    can_restore_last_session: bool,
) -> Div {
    let ui_style = current_ui_style(cx);
    let scale = EmptyWorkspaceScale::for_window(window);
    let logo = div()
        .id("empty-workspace-logo")
        .debug_selector(|| "empty-workspace-logo".to_string())
        .size(scale.pixels(88.0))
        .child(
            img(ImageSource::Resource(Resource::Embedded(
                crate::ui::app::assets::BUILTIN_APP_ICON_ASSET_PATH.into(),
            )))
            .size_full(),
        );

    div()
        .flex()
        .flex_col()
        .flex_1()
        .w_full()
        .relative()
        .justify_center()
        .items_center()
        .px(scale.spacing(ui_style.spacing.xxl))
        .py(scale.spacing(ui_style.spacing.xxl))
        .text_color(theme.text)
        .child(
            div()
                .flex()
                .flex_col()
                .items_center()
                .w_full()
                .max_w(scale.pixels(400.0))
                .gap(scale.spacing(ui_style.spacing.xxl))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(scale.spacing(ui_style.spacing.md))
                        .text_center()
                        .child(logo)
                        .child(
                            div()
                                .font_family("monospace")
                                .text_size(scale.text_pixels(26.0))
                                .line_height(relative(1.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(ui_text.get(UiTextKey::AppName)),
                        )
                        .child(
                            div()
                                .text_size(scale.text_rems(0.875))
                                .text_color(theme.text_muted)
                                .child(ui_text.get(UiTextKey::EmptySubtitle)),
                        ),
                )
                .child(
                    div()
                        .id("empty-workspace-actions")
                        .debug_selector(|| "empty-workspace-actions".to_string())
                        .flex()
                        .flex_col()
                        .gap(scale.spacing(ui_style.spacing.xs))
                        .w_full()
                        .rounded(ui_style.radius.card)
                        .border(ui_style.border.hairline)
                        .border_color(theme.border)
                        .bg(theme.surface_elevated)
                        .p(scale.spacing(ui_style.spacing.sm))
                        .child(
                            empty_workspace_action(
                                "empty-open-directory",
                                IconName::FolderOpen,
                                ui_text.get(UiTextKey::OpenDirectory),
                                Some("secondary-o"),
                                true,
                                theme,
                                ui_style,
                                scale,
                                cx,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.on_open_project(&OpenProject, window, cx);
                                },
                            )),
                        )
                        .child(
                            empty_workspace_action(
                                "empty-open-ssh-project",
                                IconName::Globe,
                                ui_text.get(UiTextKey::SshOpenRemoteProject),
                                None,
                                false,
                                theme,
                                ui_style,
                                scale,
                                cx,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.on_open_ssh_project(&OpenSshProject, window, cx);
                                },
                            )),
                        )
                        .child(
                            empty_workspace_action(
                                "empty-open-recent",
                                IconName::FolderClosed,
                                ui_text.get(UiTextKey::OpenRecent),
                                Some("secondary-shift-o"),
                                false,
                                theme,
                                ui_style,
                                scale,
                                cx,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.on_open_project_palette(&OpenProjectPalette, window, cx);
                                },
                            )),
                        )
                        .child(
                            empty_workspace_action(
                                "empty-restore-last-session",
                                IconName::GalleryVerticalEnd,
                                ui_text.get(UiTextKey::RestoreLastSession),
                                None,
                                false,
                                theme,
                                ui_style,
                                scale,
                                cx,
                            )
                            .on_click(cx.listener(|this, _, _window, cx| {
                                this.restore_last_opened_projects();
                                cx.notify();
                            }))
                            .disabled(!can_restore_last_session)
                            .tab_stop(can_restore_last_session),
                        )
                        .child(
                            empty_workspace_action(
                                "empty-command-palette",
                                IconName::SquareTerminal,
                                ui_text.get(UiTextKey::CommandPalette),
                                Some("secondary-p"),
                                false,
                                theme,
                                ui_style,
                                scale,
                                cx,
                            )
                            .on_click(cx.listener(
                                |this, _, window, cx| {
                                    this.on_open_command_palette(&OpenCommandPalette, window, cx);
                                },
                            )),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .items_center()
                        .gap(scale.spacing(ui_style.spacing.md))
                        .w_full()
                        .child(div().h(ui_style.border.hairline).w_full().bg(theme.border))
                        .child(
                            div()
                                .text_size(scale.text_rems(0.75))
                                .text_center()
                                .text_color(theme.text_subtle)
                                .child(ui_text.get(UiTextKey::EmptySidebarNote)),
                        ),
                ),
        )
}

pub(super) fn project_empty_terminal_state(
    cx: &mut Context<WorkbenchView>,
    ui_text: &UiText,
    theme: &WorkbenchTheme,
) -> Div {
    let ui_style = current_ui_style(cx);
    div()
        .flex()
        .flex_col()
        .gap(ui_style.spacing.lg)
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
            yttt_button(
                "project-empty-new-tab",
                ui_text.get(UiTextKey::NewTab),
                YtttButtonVariant::Primary,
                *theme,
                ui_style,
                cx,
            )
            .child(Kbd::new(Keystroke::parse("secondary-t").expect(
                "new terminal shortcut should be a valid GPUI keystroke",
            )))
            .on_click(cx.listener(|this, _, _window, cx| {
                let _ = this.run_command(CommandId::TabNew);
                cx.notify();
            })),
        )
}
