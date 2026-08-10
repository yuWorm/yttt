use super::super::*;

pub(in super::super) fn settings_overlay(
    root: &mut WorkbenchView,
    search_input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let appearance = root.theme_runtime();
    let theme = appearance.ui;
    let style = yttt_settings_layout(appearance.style, window.viewport_size());

    yttt_panel_overlay(
        yttt_panel(YtttPanelKind::Settings, theme, appearance.style)
            .w(style.panel_width)
            .max_w(style.panel_width)
            .h(style.panel_height)
            .max_h(style.panel_height)
            .debug_selector(|| "settings-panel".to_string())
            .flex_row()
            .p_0()
            .overflow_hidden()
            .child(settings_sidebar(root, search_input, style, cx))
            .child(settings_content(root, style, window, cx)),
        YtttPanelKind::Settings,
        YtttOverlayPlacement::Center,
        theme,
        appearance.style,
    )
}

fn settings_sidebar(
    root: &WorkbenchView,
    search_input: &Entity<InputState>,
    style: YtttSettingsLayout,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let ui_style = style.ui_style;
    let visible_groups = root.settings.settings_page.visible_groups(&root.ui_text);
    let no_results = visible_groups.is_empty();
    let groups = visible_groups.into_iter().fold(
        div().flex().flex_col().gap(ui_style.spacing.xxs),
        |groups, group| {
            let group_id = group.id.as_str().to_string();
            let background = if group.selected {
                theme.ghost_element_selected
            } else {
                theme.ghost_element_background
            };
            let text = if group.selected {
                theme.text
            } else {
                theme.text_muted
            };

            groups.child(
                div()
                    .id(SharedString::from(format!(
                        "settings-group-{}",
                        group.id.as_str()
                    )))
                    .flex()
                    .items_center()
                    .h(ui_style.rows.sidebar_height)
                    .rounded(ui_style.radius.compact)
                    .px(ui_style.settings.nav_padding_x)
                    .bg(background)
                    .text_sm()
                    .text_color(text)
                    .hover(move |this| this.bg(theme.ghost_element_hover))
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        let _ = this.select_settings_group(&group_id);
                        cx.notify();
                    }))
                    .child(group.title),
            )
        },
    );

    div()
        .debug_selector(|| "settings-sidebar".to_string())
        .flex()
        .flex_col()
        .w(style.sidebar_width)
        .h_full()
        .min_h_0()
        .flex_none()
        .border_r(ui_style.border.hairline)
        .border_color(theme.border_variant)
        .bg(theme.panel_background)
        .p(ui_style.settings.nav_padding_x)
        .gap(ui_style.settings.nav_group_gap)
        .child(
            div()
                .id(SharedString::from("settings-search"))
                .debug_selector(|| "settings-search".to_string())
                .flex()
                .items_center()
                .h(style.search_height)
                .flex_none()
                .child(
                    yttt_input(search_input, YtttInputKind::Search, theme, ui_style)
                        .small()
                        .w_full()
                        .prefix(IconName::Search)
                        .cleanable(true),
                ),
        )
        .child(
            div().flex_1().min_h_0().child(
                groups
                    .when(no_results, |groups| {
                        groups.child(
                            div()
                                .px(ui_style.spacing.md)
                                .py(ui_style.spacing.xl)
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(root.ui_text.get(UiTextKey::SettingsNoSearchResults)),
                        )
                    })
                    .overflow_y_scrollbar(),
            ),
        )
}

fn settings_content(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let group = root.settings.settings_page.selected_group;
    let no_results = root
        .settings
        .settings_page
        .visible_groups(&root.ui_text)
        .is_empty();
    let (title, description) = if no_results {
        (
            root.ui_text.get(UiTextKey::SettingsNoSearchResults),
            root.ui_text
                .get(UiTextKey::SettingsNoSearchResultsDescription),
        )
    } else {
        (group.title(&root.ui_text), group.description(&root.ui_text))
    };
    let rows = if no_results {
        div()
            .debug_selector(|| "settings-no-results".to_string())
            .flex()
            .flex_col()
            .items_center()
            .justify_center()
            .gap(style.ui_style.spacing.md)
            .size_full()
            .text_center()
            .child(
                Icon::new(IconName::Search)
                    .size_6()
                    .text_color(theme.text_muted),
            )
            .child(
                div()
                    .text_base()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(title),
            )
            .child(
                div()
                    .max_w_96()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(description),
            )
    } else {
        settings_rows(root, group, style, window, cx)
    };
    let virtualized_keybindings = !no_results && group == SettingsGroupId::Keybindings;
    let rows = rows.px(style.content_padding_x).py(style.content_padding_y);
    let rows = if virtualized_keybindings {
        rows.overflow_hidden().into_any_element()
    } else {
        rows.overflow_y_scrollbar().into_any_element()
    };
    div()
        .debug_selector(|| "settings-content".to_string())
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .bg(theme.editor_background)
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .border_b(style.ui_style.border.hairline)
                .border_color(theme.border_variant)
                .px(style.content_padding_x)
                .py(style.content_padding_y)
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(style.ui_style.spacing.xs)
                        .child(
                            div()
                                .text_base()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(title),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(description),
                        ),
                )
                .child(yttt_icon_button(
                    "settings-close",
                    IconName::Close,
                    YtttIconButtonKind::OverlayClose,
                    theme,
                    style.ui_style,
                    cx.listener(|this, _, _window, cx| {
                        this.close_settings();
                        cx.notify();
                    }),
                )),
        )
        .child(div().flex_1().min_h_0().child(rows))
}

fn settings_rows(
    root: &mut WorkbenchView,
    group: SettingsGroupId,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    match group {
        SettingsGroupId::General => settings_general_rows(root, style, window, cx),
        SettingsGroupId::Appearance => settings_appearance_rows(root, style, window, cx),
        SettingsGroupId::Languages => settings_language_rows(root, style, window, cx),
        SettingsGroupId::Editor => settings_editor_rows(root, style, window, cx),
        SettingsGroupId::Terminal => settings_terminal_rows(root, style, window, cx),
        SettingsGroupId::Agent => settings_agent_rows(root, style, cx),
        SettingsGroupId::DefaultLayout => settings_default_layout_rows(root, style, cx),
        SettingsGroupId::Keybindings => settings_keybinding_rows(root, style, window, cx),
    }
}

fn settings_general_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let language_select = root.settings_language_select(window, cx);
    let vim_mode_select = root.settings_vim_mode_select(window, cx);
    let command_input = root.settings_new_tab_command_input(window, cx);
    let command_input_for_add = command_input.clone();
    let command_add_control = div()
        .flex()
        .items_center()
        .gap(style.ui_style.spacing.md)
        .w_full()
        .max_w(px(720.0))
        .child(
            div().flex_1().min_w_0().h(style.control_height).child(
                yttt_input(
                    &command_input,
                    YtttInputKind::Settings,
                    theme,
                    style.ui_style,
                )
                .small(),
            ),
        )
        .child(
            settings_button(
                "settings-add-new-tab-command",
                text.get(UiTextKey::SettingsAddCommand),
                false,
                theme,
                cx,
                cx.listener(move |this, _, _window, cx| {
                    let command = command_input_for_add.read(cx).value().to_string();
                    if let Err(error) = this.add_new_tab_command(&command) {
                        this.load_error = Some(error.to_string());
                    }
                    cx.notify();
                }),
            )
            .debug_selector(|| "settings-add-new-tab-command".to_string()),
        );
    let command_list = root
        .new_tab_commands()
        .to_vec()
        .into_iter()
        .enumerate()
        .fold(
            div()
                .flex()
                .flex_col()
                .gap(style.ui_style.spacing.md)
                .w_full()
                .max_w(px(720.0))
                .child(command_add_control),
            |list, (index, command)| {
                list.child(
                    div()
                        .flex()
                        .items_center()
                        .gap(style.ui_style.spacing.md)
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .overflow_hidden()
                                .text_ellipsis()
                                .whitespace_nowrap()
                                .text_xs()
                                .text_color(theme.text)
                                .child(command),
                        )
                        .child(settings_button(
                            format!("settings-delete-new-tab-command-{index}"),
                            text.get(UiTextKey::SettingsDelete),
                            false,
                            theme,
                            cx,
                            cx.listener(move |this, _, _window, cx| {
                                if let Err(error) = this.remove_new_tab_command(index) {
                                    this.load_error = Some(error.to_string());
                                }
                                cx.notify();
                            }),
                        )),
                )
            },
        );
    let update_status = root.update_status().clone();
    let update_checking = matches!(update_status, UpdateStatus::Checking);
    let update_status_label = match &update_status {
        UpdateStatus::Idle => format!("v{}", root.current_app_version()),
        UpdateStatus::Checking => text.get(UiTextKey::SettingsCheckingForUpdates).to_string(),
        UpdateStatus::UpToDate => format!(
            "v{} · {}",
            root.current_app_version(),
            text.get(UiTextKey::SettingsUpToDate)
        ),
        UpdateStatus::Available(update) => {
            format!("v{} → v{}", root.current_app_version(), update.version)
        }
        UpdateStatus::Failed(_) => format!(
            "v{} · {}",
            root.current_app_version(),
            text.get(UiTextKey::SettingsUpdateCheckFailed)
        ),
    };
    let update_download_url = root.available_update_url().map(str::to_string);
    let update_control = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(style.ui_style.spacing.md)
        .w(style.control_width)
        .child(settings_value(update_status_label, theme, style.ui_style))
        .child(
            settings_button(
                "settings-check-for-updates",
                if update_checking {
                    text.get(UiTextKey::SettingsCheckingForUpdates)
                } else {
                    text.get(UiTextKey::SettingsCheckForUpdates)
                },
                false,
                theme,
                cx,
                cx.listener(|this, _, window, cx| {
                    this.check_for_updates(window, cx);
                }),
            )
            .loading(update_checking)
            .disabled(update_checking)
            .debug_selector(|| "settings-check-for-updates".to_string()),
        )
        .when_some(update_download_url, |this, url| {
            this.child(
                settings_button(
                    "settings-download-update",
                    text.get(UiTextKey::SettingsDownloadUpdate),
                    true,
                    theme,
                    cx,
                    move |_, _, cx| cx.open_url(&url),
                )
                .debug_selector(|| "settings-download-update".to_string()),
            )
        });

    div()
        .flex()
        .flex_col()
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionApplicationInteraction),
            true,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguage),
            text.get(UiTextKey::SettingsLanguageDescription),
            settings_select_control(
                language_select,
                theme,
                style.ui_style,
                false,
                text.get(UiTextKey::SettingsSelectLanguage),
            )
            .into_any_element(),
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsVimMode),
                text.get(UiTextKey::SettingsVimModeDescription),
                settings_select_control(
                    vim_mode_select,
                    theme,
                    style.ui_style,
                    false,
                    text.get(UiTextKey::SettingsVimMode),
                )
                .into_any_element(),
            )
            .id("settings-vim-mode-row")
            .debug_selector(|| "settings-vim-mode-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionStartupNotifications),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsSystemNotifications),
            text.get(UiTextKey::SettingsSystemNotificationsDescription),
            settings_switch(
                "settings-notifications",
                root.system_notifications_enabled,
                theme,
                style.ui_style,
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_system_notifications_enabled(*checked);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsRestoreLastSession),
                text.get(UiTextKey::SettingsRestoreLastSessionDescription),
                settings_switch(
                    "settings-restore-last-session",
                    root.restore_last_session_enabled(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_restore_last_session_enabled(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .debug_selector(|| "settings-restore-last-session".to_string())
                .into_any_element(),
            )
            .debug_selector(|| "settings-restore-last-session-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionPerformance),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsPerformanceMetrics),
                text.get(UiTextKey::SettingsPerformanceMetricsDescription),
                settings_switch(
                    "settings-performance-metrics",
                    root.performance_metrics_enabled(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_performance_metrics_enabled(*checked, cx) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .debug_selector(|| "settings-performance-metrics".to_string())
                .into_any_element(),
            )
            .debug_selector(|| "settings-performance-metrics-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsSystemPerformanceMetrics),
                text.get(UiTextKey::SettingsSystemPerformanceMetricsDescription),
                settings_switch(
                    "settings-system-performance-metrics",
                    root.system_performance_metrics_enabled(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) =
                            this.set_system_performance_metrics_enabled(*checked, cx)
                        {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .debug_selector(|| "settings-system-performance-metrics".to_string())
                .into_any_element(),
            )
            .debug_selector(|| "settings-system-performance-metrics-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionNewTabs),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsNewTabCommandPicker),
                text.get(UiTextKey::SettingsNewTabCommandPickerDescription),
                settings_switch(
                    "settings-new-tab-command-picker",
                    root.new_tab_command_picker_enabled(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_new_tab_command_picker_enabled(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .debug_selector(|| "settings-new-tab-command-picker".to_string())
                .into_any_element(),
            )
            .debug_selector(|| "settings-new-tab-command-picker-row".to_string()),
        )
        .child(
            setting_block(
                style,
                theme,
                text.get(UiTextKey::SettingsNewTabCommands),
                text.get(UiTextKey::SettingsNewTabCommandsDescription),
                command_list.into_any_element(),
            )
            .debug_selector(|| "settings-new-tab-commands-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionConnections),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SshConnections),
                text.get(UiTextKey::SshConnectionsDescription),
                settings_button(
                    "settings-open-ssh-connections",
                    text.get(UiTextKey::SettingsOpen),
                    false,
                    theme,
                    cx,
                    cx.listener(|this, _, _window, cx| {
                        this.open_ssh_connection_manager();
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-ssh-connections-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionUpdates),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsUpdates),
                text.get(UiTextKey::SettingsUpdatesDescription),
                update_control.into_any_element(),
            )
            .debug_selector(|| "settings-updates-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsAutoCheckUpdates),
                text.get(UiTextKey::SettingsAutoCheckUpdatesDescription),
                settings_switch(
                    "settings-auto-check-updates",
                    root.auto_check_updates_enabled(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, window, cx| {
                        if let Err(error) = this.set_auto_check_updates_enabled(*checked) {
                            this.load_error = Some(error.to_string());
                        } else if *checked {
                            this.start_update_check(window, cx);
                        }
                        cx.notify();
                    }),
                )
                .debug_selector(|| "settings-auto-check-updates".to_string())
                .into_any_element(),
            )
            .debug_selector(|| "settings-auto-check-updates-row".to_string()),
        )
}

fn settings_appearance_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let window_effect_select = root.settings_window_effect_select(window, cx);
    let window_opacity_input =
        root.settings_number_input(SettingsNumberField::WindowOpacity, window, cx);
    let ui_font_select = root.settings_ui_font_family_select(window, cx);
    let ui_font_size_input =
        root.settings_number_input(SettingsNumberField::UiFontSize, window, cx);
    let ui_line_height_input =
        root.settings_number_input(SettingsNumberField::UiLineHeight, window, cx);
    let ui_theme_select = root.settings_ui_theme_select(window, cx);
    let ui_style_select = root.settings_ui_style_select(window, cx);
    let terminal_theme_select = root.settings_terminal_theme_select(window, cx);
    let icon_theme_select = root.settings_icon_theme_select(window, cx);

    div()
        .flex()
        .flex_col()
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionWindow),
            true,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsWindowEffect),
                text.get(UiTextKey::SettingsWindowEffectDescription),
                settings_select_control(window_effect_select, theme, style.ui_style, false, "")
                    .into_any_element(),
            )
            .debug_selector(|| "settings-window-effect-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsWindowOpacity),
                text.get(UiTextKey::SettingsWindowOpacityDescription),
                settings_number_control(window_opacity_input, theme, style).into_any_element(),
            )
            .debug_selector(|| "settings-window-opacity-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionInterface),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsUiFontFamily),
                text.get(UiTextKey::SettingsUiFontFamilyDescription),
                settings_select_control(
                    ui_font_select,
                    theme,
                    style.ui_style,
                    true,
                    text.get(UiTextKey::SettingsSearchFont),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-ui-font-family-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsUiFontSize),
                text.get(UiTextKey::SettingsUiFontSizeDescription),
                settings_number_control(ui_font_size_input, theme, style).into_any_element(),
            )
            .debug_selector(|| "settings-ui-font-size-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsUiLineHeight),
                text.get(UiTextKey::SettingsUiLineHeightDescription),
                settings_number_control(ui_line_height_input, theme, style).into_any_element(),
            )
            .debug_selector(|| "settings-ui-line-height-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionThemes),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsUiTheme),
            text.get(UiTextKey::SettingsUiThemeDescription),
            settings_select_control(
                ui_theme_select,
                theme,
                style.ui_style,
                true,
                text.get(UiTextKey::SettingsSearchTheme),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsUiStyle),
            text.get(UiTextKey::SettingsUiStyleDescription),
            settings_select_control(ui_style_select, theme, style.ui_style, false, "")
                .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsIconTheme),
            text.get(UiTextKey::SettingsIconThemeDescription),
            settings_select_control(
                icon_theme_select,
                theme,
                style.ui_style,
                true,
                text.get(UiTextKey::SettingsSearchTheme),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsTerminalTheme),
            text.get(UiTextKey::SettingsTerminalThemeDescription),
            settings_select_control(
                terminal_theme_select,
                theme,
                style.ui_style,
                true,
                text.get(UiTextKey::SettingsSearchTheme),
            )
            .into_any_element(),
        ))
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionAdvanced),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsEditSettingsToml),
            text.get(UiTextKey::SettingsEditSettingsTomlDescription),
            settings_button(
                "settings-open-file",
                text.get(UiTextKey::SettingsShowPath),
                false,
                theme,
                cx,
                cx.listener(move |this, _, window, cx| {
                    this.show_settings_file_path_status();
                    this.flush_pending_status_notifications(window, cx);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsThemesDirectory),
            text.get(UiTextKey::SettingsThemesDirectoryDescription),
            settings_button(
                "settings-open-themes-dir",
                text.get(UiTextKey::SettingsShowPath),
                false,
                theme,
                cx,
                cx.listener(move |this, _, window, cx| {
                    this.show_themes_directory_status();
                    this.flush_pending_status_notifications(window, cx);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsImportZedThemes),
            text.get(UiTextKey::SettingsImportZedThemesDescription),
            settings_button(
                "settings-import-zed-themes",
                text.get(UiTextKey::SettingsImportZedThemesAction),
                true,
                theme,
                cx,
                cx.listener(move |this, _, window, cx| {
                    if let Err(error) = this.open_zed_theme_import_dialog() {
                        this.load_error = Some(error);
                    }
                    this.flush_pending_status_notifications(window, cx);
                    cx.notify();
                }),
            )
            .debug_selector(|| "settings-import-zed-themes".to_string())
            .into_any_element(),
        ))
}

fn settings_language_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let default_language_select = root.settings_editor_language_select(window, cx);
    let supported_language_count = root.available_editor_language_names().len();
    let lsp_command = if root.editor_lsp_command().is_empty() {
        text.get(UiTextKey::SettingsUnbound).to_string()
    } else {
        root.editor_lsp_command().to_string()
    };

    div()
        .flex()
        .flex_col()
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionDetectionDefaults),
            true,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguageDetection),
            text.get(UiTextKey::SettingsLanguageDetectionDescription),
            settings_switch(
                "settings-editor-auto-detect-language",
                root.editor_auto_detect_language(),
                theme,
                style.ui_style,
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_editor_auto_detect_language(*checked);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsDefaultCodeLanguage),
            text.get(UiTextKey::SettingsDefaultCodeLanguageDescription),
            settings_select_control(
                default_language_select,
                theme,
                style.ui_style,
                true,
                text.get(UiTextKey::SettingsSearchCodeLanguage),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsSupportedLanguages),
            text.get(UiTextKey::SettingsSupportedLanguagesDescription),
            settings_value(supported_language_count.to_string(), theme, style.ui_style)
                .into_any_element(),
        ))
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionLanguageServices),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguageServer),
            text.get(UiTextKey::SettingsLanguageServerDescription),
            settings_switch(
                "settings-editor-lsp-enabled",
                root.editor_lsp_enabled(),
                theme,
                style.ui_style,
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_editor_lsp_enabled(*checked);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguageServerCommand),
            text.get(UiTextKey::SettingsLanguageServerCommandDescription),
            settings_value(lsp_command, theme, style.ui_style).into_any_element(),
        ))
}

fn settings_editor_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let font_select = root.settings_editor_font_family_select(window, cx);
    let autosave_select = root.settings_editor_autosave_select(window, cx);
    let font_size_input =
        root.settings_number_input(SettingsNumberField::EditorFontSize, window, cx);
    let line_height_input =
        root.settings_number_input(SettingsNumberField::EditorLineHeight, window, cx);
    let tab_size_input = root.settings_number_input(SettingsNumberField::EditorTabSize, window, cx);
    let autosave_delay_input =
        root.settings_number_input(SettingsNumberField::EditorAutosaveDelay, window, cx);
    let project_panel_width_input =
        root.settings_number_input(SettingsNumberField::ProjectPanelWidth, window, cx);
    let project_sidebar_width_input =
        root.settings_number_input(SettingsNumberField::ProjectSidebarWidth, window, cx);

    div()
        .flex()
        .flex_col()
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionTypography),
            true,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorFontFamily),
                text.get(UiTextKey::SettingsEditorFontFamilyDescription),
                settings_select_control(
                    font_select,
                    theme,
                    style.ui_style,
                    true,
                    text.get(UiTextKey::SettingsSearchFont),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-editor-font-family-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorFontSize),
                text.get(UiTextKey::SettingsEditorFontSizeDescription),
                settings_number_control(font_size_input, theme, style).into_any_element(),
            )
            .debug_selector(|| "settings-editor-font-size-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorLineHeight),
                text.get(UiTextKey::SettingsEditorLineHeightDescription),
                settings_number_control(line_height_input, theme, style).into_any_element(),
            )
            .debug_selector(|| "settings-editor-line-height-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionEditingBehavior),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorTabSize),
                text.get(UiTextKey::SettingsEditorTabSizeDescription),
                settings_number_control(tab_size_input, theme, style).into_any_element(),
            )
            .debug_selector(|| "settings-editor-tab-size-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorSoftWrap),
                text.get(UiTextKey::SettingsEditorSoftWrapDescription),
                settings_switch(
                    "settings-editor-soft-wrap",
                    root.app_settings.editor.soft_wrap,
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, window, cx| {
                        if let Err(error) = this.set_editor_soft_wrap(*checked, window, cx) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-editor-soft-wrap-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorLineNumbers),
                text.get(UiTextKey::SettingsEditorLineNumbersDescription),
                settings_switch(
                    "settings-editor-line-numbers",
                    root.app_settings.editor.line_numbers,
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, window, cx| {
                        if let Err(error) = this.set_editor_line_numbers(*checked, window, cx) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-editor-line-numbers-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionSaving),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorAutosave),
                text.get(UiTextKey::SettingsEditorAutosaveDescription),
                settings_select_control(
                    autosave_select,
                    theme,
                    style.ui_style,
                    false,
                    text.get(UiTextKey::SettingsEditorAutosave),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-editor-autosave-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorAutosaveDelay),
                text.get(UiTextKey::SettingsEditorAutosaveDelayDescription),
                settings_number_control(autosave_delay_input, theme, style).into_any_element(),
            )
            .debug_selector(|| "settings-editor-autosave-delay-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionProjectPanels),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsProjectPanelDefaultOpen),
                text.get(UiTextKey::SettingsProjectPanelDefaultOpenDescription),
                settings_switch(
                    "settings-project-panel-default-open",
                    root.app_settings.project_panel.default_open,
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_project_panel_default_open(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-project-panel-default-open-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsProjectPanelShowHidden),
                text.get(UiTextKey::SettingsProjectPanelShowHiddenDescription),
                settings_switch(
                    "settings-project-panel-show-hidden",
                    root.app_settings.project_panel.show_hidden,
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_project_panel_show_hidden(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-project-panel-show-hidden-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsProjectPanelWidth),
                text.get(UiTextKey::SettingsProjectPanelWidthDescription),
                settings_number_control(project_panel_width_input, theme, style).into_any_element(),
            )
            .debug_selector(|| "settings-project-panel-width-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsProjectSidebarWidth),
                text.get(UiTextKey::SettingsProjectSidebarWidthDescription),
                settings_number_control(project_sidebar_width_input, theme, style)
                    .into_any_element(),
            )
            .debug_selector(|| "settings-project-sidebar-width-row".to_string()),
        )
}

fn settings_terminal_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let shell_select = root.settings_shell_select(window, cx);
    let custom_shell_input = root.settings_custom_shell_input(window, cx);
    let font_select = root.settings_font_family_select(window, cx);
    let font_size_input = root.settings_number_input(SettingsNumberField::FontSize, window, cx);
    let line_height_input = root.settings_number_input(SettingsNumberField::LineHeight, window, cx);
    let padding_input = root.settings_number_input(SettingsNumberField::Padding, window, cx);
    let scrollback_input = root.settings_number_input(SettingsNumberField::Scrollback, window, cx);
    let cursor_shape_select = root.settings_terminal_cursor_shape_select(window, cx);
    let osc52_policy_select = root.settings_terminal_osc52_policy_select(window, cx);
    let custom_shell_input_for_add = custom_shell_input.clone();
    let custom_shell_control = div()
        .flex()
        .items_center()
        .gap(style.ui_style.spacing.md)
        .w_full()
        .max_w(px(640.0))
        .child(
            div().flex_1().min_w_0().h(style.control_height).child(
                yttt_input(
                    &custom_shell_input,
                    YtttInputKind::Settings,
                    theme,
                    style.ui_style,
                )
                .small(),
            ),
        )
        .child(settings_button(
            "settings-add-custom-shell",
            text.get(UiTextKey::SettingsAddShell),
            false,
            theme,
            cx,
            cx.listener(move |this, _, _window, cx| {
                let shell = custom_shell_input_for_add.read(cx).value().to_string();
                if let Err(error) = this.add_custom_terminal_shell(&shell) {
                    this.load_error = Some(error.to_string());
                }
                cx.notify();
            }),
        ));
    let environment_name_input = root.settings_environment_name_input(window, cx);
    let environment_value_input = root.settings_environment_value_input(window, cx);
    let environment_name_input_for_set = environment_name_input.clone();
    let environment_value_input_for_set = environment_value_input.clone();
    let environment_set_control = div()
        .flex()
        .items_center()
        .gap(style.ui_style.spacing.md)
        .w_full()
        .max_w(px(720.0))
        .child(
            div().flex_1().min_w_0().h(style.control_height).child(
                yttt_input(
                    &environment_name_input,
                    YtttInputKind::Settings,
                    theme,
                    style.ui_style,
                )
                .small(),
            ),
        )
        .child(
            div().flex_1().min_w_0().h(style.control_height).child(
                yttt_input(
                    &environment_value_input,
                    YtttInputKind::Settings,
                    theme,
                    style.ui_style,
                )
                .small(),
            ),
        )
        .child(settings_button(
            "settings-set-environment-variable",
            text.get(UiTextKey::SettingsSetEnvironmentVariable),
            false,
            theme,
            cx,
            cx.listener(move |this, _, _window, cx| {
                let name = environment_name_input_for_set.read(cx).value().to_string();
                let value = environment_value_input_for_set.read(cx).value().to_string();
                if let Err(error) = this.set_terminal_environment_variable(&name, &value) {
                    this.load_error = Some(error.to_string());
                }
                cx.notify();
            }),
        ));
    let environment_variables = root
        .terminal_environment()
        .iter()
        .map(|(name, value)| (name.clone(), value.clone()))
        .collect::<Vec<_>>();
    let environment_list = environment_variables.into_iter().enumerate().fold(
        div()
            .flex()
            .flex_col()
            .gap(style.ui_style.spacing.md)
            .w_full()
            .max_w(px(720.0))
            .child(environment_set_control),
        |list, (index, (name, value))| {
            let name_for_remove = name.clone();
            list.child(
                div()
                    .flex()
                    .items_center()
                    .gap(style.ui_style.spacing.md)
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .overflow_hidden()
                            .text_ellipsis()
                            .whitespace_nowrap()
                            .text_xs()
                            .text_color(theme.text)
                            .child(format!("{name}={value}")),
                    )
                    .child(settings_button(
                        format!("settings-delete-environment-variable-{index}"),
                        text.get(UiTextKey::SettingsDelete),
                        false,
                        theme,
                        cx,
                        cx.listener(move |this, _, _window, cx| {
                            if let Err(error) =
                                this.remove_terminal_environment_variable(&name_for_remove)
                            {
                                this.load_error = Some(error.to_string());
                            }
                            cx.notify();
                        }),
                    )),
            )
        },
    );

    div()
        .flex()
        .flex_col()
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionShell),
            true,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsDefaultShell),
            text.get(UiTextKey::SettingsDefaultShellDescription),
            settings_select_control(
                shell_select,
                theme,
                style.ui_style,
                false,
                text.get(UiTextKey::SettingsSelectShell),
            )
            .into_any_element(),
        ))
        .child(setting_block(
            style,
            theme,
            text.get(UiTextKey::SettingsCustomShell),
            text.get(UiTextKey::SettingsCustomShellDescription),
            custom_shell_control.into_any_element(),
        ))
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionEnvironment),
            false,
        ))
        .child(
            setting_block(
                style,
                theme,
                text.get(UiTextKey::SettingsEnvironmentVariables),
                text.get(UiTextKey::SettingsEnvironmentVariablesDescription),
                environment_list.into_any_element(),
            )
            .debug_selector(|| "settings-terminal-environment-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionTypography),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsFontFamily),
            text.get(UiTextKey::SettingsFontFamilyDescription),
            settings_select_control(
                font_select,
                theme,
                style.ui_style,
                true,
                text.get(UiTextKey::SettingsSearchFont),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsFontSize),
            text.get(UiTextKey::SettingsFontSizeDescription),
            settings_number_control(font_size_input, theme, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLineHeight),
            text.get(UiTextKey::SettingsLineHeightDescription),
            settings_number_control(line_height_input, theme, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsPadding),
            text.get(UiTextKey::SettingsPaddingDescription),
            settings_number_control(padding_input, theme, style).into_any_element(),
        ))
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionScrolling),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsScrollback),
            text.get(UiTextKey::SettingsScrollbackDescription),
            settings_number_control(scrollback_input, theme, style).into_any_element(),
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsScrollbar),
                text.get(UiTextKey::SettingsScrollbarDescription),
                settings_switch(
                    "settings-show-scrollbar",
                    root.terminal_show_scrollbar(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_terminal_show_scrollbar(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        this.sync_terminal_pane_configs(cx);
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-terminal-scrollbar-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionCursorMouse),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsTerminalCursorShape),
                text.get(UiTextKey::SettingsTerminalCursorShapeDescription),
                settings_select_control(
                    cursor_shape_select,
                    theme,
                    style.ui_style,
                    false,
                    text.get(UiTextKey::SettingsTerminalCursorShape),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-terminal-cursor-shape-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsTerminalCursorBlinking),
                text.get(UiTextKey::SettingsTerminalCursorBlinkingDescription),
                settings_switch(
                    "settings-terminal-cursor-blinking",
                    root.terminal_cursor_blinking(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_terminal_cursor_blinking(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        this.sync_terminal_pane_configs(cx);
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-terminal-cursor-blinking-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsTerminalHideMouseWhenTyping),
                text.get(UiTextKey::SettingsTerminalHideMouseWhenTypingDescription),
                settings_switch(
                    "settings-terminal-hide-mouse-when-typing",
                    root.terminal_hide_mouse_when_typing(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_terminal_hide_mouse_when_typing(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        this.sync_terminal_pane_configs(cx);
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-terminal-hide-mouse-when-typing-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsTerminalCopyOnSelect),
                text.get(UiTextKey::SettingsTerminalCopyOnSelectDescription),
                settings_switch(
                    "settings-terminal-copy-on-select",
                    root.terminal_copy_on_select(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_terminal_copy_on_select(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        this.sync_terminal_pane_configs(cx);
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-terminal-copy-on-select-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionCompatibility),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsTerminalOsc52Policy),
                text.get(UiTextKey::SettingsTerminalOsc52PolicyDescription),
                settings_select_control(
                    osc52_policy_select,
                    theme,
                    style.ui_style,
                    false,
                    text.get(UiTextKey::SettingsTerminalOsc52Policy),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-terminal-osc52-policy-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsTerminalKittyKeyboard),
                text.get(UiTextKey::SettingsTerminalKittyKeyboardDescription),
                settings_switch(
                    "settings-terminal-kitty-keyboard",
                    root.terminal_kitty_keyboard(),
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_terminal_kitty_keyboard(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        this.sync_terminal_pane_configs(cx);
                        cx.notify();
                    }),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-terminal-kitty-keyboard-row".to_string()),
        )
}

fn settings_agent_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let primary = root.primary_agent();
    let sessions_enabled = root.agent_sessions_enabled();
    let additional_agent_rows = BuiltinAgent::ALL
        .into_iter()
        .filter(|agent| *agent != primary)
        .map(|agent| {
            let provider_id = agent.id();
            setting_row(
                style,
                theme,
                agent.display_name(),
                text.get(UiTextKey::SettingsAgentSessionProviderDescription),
                settings_switch(
                    format!("settings-agent-session-provider-{provider_id}"),
                    root.agent_session_agent_enabled(agent),
                    theme,
                    style.ui_style,
                    cx.listener(move |this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_agent_session_agent_enabled(agent, *checked) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .debug_selector(move || format!("settings-agent-session-provider-{provider_id}"))
                .into_any_element(),
            )
            .debug_selector(move || format!("settings-agent-session-provider-{provider_id}-row"))
        })
        .collect::<Vec<_>>();

    div()
        .flex()
        .flex_col()
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionAgentOverview),
            true,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsAgentPrimary),
                text.get(UiTextKey::SettingsAgentPrimaryDescription),
                settings_value(primary.display_name(), theme, style.ui_style)
                    .debug_selector(|| "settings-agent-primary".to_string())
                    .into_any_element(),
            )
            .debug_selector(|| "settings-agent-primary-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsAgentSessions),
                text.get(UiTextKey::SettingsAgentSessionsDescription),
                settings_switch(
                    "settings-agent-sessions",
                    sessions_enabled,
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_agent_sessions_enabled(*checked) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .debug_selector(|| "settings-agent-sessions".to_string())
                .into_any_element(),
            )
            .debug_selector(|| "settings-agent-sessions-row".to_string()),
        )
        .when(sessions_enabled, |settings| {
            settings
                .child(settings_section_header(
                    style,
                    theme,
                    text.get(UiTextKey::SettingsSectionProviders),
                    false,
                ))
                .children(additional_agent_rows)
        })
}

fn settings_default_layout_rows(
    root: &WorkbenchView,
    style: YtttSettingsLayout,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let path = root.default_layout_state.path().display().to_string();

    div()
        .flex()
        .flex_col()
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionCurrentLayout),
            true,
        ))
        .child(setting_block(
            style,
            theme,
            text.get(UiTextKey::SettingsDefaultLayoutPath),
            text.get(UiTextKey::SettingsDefaultLayoutPathDescription),
            settings_value(path, theme, style.ui_style).into_any_element(),
        ))
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionLayoutActions),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsEditDefaultLayout),
            text.get(UiTextKey::SettingsEditDefaultLayoutDescription),
            settings_command_button(
                "settings-default-layout-edit",
                text.get(UiTextKey::SettingsEdit),
                true,
                theme,
                CommandId::LayoutDefaultEdit,
                cx,
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsReloadDefaultLayout),
            text.get(UiTextKey::SettingsReloadDefaultLayoutDescription),
            settings_command_button(
                "settings-default-layout-reload",
                text.get(UiTextKey::SettingsOpen),
                true,
                theme,
                CommandId::LayoutDefaultReload,
                cx,
            )
            .into_any_element(),
        ))
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionDangerZone),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsResetDefaultLayout),
            text.get(UiTextKey::SettingsResetDefaultLayoutDescription),
            settings_command_button(
                "settings-default-layout-reset",
                text.get(UiTextKey::SettingsReset),
                true,
                theme,
                CommandId::LayoutDefaultReset,
                cx,
            )
            .into_any_element(),
        ))
}

fn settings_keybinding_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let diagnostics = if root.settings.keybinding_warning_lines.is_empty() {
        text.get(UiTextKey::SettingsNoKeybindingConflicts)
            .to_string()
    } else {
        root.settings.keybinding_warning_lines.join("; ")
    };
    let leader = root.keybinding_leader().to_string();
    if root.settings.keybinding_rows_cache.is_none() {
        root.settings.keybinding_rows_cache = Some(Rc::new(
            root.settings.keybindings_editor.rows_with_text(&text),
        ));
    }
    let keybinding_rows = root
        .settings
        .keybinding_rows_cache
        .as_ref()
        .expect("keybinding rows cache should be initialized")
        .clone();
    let row_height_rems =
        style.ui_style.rows.settings_height.0 + if style.stack_rows { 2.5 } else { 0.0 };
    let row_height = px(f32::from(window.rem_size()) * row_height_rems);
    let item_sizes = Rc::new(vec![gpui::size(px(0.0), row_height); keybinding_rows.len()]);
    let list_rows = keybinding_rows.clone();
    let virtual_list = v_virtual_list(
        cx.entity(),
        "settings-keybinding-virtual-list",
        item_sizes,
        move |_root, visible_range, _window, cx| {
            visible_range
                .filter_map(|index| list_rows.get(index))
                .map(|row| settings_keybinding_row(row, style, theme, text, cx))
                .collect::<Vec<_>>()
        },
    );

    div()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionConfiguration),
            true,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsEditKeybindingsToml),
            text.get(UiTextKey::SettingsEditKeybindingsTomlDescription),
            settings_command_button(
                "settings-keybindings-open",
                text.get(UiTextKey::SettingsOpen),
                true,
                theme,
                CommandId::SettingsKeybindings,
                cx,
            )
            .into_any_element(),
        ))
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionVim),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsVimQuickStart),
                text.get(UiTextKey::SettingsVimQuickStartDescription),
                settings_value("Ctrl-W · gt · j/k · i/Esc", theme, style.ui_style)
                    .into_any_element(),
            )
            .id("settings-vim-quick-start-row")
            .debug_selector(|| "settings-vim-quick-start-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsVimLeader),
                text.get(UiTextKey::SettingsVimLeaderDescription),
                settings_keybinding_value(
                    vec![leader],
                    text.get(UiTextKey::SettingsUnbound),
                    theme,
                    style.ui_style,
                )
                .into_any_element(),
            )
            .id("settings-vim-leader-row")
            .debug_selector(|| "settings-vim-leader-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionDiagnostics),
            false,
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsKeybindingDiagnostics),
            text.get(UiTextKey::SettingsKeybindingDiagnosticsDescription),
            settings_value(diagnostics, theme, style.ui_style).into_any_element(),
        ))
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionBindings),
            false,
        ))
        .child(
            div()
                .debug_selector(|| "settings-keybinding-virtual-list".to_string())
                .flex_1()
                .min_h_0()
                .overflow_hidden()
                .child(virtual_list),
        )
}

fn settings_keybinding_row(
    row: &KeybindingRow,
    style: YtttSettingsLayout,
    theme: WorkbenchTheme,
    text: UiText,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let command = row.command;
    let row_id = row.command_id;
    let title_text = if row.has_conflict {
        format!("{} ({})", row.title, text.get(UiTextKey::SettingsConflict))
    } else {
        row.title.to_string()
    };
    let label = div()
        .flex()
        .flex_col()
        .gap(style.ui_style.spacing.xs)
        .min_w_0()
        .flex_1()
        .child(
            div()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(title_text),
        )
        .child(
            div()
                .truncate()
                .text_xs()
                .text_color(theme.text_muted)
                .child(row.description),
        );
    let actions = div()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_end()
        .gap(style.ui_style.spacing.xs)
        .flex_none()
        .child(settings_keybinding_value(
            row.display_keys(),
            text.get(UiTextKey::SettingsUnbound),
            theme,
            style.ui_style,
        ))
        .child(settings_button(
            format!("settings-keybinding-edit-{row_id}"),
            text.get(UiTextKey::SettingsEdit),
            false,
            theme,
            cx,
            cx.listener(move |this, _, _window, cx| {
                let _ = this.open_keybinding_action_edit_dialog(command);
                cx.notify();
            }),
        ))
        .child(settings_button(
            format!("settings-keybinding-reset-{row_id}"),
            text.get(UiTextKey::SettingsReset),
            false,
            theme,
            cx,
            cx.listener(move |this, _, _window, cx| {
                let _ = this.reset_keybinding_action_keys(command);
                cx.notify();
            }),
        ))
        .child(settings_button(
            format!("settings-keybinding-delete-{row_id}"),
            text.get(UiTextKey::SettingsDelete),
            false,
            theme,
            cx,
            cx.listener(move |this, _, _window, cx| {
                let _ = this.delete_keybinding_action_keys(command);
                cx.notify();
            }),
        ));
    let binding_row = yttt_row(
        YtttRowKind::Settings,
        SelectableState::Inactive,
        true,
        theme,
        style.ui_style,
    )
    .h_full()
    .flex();
    let binding_row = if style.stack_rows {
        binding_row
            .flex_col()
            .items_start()
            .gap(style.ui_style.spacing.lg)
            .child(label)
            .child(actions.w_full().justify_start())
    } else {
        binding_row
            .items_center()
            .justify_between()
            .gap(style.ui_style.spacing.xl)
            .child(label)
            .child(actions)
    };

    binding_row
        .debug_selector(move || format!("settings-keybinding-row-{row_id}"))
        .border_b(style.ui_style.border.hairline)
        .border_color(theme.border_variant.alpha(0.65))
}

fn setting_row(
    style: YtttSettingsLayout,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    yttt_settings_row(
        style.control_width,
        style.stack_rows,
        theme,
        style.ui_style,
        title,
        description,
        control,
    )
    .border_b(style.ui_style.border.hairline)
    .border_color(theme.border_variant.alpha(0.65))
}

fn setting_block(
    style: YtttSettingsLayout,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    yttt_settings_row(
        style.control_width,
        true,
        theme,
        style.ui_style,
        title,
        description,
        control,
    )
    .border_b(style.ui_style.border.hairline)
    .border_color(theme.border_variant.alpha(0.65))
}

fn settings_section_header(
    style: YtttSettingsLayout,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    first: bool,
) -> Div {
    div()
        .flex()
        .items_center()
        .gap(style.ui_style.spacing.md)
        .when(!first, |this| this.pt(style.section_gap))
        .pb(style.ui_style.spacing.sm)
        .px(style.ui_style.spacing.xxs)
        .child(div().w(px(3.0)).h_4().rounded_full().bg(theme.accent))
        .child(
            div()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(title.into()),
        )
}

fn settings_select_control<D>(
    select: Entity<SelectState<D>>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    searchable: bool,
    search_placeholder: &'static str,
) -> Select<D>
where
    D: SearchableListDelegate + 'static,
    <D::Item as SearchableListItem>::Value: Clone + PartialEq,
{
    yttt_select(&select, theme, ui_style)
        .search_placeholder(search_placeholder)
        .when(searchable, |select| select.cleanable(false))
}

fn settings_number_control(
    input: Entity<InputState>,
    theme: WorkbenchTheme,
    style: YtttSettingsLayout,
) -> Div {
    div()
        .w(style.compact_control_width)
        .h(style.control_height)
        .child(
            yttt_number_input(&input, theme, style.ui_style)
                .w(style.compact_control_width)
                .h(style.control_height),
        )
}

fn settings_command_button(
    id: impl Into<String>,
    label: impl Into<String>,
    enabled: bool,
    theme: WorkbenchTheme,
    command: CommandId,
    cx: &mut Context<WorkbenchView>,
) -> Button {
    settings_button(
        id,
        label,
        false,
        theme,
        cx,
        cx.listener(move |this, _, window, cx| {
            if enabled {
                let _ = this.run_command(command);
                this.flush_pending_status_notifications(window, cx);
            }
            cx.notify();
        }),
    )
    .disabled(!enabled)
    .tab_stop(enabled)
}

fn settings_switch<H>(
    id: impl Into<String>,
    checked: bool,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    on_change: H,
) -> Div
where
    H: Fn(&bool, &mut Window, &mut gpui::App) + 'static,
{
    yttt_switch(
        SharedString::from(id.into()),
        checked,
        theme,
        ui_style,
        on_change,
    )
}

pub(in super::super) fn settings_button<H>(
    id: impl Into<String>,
    label: impl Into<String>,
    selected: bool,
    theme: WorkbenchTheme,
    cx: &mut Context<WorkbenchView>,
    on_click: H,
) -> Button
where
    H: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    let variant = if selected {
        YtttButtonVariant::Primary
    } else {
        YtttButtonVariant::Secondary
    };
    let ui_style = current_ui_style(cx);
    yttt_button(
        SharedString::from(id.into()),
        SharedString::from(label.into()),
        variant,
        theme,
        ui_style,
        cx,
    )
    .on_click(on_click)
}

fn settings_value(value: impl Into<String>, theme: WorkbenchTheme, ui_style: UiStyle) -> Div {
    div()
        .max_w(px(720.0))
        .rounded(ui_style.radius.compact)
        .border(ui_style.border.hairline)
        .border_color(theme.border_variant)
        .bg(theme.element_background)
        .px(ui_style.spacing.lg)
        .py(ui_style.spacing.xs)
        .text_xs()
        .text_color(theme.text_muted)
        .child(value.into())
}

fn settings_keybinding_value(
    keybindings: Vec<String>,
    unbound_label: impl Into<String>,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    if keybindings.is_empty() {
        return div().child(settings_value(unbound_label, theme, ui_style));
    }

    let mut value = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(ui_style.spacing.xs)
        .max_w_96();
    for keybinding in keybindings {
        value = value.child(workbench_keybinding_badge(keybinding, theme, ui_style));
    }
    value
}
