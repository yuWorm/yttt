use super::super::*;

pub(in super::super) fn settings_overlay(
    root: &mut WorkbenchView,
    search_input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let style = settings_panel_style();
    let panel = yttt_panel_style(YtttPanelKind::Settings, theme);

    capture_overlay_input(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000066))
            .child(
                div()
                    .flex()
                    .w(panel.width)
                    .h(panel.height.unwrap_or(style.height))
                    .max_w(panel.max_width)
                    .max_h(panel.max_height)
                    .rounded(panel.radius)
                    .border_1()
                    .border_color(panel.border)
                    .bg(panel.background)
                    .text_color(theme.text)
                    .overflow_hidden()
                    .child(settings_sidebar(root, search_input, style, cx))
                    .child(settings_content(root, style, window, cx)),
            ),
    )
}

fn settings_sidebar(
    root: &WorkbenchView,
    search_input: &Entity<InputState>,
    style: SettingsPanelStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let groups = root
        .settings
        .settings_page
        .visible_groups(&root.ui_text)
        .into_iter()
        .fold(div().flex().flex_col().gap_1(), |groups, group| {
            let group_id = group.id.as_str().to_string();
            let background = if group.selected {
                theme.active_surface
            } else {
                rgba(0x00000000)
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
                    .h_8()
                    .rounded_sm()
                    .px_3()
                    .bg(background)
                    .text_sm()
                    .text_color(text)
                    .hover(move |this| this.bg(theme.hover_surface))
                    .on_click(cx.listener(move |this, _, _window, cx| {
                        let _ = this.select_settings_group(&group_id);
                        cx.notify();
                    }))
                    .child(group.title),
            )
        });

    div()
        .flex()
        .flex_col()
        .w(style.sidebar_width)
        .h_full()
        .min_h_0()
        .flex_none()
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.app_background)
        .p_3()
        .gap_3()
        .child(
            div()
                .id(SharedString::from("settings-search"))
                .flex()
                .items_center()
                .h(style.search_height)
                .flex_none()
                .rounded_md()
                .bg(theme.surface)
                .overflow_hidden()
                .child(
                    Input::new(search_input)
                        .prefix(IconName::Search)
                        .cleanable(true)
                        .appearance(true)
                        .bg(theme.surface),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(groups.overflow_y_scrollbar()),
        )
}

fn settings_content(
    root: &mut WorkbenchView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let group = root.settings.settings_page.selected_group;

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .bg(theme.surface)
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(theme.border)
                .px_6()
                .py_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(group.title(&root.ui_text)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(group.description(&root.ui_text)),
                        ),
                )
                .child(settings_button(
                    "settings-close",
                    root.ui_text.get(UiTextKey::SettingsClose),
                    false,
                    theme,
                    cx,
                    cx.listener(|this, _, _window, cx| {
                        this.close_settings();
                        cx.notify();
                    }),
                )),
        )
        .child(
            div().flex_1().min_h_0().child(
                settings_rows(root, group, style, window, cx)
                    .px_6()
                    .overflow_y_scrollbar(),
            ),
        )
}

fn settings_rows(
    root: &mut WorkbenchView,
    group: SettingsGroupId,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    match group {
        SettingsGroupId::General => settings_general_rows(root, style, window, cx),
        SettingsGroupId::Appearance => settings_appearance_rows(root, style, window, cx),
        SettingsGroupId::Languages => settings_language_rows(root, style, window, cx),
        SettingsGroupId::Editor => settings_editor_rows(root, style, window, cx),
        SettingsGroupId::Terminal => settings_terminal_rows(root, style, window, cx),
        SettingsGroupId::DefaultLayout => settings_default_layout_rows(root, style, cx),
        SettingsGroupId::Keybindings => settings_keybinding_rows(root, style, cx),
    }
}

fn settings_general_rows(
    root: &mut WorkbenchView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let language_select = root.settings_language_select(window, cx);
    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguage),
            text.get(UiTextKey::SettingsLanguageDescription),
            settings_select_control(
                language_select,
                theme,
                false,
                text.get(UiTextKey::SettingsSelectLanguage),
            )
            .into_any_element(),
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
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_system_notifications_enabled(*checked);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
}

fn settings_appearance_rows(
    root: &mut WorkbenchView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let ui_theme_select = root.settings_ui_theme_select(window, cx);
    let terminal_theme_select = root.settings_terminal_theme_select(window, cx);
    let icon_theme_select = root.settings_icon_theme_select(window, cx);

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsUiTheme),
            text.get(UiTextKey::SettingsUiThemeDescription),
            settings_select_control(
                ui_theme_select,
                theme,
                true,
                text.get(UiTextKey::SettingsSearchTheme),
            )
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
                true,
                text.get(UiTextKey::SettingsSearchTheme),
            )
            .into_any_element(),
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
}

fn settings_language_rows(
    root: &mut WorkbenchView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
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
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguageDetection),
            text.get(UiTextKey::SettingsLanguageDetectionDescription),
            settings_switch(
                "settings-editor-auto-detect-language",
                root.editor_auto_detect_language(),
                theme,
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
            settings_value(supported_language_count.to_string(), theme).into_any_element(),
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
            settings_value(lsp_command, theme).into_any_element(),
        ))
}

fn settings_editor_rows(
    root: &mut WorkbenchView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
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
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorFontFamily),
                text.get(UiTextKey::SettingsEditorFontFamilyDescription),
                settings_select_control(
                    font_select,
                    theme,
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
                settings_number_control(font_size_input, style).into_any_element(),
            )
            .debug_selector(|| "settings-editor-font-size-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorLineHeight),
                text.get(UiTextKey::SettingsEditorLineHeightDescription),
                settings_number_control(line_height_input, style).into_any_element(),
            )
            .debug_selector(|| "settings-editor-line-height-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorTabSize),
                text.get(UiTextKey::SettingsEditorTabSizeDescription),
                settings_number_control(tab_size_input, style).into_any_element(),
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
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditorAutosave),
                text.get(UiTextKey::SettingsEditorAutosaveDescription),
                settings_select_control(
                    autosave_select,
                    theme,
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
                settings_number_control(autosave_delay_input, style).into_any_element(),
            )
            .debug_selector(|| "settings-editor-autosave-delay-row".to_string()),
        )
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
                settings_number_control(project_panel_width_input, style).into_any_element(),
            )
            .debug_selector(|| "settings-project-panel-width-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsProjectSidebarWidth),
                text.get(UiTextKey::SettingsProjectSidebarWidthDescription),
                settings_number_control(project_sidebar_width_input, style).into_any_element(),
            )
            .debug_selector(|| "settings-project-sidebar-width-row".to_string()),
        )
}

fn settings_terminal_rows(
    root: &mut WorkbenchView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
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
        .gap_2()
        .w(style.control_width)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .h(style.control_height)
                .child(Input::new(&custom_shell_input).small().appearance(true)),
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

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsDefaultShell),
            text.get(UiTextKey::SettingsDefaultShellDescription),
            settings_select_control(
                shell_select,
                theme,
                false,
                text.get(UiTextKey::SettingsSelectShell),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsCustomShell),
            text.get(UiTextKey::SettingsCustomShellDescription),
            custom_shell_control.into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsFontFamily),
            text.get(UiTextKey::SettingsFontFamilyDescription),
            settings_select_control(
                font_select,
                theme,
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
            settings_number_control(font_size_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLineHeight),
            text.get(UiTextKey::SettingsLineHeightDescription),
            settings_number_control(line_height_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsPadding),
            text.get(UiTextKey::SettingsPaddingDescription),
            settings_number_control(padding_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsScrollback),
            text.get(UiTextKey::SettingsScrollbackDescription),
            settings_number_control(scrollback_input, style).into_any_element(),
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
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsTerminalCursorShape),
                text.get(UiTextKey::SettingsTerminalCursorShapeDescription),
                settings_select_control(
                    cursor_shape_select,
                    theme,
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
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsTerminalOsc52Policy),
                text.get(UiTextKey::SettingsTerminalOsc52PolicyDescription),
                settings_select_control(
                    osc52_policy_select,
                    theme,
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

fn settings_default_layout_rows(
    root: &WorkbenchView,
    style: SettingsPanelStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let path = root.default_layout_state.path().display().to_string();

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsDefaultLayoutPath),
            text.get(UiTextKey::SettingsDefaultLayoutPathDescription),
            settings_value(path, theme).into_any_element(),
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
    style: SettingsPanelStyle,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let diagnostics = if root.settings.keybinding_warning_lines.is_empty() {
        text.get(UiTextKey::SettingsNoKeybindingConflicts)
            .to_string()
    } else {
        root.settings.keybinding_warning_lines.join("; ")
    };

    let mut rows = div()
        .flex()
        .flex_col()
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
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsKeybindingDiagnostics),
            text.get(UiTextKey::SettingsKeybindingDiagnosticsDescription),
            settings_value(diagnostics, theme).into_any_element(),
        ));

    for row in root.visible_keybinding_rows() {
        let command = row.command;
        let keys = row.display_keys();
        let title = row.title;
        let description = row.command_id;
        let title_text = if row.has_conflict {
            format!("{title} ({})", text.get(UiTextKey::SettingsConflict))
        } else {
            title.to_string()
        };

        rows = rows.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .min_h(style.row_min_height)
                .border_b_1()
                .border_color(theme.border)
                .py_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(title_text),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(description),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .flex_none()
                        .child(settings_keybinding_value(
                            keys,
                            text.get(UiTextKey::SettingsUnbound),
                            theme,
                        ))
                        .child(settings_button(
                            format!("settings-keybinding-edit-{}", row.command_id),
                            text.get(UiTextKey::SettingsEdit),
                            false,
                            theme,
                            cx,
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.open_keybinding_edit_dialog(command);
                                cx.notify();
                            }),
                        ))
                        .child(settings_button(
                            format!("settings-keybinding-reset-{}", row.command_id),
                            text.get(UiTextKey::SettingsReset),
                            false,
                            theme,
                            cx,
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.reset_keybinding_command_keys(command);
                                cx.notify();
                            }),
                        ))
                        .child(settings_button(
                            format!("settings-keybinding-delete-{}", row.command_id),
                            text.get(UiTextKey::SettingsDelete),
                            false,
                            theme,
                            cx,
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.delete_keybinding_command_keys(command);
                                cx.notify();
                            }),
                        )),
                ),
        );
    }

    rows
}

fn setting_row(
    style: SettingsPanelStyle,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    workbench_settings_row(style.control_width, theme, title, description, control)
}

fn settings_select_control(
    select: Entity<SettingsStringSelectState>,
    theme: WorkbenchTheme,
    searchable: bool,
    search_placeholder: &'static str,
) -> Select<SearchableVec<String>> {
    let select_style = yttt_select_style(theme);
    Select::new(&select)
        .small()
        .menu_width(select_style.menu_width)
        .search_placeholder(search_placeholder)
        .appearance(true)
        .w(select_style.width)
        .h(select_style.height)
        .rounded(select_style.radius)
        .bg(select_style.background)
        .border_color(select_style.border)
        .text_color(select_style.text)
        .when(searchable, |select| select.cleanable(false))
}

fn settings_number_control(input: Entity<InputState>, style: SettingsPanelStyle) -> Div {
    div()
        .w(style.compact_control_width)
        .h(style.control_height)
        .child(
            NumberInput::new(&input)
                .small()
                .appearance(true)
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
    on_change: H,
) -> Div
where
    H: Fn(&bool, &mut Window, &mut gpui::App) + 'static,
{
    workbench_switch(SharedString::from(id.into()), checked, theme, on_change)
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
    yttt_button(
        SharedString::from(id.into()),
        SharedString::from(label.into()),
        variant,
        theme,
        cx,
    )
    .on_click(on_click)
}

fn settings_value(value: impl Into<String>, theme: WorkbenchTheme) -> Div {
    div()
        .max_w_64()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_elevated)
        .px_3()
        .py_1()
        .text_xs()
        .text_color(theme.text_muted)
        .child(value.into())
}

fn settings_keybinding_value(
    keybindings: Vec<String>,
    unbound_label: impl Into<String>,
    theme: WorkbenchTheme,
) -> Div {
    if keybindings.is_empty() {
        return div().child(settings_value(unbound_label, theme));
    }

    let mut value = div().flex().items_center().justify_end().gap_1().max_w_96();
    for keybinding in keybindings {
        value = value.child(workbench_keybinding_badge(keybinding, theme));
    }
    value
}
