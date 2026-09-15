use crate::config::scope::{SettingApply, SettingsScope, setting_apply};

use super::super::*;

pub(in super::super) fn settings_window_content(
    root: &mut WorkbenchView,
    search_input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let appearance = root.theme_runtime();
    let theme = appearance.ui;
    let style = yttt_settings_layout(appearance.style, window.viewport_size());

    div()
        .debug_selector(|| "settings-panel".to_string())
        .flex()
        .size_full()
        .overflow_hidden()
        .bg(theme.editor_background)
        .child(settings_sidebar(root, search_input, style, cx))
        .child(settings_content(root, style, window, cx))
}

fn settings_sidebar(
    root: &WorkbenchView,
    search_input: &Entity<InputState>,
    style: YtttSettingsLayout,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let ui_style = style.ui_style;
    let visible_groups = root
        .settings
        .settings_page
        .visible_groups_for_settings_scope(&root.ui_text, root.selected_settings_scope());
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
    let visible_groups = root
        .settings
        .settings_page
        .visible_groups_for_settings_scope(&root.ui_text, root.selected_settings_scope());
    let no_results = visible_groups.is_empty();
    let (title, description) = if no_results {
        (
            root.ui_text.get(UiTextKey::SettingsNoSearchResults),
            root.ui_text
                .get(UiTextKey::SettingsNoSearchResultsDescription),
        )
    } else {
        (group.title(&root.ui_text), group.description(&root.ui_text))
    };
    let scope_selector = settings_scope_selector(root, style, window, cx);
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
                .child(scope_selector),
        )
        .child(div().flex_1().min_h_0().child(rows))
}
fn settings_scope_selector(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    _window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let scope = root.selected_settings_scope();
    let selected_index = match scope {
        SettingsScope::Device => 0,
        SettingsScope::Host => 1,
        SettingsScope::Project => 2,
    };
    let project_name = root.selected_settings_project_name();
    let host_name = root
        .terminal
        .host_runtime
        .as_ref()
        .and_then(|runtime| runtime.remote_label())
        .unwrap_or(text.get(UiTextKey::SettingsTargetDevice));
    let profile_name = root
        .terminal
        .host_runtime
        .as_ref()
        .and_then(|runtime| runtime.remote_environment())
        .map(|environment| environment.environment_id.as_str())
        .or_else(|| {
            root.config_paths
                .profile()
                .map(|profile| profile.id().as_str())
        });
    let identity = match scope {
        SettingsScope::Device => text.get(UiTextKey::SettingsScopeDevice).to_string(),
        SettingsScope::Host => match profile_name {
            Some(profile) => format!(
                "{}{host_name} · {profile}",
                text.get(UiTextKey::SettingsHostIdentity)
            ),
            None => format!("{}{host_name}", text.get(UiTextKey::SettingsHostIdentity)),
        },
        SettingsScope::Project => project_name
            .as_ref()
            .map(|name| format!("{}: {name}", text.get(UiTextKey::SettingsTargetProject)))
            .unwrap_or_else(|| {
                text.get(UiTextKey::SettingsProjectTargetUnavailable)
                    .to_string()
            }),
    };
    let read_only_reason = root.settings_scope_read_only_reason();

    div()
        .debug_selector(|| "settings-scope-selector".to_string())
        .flex()
        .flex_col()
        .items_end()
        .gap(style.ui_style.spacing.xs)
        .child(
            TabBar::new("settings-scope-tabs")
                .segmented()
                .xsmall()
                .selected_index(selected_index)
                .on_click(cx.listener(|root, selected_index: &usize, window, cx| {
                    let scope = match *selected_index {
                        0 => SettingsScope::Device,
                        1 => SettingsScope::Host,
                        2 => SettingsScope::Project,
                        _ => return,
                    };
                    root.select_settings_scope(scope, window, cx);
                    cx.notify();
                }))
                .child(
                    Tab::new()
                        .debug_selector(|| "settings-scope-device".to_string())
                        .label(text.get(UiTextKey::SettingsTargetDevice)),
                )
                .child(
                    Tab::new()
                        .debug_selector(|| "settings-scope-host".to_string())
                        .label(text.get(UiTextKey::SettingsTargetHost)),
                )
                .child(
                    Tab::new()
                        .debug_selector(|| "settings-scope-project".to_string())
                        .label(text.get(UiTextKey::SettingsTargetProject)),
                ),
        )
        .child(
            div()
                .text_xs()
                .text_color(theme.text_subtle)
                .child(identity),
        )
        .when_some(read_only_reason, |selector, reason| {
            selector.child(
                div()
                    .max_w(px(360.0))
                    .text_right()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(text.get(reason)),
            )
        })
        .when(
            scope == SettingsScope::Project && project_name.is_none(),
            |selector| {
                selector.child(
                    div()
                        .text_xs()
                        .text_color(theme.warning)
                        .child(text.get(UiTextKey::SettingsProjectTargetUnavailable)),
                )
            },
        )
        .when(root.has_failed_settings_save(), |selector| {
            selector.child(settings_failed_save_banner(root, style, cx))
        })
}

fn settings_failed_save_banner(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let retry_allowed = root.settings_scope_retry_allowed();
    let message = root
        .settings
        .settings_save_error
        .clone()
        .unwrap_or_else(|| "A settings save failed.".to_string());

    div()
        .debug_selector(|| "settings-save-failure".to_string())
        .flex()
        .flex_col()
        .items_end()
        .gap(style.ui_style.spacing.xs)
        .max_w(px(360.0))
        .p(style.ui_style.spacing.sm)
        .rounded(style.ui_style.radius.compact)
        .bg(theme.warning.alpha(0.12))
        .child(
            div()
                .text_right()
                .text_xs()
                .text_color(theme.warning)
                .child(message),
        )
        .child(
            div()
                .flex()
                .gap(style.ui_style.spacing.xs)
                .child(
                    settings_button(
                        "settings-retry-failed-save",
                        text.get(UiTextKey::Retry),
                        true,
                        theme,
                        cx,
                        cx.listener(|this, _, _window, cx| {
                            if let Err(error) = this.retry_failed_settings_save() {
                                this.settings.settings_save_error = Some(error.to_string());
                            }
                            cx.notify();
                        }),
                    )
                    .disabled(!retry_allowed)
                    .tab_stop(retry_allowed),
                )
                .child(settings_button(
                    "settings-copy-failed-draft",
                    text.get(UiTextKey::SettingsCopyDraft),
                    false,
                    theme,
                    cx,
                    cx.listener(|this, _, _window, cx| {
                        this.copy_failed_settings_draft(cx);
                        cx.notify();
                    }),
                ))
                .child(settings_button(
                    "settings-discard-failed-save",
                    text.get(UiTextKey::Discard),
                    false,
                    theme,
                    cx,
                    cx.listener(|this, _, _window, cx| {
                        this.discard_failed_settings_save(cx);
                        cx.notify();
                    }),
                )),
        )
}

fn settings_project_target_status(root: &WorkbenchView, style: YtttSettingsLayout) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let (status, message, color) = if root.selected_settings_project_name().is_none() {
        (
            UiTextKey::SettingsProjectTargetUnavailable,
            None,
            theme.warning,
        )
    } else if let Some(error) = root.project_settings_load_error() {
        (
            UiTextKey::SettingsProjectLoadFailed,
            Some(error),
            theme.warning,
        )
    } else {
        (UiTextKey::SettingsProjectLoading, None, theme.accent)
    };

    div()
        .debug_selector(|| "settings-project-load-status".to_string())
        .flex()
        .flex_col()
        .gap(style.ui_style.spacing.xs)
        .rounded(style.ui_style.radius.compact)
        .border(style.ui_style.border.hairline)
        .border_color(color.alpha(0.5))
        .bg(color.alpha(0.1))
        .p(style.ui_style.spacing.md)
        .text_xs()
        .text_color(color)
        .child(text.get(status))
        .when_some(message, |status, message| {
            status.child(
                div()
                    .text_xs()
                    .text_color(theme.text_muted)
                    .child(message.to_string()),
            )
        })
}

fn settings_rows(
    root: &mut WorkbenchView,
    group: SettingsGroupId,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    if root.selected_settings_scope() == SettingsScope::Project {
        if !root.project_settings_snapshot_is_confirmed() {
            return settings_project_target_status(root, style);
        }
        return match group {
            SettingsGroupId::Languages | SettingsGroupId::Editor => {
                settings_project_override_rows(root, group, style, window, cx)
            }
            SettingsGroupId::ProjectLayout => settings_project_layout_rows(root, style, cx),
            _ => div().flex().flex_col(),
        };
    }

    match group {
        SettingsGroupId::General => settings_general_rows(root, style, window, cx),
        SettingsGroupId::Appearance => settings_appearance_rows(root, style, window, cx),
        SettingsGroupId::Languages => settings_language_rows(root, style, window, cx),
        SettingsGroupId::Editor => settings_editor_rows(root, style, window, cx),
        SettingsGroupId::Terminal => settings_terminal_rows(root, style, window, cx),
        SettingsGroupId::Agent => settings_agent_rows(root, style, cx),
        SettingsGroupId::Permissions => settings_permission_rows(root, style, cx),
        SettingsGroupId::ProjectLayout => div().flex().flex_col(),
        SettingsGroupId::DefaultLayout => settings_default_layout_rows(root, style, cx),
        SettingsGroupId::Keybindings => settings_keybinding_rows(root, style, window, cx),
    }
}

fn project_override_description(
    root: &WorkbenchView,
    key: crate::config::project_settings::ProjectEditorSettingKey,
    description: &str,
) -> String {
    let source = match root
        .cached_project_editor_setting(key)
        .map(|setting| setting.source)
    {
        Some(crate::config::project_settings::ProjectSettingSource::Project) => {
            root.ui_text.get(UiTextKey::SettingsSourceProject)
        }
        _ => root.ui_text.get(UiTextKey::SettingsSourceHost),
    };
    format!(
        "{description} · {}: {source}",
        root.ui_text.get(UiTextKey::SettingsEffectiveSource)
    )
}

fn project_override_reset_button(
    id: &'static str,
    key: crate::config::project_settings::ProjectEditorSettingKey,
    text: UiText,
    theme: WorkbenchTheme,
    cx: &mut Context<WorkbenchView>,
) -> Button {
    settings_button(
        id,
        text.get(UiTextKey::SettingsUseHostDefault),
        false,
        theme,
        cx,
        cx.listener(move |this, _, window, cx| {
            if let Err(error) = this.save_project_editor_setting(key, None, window, cx) {
                this.load_error = Some(error);
            }
            cx.notify();
        }),
    )
}

fn settings_project_override_rows(
    root: &mut WorkbenchView,
    group: SettingsGroupId,
    mut style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    use crate::config::project_settings::{ProjectEditorSettingKey, ProjectEditorSettingValue};
    // Project controls include an explicit reset action; keep their full width below the source.
    style.stack_rows = true;

    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    match group {
        SettingsGroupId::Languages => {
            let auto_detect = root
                .cached_project_editor_setting(ProjectEditorSettingKey::AutoDetectLanguage)
                .and_then(|setting| match &setting.value {
                    ProjectEditorSettingValue::AutoDetectLanguage(value) => Some(*value),
                    _ => None,
                })
                .unwrap_or_else(|| root.editor_auto_detect_language());
            let default_language_input = root.settings_project_default_language_input(window, cx);
            let auto_detect_description = project_override_description(
                root,
                ProjectEditorSettingKey::AutoDetectLanguage,
                text.get(UiTextKey::SettingsLanguageDetectionDescription),
            );
            let default_language_description = project_override_description(
                root,
                ProjectEditorSettingKey::DefaultLanguage,
                text.get(UiTextKey::SettingsDefaultCodeLanguageDescription),
            );
            div()
                .flex()
                .flex_col()
                .child(settings_section_header(
                    style,
                    theme,
                    text.get(UiTextKey::SettingsSectionDetectionDefaults),
                    true,
                ))
                .child(scoped_settings_row(
                    root,
                    "editor.auto_detect_language",
                    style,
                    theme,
                    text.get(UiTextKey::SettingsLanguageDetection),
                    auto_detect_description,
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(style.ui_style.spacing.sm)
                        .child(settings_switch(
                            "settings-project-auto-detect-language",
                            auto_detect,
                            theme,
                            style.ui_style,
                            cx.listener(|this, checked: &bool, window, cx| {
                                if let Err(error) = this.save_project_editor_setting(
                                    ProjectEditorSettingKey::AutoDetectLanguage,
                                    Some(ProjectEditorSettingValue::AutoDetectLanguage(*checked)),
                                    window,
                                    cx,
                                ) {
                                    this.load_error = Some(error);
                                }
                                cx.notify();
                            }),
                        ))
                        .child(project_override_reset_button(
                            "settings-project-auto-detect-reset",
                            ProjectEditorSettingKey::AutoDetectLanguage,
                            text,
                            theme,
                            cx,
                        ))
                        .into_any_element(),
                ))
                .child(scoped_settings_row(
                    root,
                    "editor.default_language",
                    style,
                    theme,
                    text.get(UiTextKey::SettingsDefaultCodeLanguage),
                    default_language_description,
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(style.ui_style.spacing.sm)
                        .child(
                            div()
                                .w(style.compact_control_width)
                                .h(style.control_height)
                                .child(
                                    yttt_input(
                                        &default_language_input,
                                        YtttInputKind::Settings,
                                        theme,
                                        style.ui_style,
                                    )
                                    .small(),
                                ),
                        )
                        .child(project_override_reset_button(
                            "settings-project-default-language-reset",
                            ProjectEditorSettingKey::DefaultLanguage,
                            text,
                            theme,
                            cx,
                        ))
                        .into_any_element(),
                ))
        }
        SettingsGroupId::Editor => {
            let tab_size_input = root.settings_project_tab_size_input(window, cx);
            let tab_size_description = project_override_description(
                root,
                ProjectEditorSettingKey::TabSize,
                text.get(UiTextKey::SettingsEditorTabSizeDescription),
            );
            div()
                .flex()
                .flex_col()
                .child(settings_section_header(
                    style,
                    theme,
                    text.get(UiTextKey::SettingsSectionEditingBehavior),
                    true,
                ))
                .child(scoped_settings_row(
                    root,
                    "editor.tab_size",
                    style,
                    theme,
                    text.get(UiTextKey::SettingsEditorTabSize),
                    tab_size_description,
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap(style.ui_style.spacing.sm)
                        .child(settings_number_control(tab_size_input, theme, style))
                        .child(project_override_reset_button(
                            "settings-project-tab-size-reset",
                            ProjectEditorSettingKey::TabSize,
                            text,
                            theme,
                            cx,
                        ))
                        .into_any_element(),
                ))
        }
        _ => div().flex().flex_col(),
    }
}

fn settings_project_layout_rows(
    root: &WorkbenchView,
    style: YtttSettingsLayout,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let editable = root.settings_scope_is_editable();
    div()
        .flex()
        .flex_col()
        .child(scoped_settings_row(
            root,
            "project_layout.edit",
            style,
            theme,
            text.get(UiTextKey::SettingsEditLayoutToml),
            text.get(UiTextKey::SettingsEditLayoutTomlDescription),
            settings_command_button(
                "settings-project-layout-edit",
                text.get(UiTextKey::SettingsEdit),
                editable,
                theme,
                CommandId::LayoutProjectEdit,
                cx,
            )
            .into_any_element(),
        ))
        .child(scoped_settings_row(
            root,
            "project_layout.save",
            style,
            theme,
            text.get(UiTextKey::SettingsSaveCurrentLayout),
            text.get(UiTextKey::SettingsSaveCurrentLayoutDescription),
            settings_command_button(
                "settings-project-layout-save",
                text.get(UiTextKey::SettingsSave),
                editable,
                theme,
                CommandId::LayoutSaveCurrent,
                cx,
            )
            .into_any_element(),
        ))
        .child(scoped_settings_row(
            root,
            "project_layout.export",
            style,
            theme,
            text.get(UiTextKey::SettingsExportProjectLayout),
            text.get(UiTextKey::SettingsExportProjectLayoutDescription),
            settings_command_button(
                "settings-project-layout-export",
                text.get(UiTextKey::SettingsExport),
                editable,
                theme,
                CommandId::LayoutExportProjectConfig,
                cx,
            )
            .into_any_element(),
        ))
}

fn settings_general_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    window: &mut Window,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    if root.selected_settings_scope() == SettingsScope::Host {
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
        return div()
            .flex()
            .flex_col()
            .child(
                scoped_settings_block(
                    root,
                    "general.new_tab_commands",
                    style,
                    theme,
                    text.get(UiTextKey::SettingsNewTabCommands),
                    text.get(UiTextKey::SettingsNewTabCommandsDescription),
                    command_list.into_any_element(),
                )
                .debug_selector(|| "settings-new-tab-commands-row".to_string()),
            )
            .child(setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsEditSettingsToml),
                text.get(UiTextKey::SettingsEditSettingsTomlDescription),
                settings_button(
                    "settings-open-host-file",
                    text.get(UiTextKey::SettingsShowPath),
                    false,
                    theme,
                    cx,
                    cx.listener(|this, _, window, cx| {
                        this.show_settings_file_path_status();
                        this.flush_pending_status_notifications(window, cx);
                        cx.notify();
                    }),
                )
                .into_any_element(),
            ));
    }
    let language_select = root.settings_language_select(window, cx);
    let vim_mode_select = root.settings_vim_mode_select(window, cx);
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
                text.get(UiTextKey::RemoteServices),
                text.get(
                    if root
                        .terminal
                        .host_runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.is_remote())
                    {
                        UiTextKey::RemoteManageLocally
                    } else {
                        UiTextKey::RemoteServicesDescription
                    },
                ),
                settings_button(
                    "settings-open-remote-services",
                    text.get(UiTextKey::SettingsOpen),
                    false,
                    theme,
                    cx,
                    cx.listener(|this, _, _window, cx| {
                        this.open_ssh_connection_manager();
                        cx.notify();
                    }),
                )
                .disabled(
                    root.terminal
                        .host_runtime
                        .as_ref()
                        .is_some_and(|runtime| runtime.is_remote()),
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-remote-services-row".to_string()),
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
    let window_bar_inputs = [
        root.settings_bar_input(SettingsBarField::WindowLeft, window, cx),
        root.settings_bar_input(SettingsBarField::WindowCenter, window, cx),
        root.settings_bar_input(SettingsBarField::WindowRight, window, cx),
    ];
    let status_bar_inputs = [
        root.settings_bar_input(SettingsBarField::StatusLeft, window, cx),
        root.settings_bar_input(SettingsBarField::StatusCenter, window, cx),
        root.settings_bar_input(SettingsBarField::StatusRight, window, cx),
    ];
    let window_bar_inputs_for_apply = window_bar_inputs.clone();
    let status_bar_inputs_for_apply = status_bar_inputs.clone();
    let status_bar_control = div()
        .flex()
        .flex_col()
        .gap(style.ui_style.spacing.md)
        .w_full()
        .child(settings_bar_modules_control(
            "settings-status-bar",
            &status_bar_inputs,
            text,
            theme,
            style,
        ))
        .child(div().flex().justify_end().child(settings_button(
            "settings-apply-bar-layout",
            text.get(UiTextKey::SettingsApplyBarLayout),
            true,
            theme,
            cx,
            cx.listener(move |this, _, _window, cx| {
                let window_values = window_bar_inputs_for_apply
                    .each_ref()
                    .map(|input| input.read(cx).value().to_string());
                let status_values = status_bar_inputs_for_apply
                    .each_ref()
                    .map(|input| input.read(cx).value().to_string());
                if let Err(error) = this.apply_shell_bar_layout(window_values, status_values, cx) {
                    this.load_error = Some(error);
                }
                cx.notify();
            }),
        )));

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
            text.get(UiTextKey::SettingsSectionBars),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsWindowBarModules),
                text.get(UiTextKey::SettingsWindowBarModulesDescription),
                settings_bar_modules_control(
                    "settings-window-bar",
                    &window_bar_inputs,
                    text,
                    theme,
                    style,
                )
                .into_any_element(),
            )
            .debug_selector(|| "settings-window-bar-modules-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsStatusBarEnabled),
                text.get(UiTextKey::SettingsStatusBarEnabledDescription),
                settings_switch(
                    "settings-status-bar-enabled",
                    root.app_settings.bars.status.enabled,
                    theme,
                    style.ui_style,
                    cx.listener(|this, checked: &bool, _window, cx| {
                        if let Err(error) = this.set_status_bar_enabled(*checked, cx) {
                            this.load_error = Some(error.to_string());
                        }
                        cx.notify();
                    }),
                )
                .debug_selector(|| "settings-status-bar-enabled".to_string())
                .into_any_element(),
            )
            .debug_selector(|| "settings-status-bar-enabled-row".to_string()),
        )
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsStatusBarModules),
                text.get(UiTextKey::SettingsStatusBarModulesDescription),
                status_bar_control.into_any_element(),
            )
            .debug_selector(|| "settings-status-bar-modules-row".to_string()),
        )
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
            text.get(UiTextKey::SettingsEditBarsToml),
            text.get(UiTextKey::SettingsEditBarsTomlDescription),
            settings_button(
                "settings-open-bars-file",
                text.get(UiTextKey::SettingsShowPath),
                false,
                theme,
                cx,
                cx.listener(move |this, _, window, cx| {
                    this.show_bars_file_path_status();
                    this.flush_pending_status_notifications(window, cx);
                    cx.notify();
                }),
            )
            .debug_selector(|| "settings-open-bars-file".to_string())
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
                    this.open_zed_theme_import_dialog(window, cx);
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
    if root.selected_settings_scope() == SettingsScope::Device {
        return setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsSupportedLanguages),
            text.get(UiTextKey::SettingsSupportedLanguagesDescription),
            settings_value(
                root.available_editor_language_names().len().to_string(),
                theme,
                style.ui_style,
            )
            .into_any_element(),
        );
    }
    let default_language_select = root.settings_editor_language_select(window, cx);
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
        .child(scoped_settings_row(
            root,
            "editor.auto_detect_language",
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
        .child(scoped_settings_row(
            root,
            "editor.default_language",
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
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionLanguageServices),
            false,
        ))
        .child(scoped_settings_row(
            root,
            "editor.lsp.enabled",
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
        .child(scoped_settings_row(
            root,
            "editor.lsp.command",
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
    if root.selected_settings_scope() == SettingsScope::Host {
        let tab_size_input =
            root.settings_number_input(SettingsNumberField::EditorTabSize, window, cx);
        return scoped_settings_row(
            root,
            "editor.tab_size",
            style,
            theme,
            text.get(UiTextKey::SettingsEditorTabSize),
            text.get(UiTextKey::SettingsEditorTabSizeDescription),
            settings_number_control(tab_size_input, theme, style).into_any_element(),
        )
        .debug_selector(|| "settings-editor-tab-size-row".to_string());
    }
    let font_select = root.settings_editor_font_family_select(window, cx);
    let autosave_select = root.settings_editor_autosave_select(window, cx);
    let font_size_input =
        root.settings_number_input(SettingsNumberField::EditorFontSize, window, cx);
    let line_height_input =
        root.settings_number_input(SettingsNumberField::EditorLineHeight, window, cx);
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
    if root.selected_settings_scope() == SettingsScope::Host {
        let shell_select = root.settings_shell_select(window, cx);
        let custom_shell_input = root.settings_custom_shell_input(window, cx);
        let scrollback_input =
            root.settings_number_input(SettingsNumberField::Scrollback, window, cx);
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
        return div()
            .flex()
            .flex_col()
            .child(settings_section_header(
                style,
                theme,
                text.get(UiTextKey::SettingsSectionShell),
                true,
            ))
            .child(scoped_settings_row(
                root,
                "terminal.shell",
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
            .child(scoped_settings_block(
                root,
                "terminal.custom_shells",
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
                scoped_settings_block(
                    root,
                    "terminal.environment",
                    style,
                    theme,
                    text.get(UiTextKey::SettingsEnvironmentVariables),
                    text.get(UiTextKey::SettingsEnvironmentVariablesDescription),
                    environment_list.into_any_element(),
                )
                .debug_selector(|| "settings-terminal-environment-row".to_string()),
            )
            .child(scoped_settings_row(
                root,
                "terminal.scrollback",
                style,
                theme,
                text.get(UiTextKey::SettingsScrollback),
                text.get(UiTextKey::SettingsScrollbackDescription),
                settings_number_control(scrollback_input, theme, style).into_any_element(),
            ))
            .child(
                scoped_settings_row(
                    root,
                    "terminal.kitty_keyboard",
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
            );
    }
    let font_select = root.settings_font_family_select(window, cx);
    let font_size_input = root.settings_number_input(SettingsNumberField::FontSize, window, cx);
    let line_height_input = root.settings_number_input(SettingsNumberField::LineHeight, window, cx);
    let padding_input = root.settings_number_input(SettingsNumberField::Padding, window, cx);
    let cursor_shape_select = root.settings_terminal_cursor_shape_select(window, cx);
    let osc52_policy_select = root.settings_terminal_osc52_policy_select(window, cx);
    div()
        .flex()
        .flex_col()
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
                if root.settings_scope_is_editable() {
                    settings_switch(
                        format!("settings-agent-session-provider-{provider_id}"),
                        root.agent_session_agent_enabled(agent),
                        theme,
                        style.ui_style,
                        cx.listener(move |this, checked: &bool, _window, cx| {
                            if let Err(error) =
                                this.set_agent_session_agent_enabled(agent, *checked)
                            {
                                this.load_error = Some(error.to_string());
                            }
                            cx.notify();
                        }),
                    )
                } else {
                    settings_value(
                        (root.agent_session_agent_enabled(agent)).to_string(),
                        theme,
                        style.ui_style,
                    )
                }
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
            scoped_settings_row(
                root,
                "agent.primary",
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
            scoped_settings_row(
                root,
                "agent.sessions_enabled",
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

fn settings_permission_rows(
    root: &mut WorkbenchView,
    style: YtttSettingsLayout,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    root.ensure_permission_status_refresh(cx);
    root.ensure_login_startup_refresh(cx);
    let theme = root.theme_runtime().ui;
    let text = root.ui_text;
    let remote_permissions_local = root
        .terminal
        .host_runtime
        .as_ref()
        .is_some_and(|runtime| runtime.is_remote());
    let refreshing = root.permission_refreshing();
    let action_in_progress = root.permission_action_in_progress();
    let login_startup_state = root.login_startup_state();
    let login_startup_enabled = root.login_startup_enabled();
    let login_startup_busy = root.login_startup_busy();
    let login_startup_control = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(style.ui_style.spacing.sm)
        .child(login_startup_status_value(
            login_startup_state.status,
            login_startup_busy,
            text,
            theme,
            style.ui_style,
        ))
        .child(
            settings_switch(
                "settings-login-startup",
                login_startup_enabled,
                theme,
                style.ui_style,
                cx.listener(move |this, checked: &bool, window, cx| {
                    if !login_startup_busy {
                        this.request_login_startup_change(*checked, window, cx);
                        cx.notify();
                    }
                }),
            )
            .opacity(if login_startup_busy { 0.5 } else { 1.0 })
            .debug_selector(|| "settings-login-startup".to_string()),
        )
        .into_any_element();
    let permissions = platform::platform_permissions();
    let core_rows = permissions
        .iter()
        .copied()
        .filter(|permission| !permission.kind.is_optional())
        .fold(div().flex().flex_col(), |rows, permission| {
            rows.child(permission_setting_row(
                permission,
                root.permission_status(permission.kind),
                root.permission_action(permission.kind),
                root.permission_request_in_progress(permission.kind),
                action_in_progress,
                text,
                theme,
                style,
                cx,
            ))
        });
    let optional_rows = permissions
        .iter()
        .copied()
        .filter(|permission| permission.kind.is_optional())
        .fold(div().flex().flex_col(), |rows, permission| {
            rows.child(permission_setting_row(
                permission,
                root.permission_status(permission.kind),
                root.permission_action(permission.kind),
                root.permission_request_in_progress(permission.kind),
                action_in_progress,
                text,
                theme,
                style,
                cx,
            ))
        });
    let refresh_disabled = refreshing || action_in_progress;
    let refresh_control = settings_button(
        "settings-permissions-refresh",
        text.get(if refreshing {
            UiTextKey::SettingsPermissionChecking
        } else {
            UiTextKey::SettingsPermissionRefresh
        }),
        false,
        theme,
        cx,
        cx.listener(|this, _, _window, cx| {
            this.refresh_permission_statuses(cx);
            cx.notify();
        }),
    )
    .disabled(refresh_disabled)
    .tab_stop(!refresh_disabled)
    .debug_selector(|| "settings-permissions-refresh".to_string())
    .into_any_element();

    div()
        .flex()
        .flex_col()
        .when(remote_permissions_local, |rows| {
            rows.child(setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsRemotePermissionsLocal),
                text.get(UiTextKey::SettingsRemotePermissionsLocalDescription),
                settings_value(
                    text.get(UiTextKey::SettingsScopeDevice),
                    theme,
                    style.ui_style,
                )
                .into_any_element(),
            ))
        })
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionBackgroundHost),
            true,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsLoginStartup),
                text.get(UiTextKey::SettingsLoginStartupDescription),
                login_startup_control,
            )
            .debug_selector(|| "settings-login-startup-row".to_string()),
        )
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionCorePermissions),
            false,
        ))
        .child(
            setting_row(
                style,
                theme,
                text.get(UiTextKey::SettingsPermissionStatus),
                text.get(UiTextKey::SettingsPermissionStatusDescription),
                refresh_control,
            )
            .debug_selector(|| "settings-permissions-status-row".to_string()),
        )
        .child(core_rows)
        .child(settings_section_header(
            style,
            theme,
            text.get(UiTextKey::SettingsSectionOptionalPermissions),
            false,
        ))
        .child(optional_rows)
}

#[allow(clippy::too_many_arguments)]
fn permission_setting_row(
    permission: platform::PlatformPermission,
    status: platform::PermissionStatus,
    action: platform::PermissionAction,
    requesting: bool,
    action_in_progress: bool,
    text: UiText,
    theme: WorkbenchTheme,
    style: YtttSettingsLayout,
    cx: &mut Context<WorkbenchView>,
) -> Div {
    let (title_key, description_key) = permission_text_keys(permission.kind);
    let mut control = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(style.ui_style.spacing.sm)
        .child(permission_status_value(
            permission.kind,
            status,
            text,
            theme,
            style.ui_style,
        ));
    if requesting {
        control = control.child(
            settings_button(
                format!("settings-permission-{}", permission.kind.as_str()),
                text.get(UiTextKey::SettingsPermissionRequesting),
                false,
                theme,
                cx,
                cx.listener(|_, _, _, _| {}),
            )
            .disabled(true)
            .tab_stop(false),
        );
    } else if action != platform::PermissionAction::None {
        let kind = permission.kind;
        let action_label = match action {
            platform::PermissionAction::Request => UiTextKey::SettingsPermissionRequest,
            platform::PermissionAction::OpenSettings
                if status == platform::PermissionStatus::Granted =>
            {
                UiTextKey::SettingsPermissionManage
            }
            platform::PermissionAction::OpenSettings => {
                UiTextKey::SettingsPermissionOpenSystemSettings
            }
            platform::PermissionAction::None => unreachable!(),
        };
        control = control.child(
            settings_button(
                format!("settings-permission-{}", permission.kind.as_str()),
                text.get(action_label),
                false,
                theme,
                cx,
                cx.listener(move |this, _, _window, cx| {
                    this.request_or_open_permission(kind, cx);
                    cx.notify();
                }),
            )
            .disabled(action_in_progress)
            .tab_stop(!action_in_progress),
        );
    }

    setting_row(
        style,
        theme,
        text.get(title_key),
        text.get(description_key),
        control.into_any_element(),
    )
    .debug_selector(move || format!("settings-permission-{}-row", permission.kind.as_str()))
}

fn login_startup_status_value(
    status: LoginStartupStatus,
    busy: bool,
    text: UiText,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let (key, color) = if busy {
        (UiTextKey::SettingsLoginStartupChecking, theme.accent)
    } else {
        match status {
            LoginStartupStatus::Enabled => (UiTextKey::SettingsLoginStartupEnabled, theme.success),
            LoginStartupStatus::Disabled => {
                (UiTextKey::SettingsLoginStartupDisabled, theme.text_subtle)
            }
            LoginStartupStatus::RequiresApproval => (
                UiTextKey::SettingsLoginStartupRequiresApproval,
                theme.warning,
            ),
            LoginStartupStatus::Unavailable => (
                UiTextKey::SettingsLoginStartupUnavailable,
                theme.text_subtle,
            ),
        }
    };
    div()
        .debug_selector(|| "settings-login-startup-status".to_string())
        .rounded(ui_style.radius.compact)
        .border(ui_style.border.hairline)
        .border_color(color.alpha(0.5))
        .bg(color.alpha(0.1))
        .px(ui_style.spacing.md)
        .py(ui_style.spacing.xs)
        .text_xs()
        .text_color(color)
        .child(text.get(key))
}

fn permission_status_value(
    kind: platform::PermissionKind,
    status: platform::PermissionStatus,
    text: UiText,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Div {
    let color = match status {
        platform::PermissionStatus::Granted => theme.success,
        platform::PermissionStatus::Denied => theme.warning,
        platform::PermissionStatus::Checking => theme.accent,
        platform::PermissionStatus::Unknown
        | platform::PermissionStatus::NotDetermined
        | platform::PermissionStatus::Unavailable
        | platform::PermissionStatus::ManagedBySystem
        | platform::PermissionStatus::RequestedWhenNeeded
        | platform::PermissionStatus::NotRequired => theme.text_subtle,
    };
    div()
        .debug_selector(move || format!("settings-permission-{}-status", kind.as_str()))
        .rounded(ui_style.radius.compact)
        .border(ui_style.border.hairline)
        .border_color(color.alpha(0.5))
        .bg(color.alpha(0.1))
        .px(ui_style.spacing.md)
        .py(ui_style.spacing.xs)
        .text_xs()
        .text_color(color)
        .child(text.get(permission_status_key(status)))
}

fn permission_text_keys(kind: platform::PermissionKind) -> (UiTextKey, UiTextKey) {
    match kind {
        platform::PermissionKind::Notifications => (
            UiTextKey::SettingsPermissionNotifications,
            UiTextKey::SettingsPermissionNotificationsDescription,
        ),
        platform::PermissionKind::FileSystem => (
            UiTextKey::SettingsPermissionFileSystem,
            UiTextKey::SettingsPermissionFileSystemDescription,
        ),
        platform::PermissionKind::DeveloperTools => (
            UiTextKey::SettingsPermissionDeveloperTools,
            UiTextKey::SettingsPermissionDeveloperToolsDescription,
        ),
        platform::PermissionKind::Accessibility => (
            UiTextKey::SettingsPermissionAccessibility,
            UiTextKey::SettingsPermissionAccessibilityDescription,
        ),
        platform::PermissionKind::ScreenCapture => (
            UiTextKey::SettingsPermissionScreenCapture,
            UiTextKey::SettingsPermissionScreenCaptureDescription,
        ),
    }
}

fn permission_status_key(status: platform::PermissionStatus) -> UiTextKey {
    match status {
        platform::PermissionStatus::Checking => UiTextKey::SettingsPermissionChecking,
        platform::PermissionStatus::Unknown | platform::PermissionStatus::Unavailable => {
            UiTextKey::SettingsPermissionUnavailable
        }
        platform::PermissionStatus::NotDetermined => UiTextKey::SettingsPermissionNotDetermined,
        platform::PermissionStatus::Granted => UiTextKey::SettingsPermissionGranted,
        platform::PermissionStatus::Denied => UiTextKey::SettingsPermissionDenied,
        platform::PermissionStatus::ManagedBySystem => UiTextKey::SettingsPermissionManagedBySystem,
        platform::PermissionStatus::RequestedWhenNeeded => {
            UiTextKey::SettingsPermissionRequestedWhenNeeded
        }
        platform::PermissionStatus::NotRequired => UiTextKey::SettingsPermissionNotRequired,
    }
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
    let profile = root.settings.keybinding_profile;
    let has_diagnostics = !root.settings.keybinding_warning_lines.is_empty();
    let diagnostics = if has_diagnostics {
        root.settings.keybinding_warning_lines.join("; ")
    } else {
        text.get(UiTextKey::SettingsNoKeybindingConflicts)
            .to_string()
    };
    let profile_description = match profile {
        KeybindingProfile::Base => text.get(UiTextKey::SettingsKeybindingProfileBaseDescription),
        KeybindingProfile::Vim => text.get(UiTextKey::SettingsKeybindingProfileVimDescription),
    };
    let leader = root.keybinding_leader().to_string();
    if root.settings.keybinding_rows_cache.is_none() {
        root.settings.keybinding_rows_cache = Some(Rc::new(
            root.settings
                .keybindings_editor
                .rows_for_profile(profile, &text),
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
    let scroll_handle = root.settings.keybinding_scroll_handle.clone();
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
    let selected_profile_index = match profile {
        KeybindingProfile::Base => 0,
        KeybindingProfile::Vim => 1,
    };
    let profile_selector = div()
        .debug_selector(|| "settings-keybinding-profile-selector".to_string())
        .w(px(232.0))
        .flex_none()
        .child(
            TabBar::new("settings-keybinding-profile-tabs")
                .segmented()
                .xsmall()
                .w_full()
                .selected_index(selected_profile_index)
                .on_click(cx.listener(|this, selected_index: &usize, _window, cx| {
                    let profile = if *selected_index == 0 {
                        KeybindingProfile::Base
                    } else {
                        KeybindingProfile::Vim
                    };
                    this.select_keybinding_profile(profile);
                    cx.notify();
                }))
                .child(
                    Tab::new()
                        .debug_selector(|| "settings-keybinding-profile-base".to_string())
                        .flex_1()
                        .label(text.get(UiTextKey::SettingsKeybindingProfileBase)),
                )
                .child(
                    Tab::new()
                        .debug_selector(|| "settings-keybinding-profile-vim".to_string())
                        .flex_1()
                        .label(text.get(UiTextKey::SettingsKeybindingProfileVim)),
                ),
        );
    let profile_toolbar = div()
        .flex()
        .items_center()
        .gap(style.ui_style.spacing.lg)
        .pb(style.ui_style.spacing.lg)
        .border_b(style.ui_style.border.hairline)
        .border_color(theme.border_variant.alpha(0.65))
        .child(profile_selector)
        .child(
            div()
                .min_w_0()
                .flex_1()
                .text_xs()
                .text_color(theme.text_muted)
                .truncate()
                .child(profile_description),
        )
        .child(settings_command_button(
            "settings-keybindings-open",
            text.get(UiTextKey::SettingsOpen),
            true,
            theme,
            CommandId::SettingsKeybindings,
            cx,
        ));

    let mut rows = div()
        .flex()
        .flex_col()
        .size_full()
        .min_h_0()
        .child(profile_toolbar);

    if profile == KeybindingProfile::Vim {
        rows = rows.child(
            div()
                .debug_selector(|| "settings-vim-profile-summary".to_string())
                .flex()
                .items_center()
                .gap(style.ui_style.spacing.md)
                .mt(style.ui_style.spacing.sm)
                .px(style.ui_style.spacing.md)
                .py(style.ui_style.spacing.sm)
                .rounded(style.ui_style.radius.compact)
                .bg(theme.surface.alpha(0.55))
                .child(
                    div()
                        .flex_none()
                        .text_xs()
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(theme.text)
                        .child(text.get(UiTextKey::SettingsVimModeScope)),
                )
                .child(
                    div()
                        .min_w_0()
                        .flex_1()
                        .truncate()
                        .text_xs()
                        .text_color(theme.text_muted)
                        .child(text.get(UiTextKey::SettingsVimModeScopeDescription)),
                )
                .child(settings_keybinding_value(
                    vec![leader],
                    text.get(UiTextKey::SettingsUnbound),
                    theme,
                    style.ui_style,
                )),
        );
    }

    rows.child(
        div()
            .flex()
            .items_center()
            .justify_between()
            .gap(style.ui_style.spacing.lg)
            .h(style.ui_style.controls.toolbar_height)
            .flex_none()
            .child(
                div()
                    .text_sm()
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(theme.text)
                    .child(text.get(UiTextKey::SettingsSectionBindings)),
            )
            .child(
                div()
                    .min_w_0()
                    .truncate()
                    .text_xs()
                    .text_color(if has_diagnostics {
                        theme.warning
                    } else {
                        theme.text_subtle
                    })
                    .child(diagnostics),
            ),
    )
    .child(
        div()
            .debug_selector(|| "settings-keybinding-virtual-list".to_string())
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .border_t(style.ui_style.border.hairline)
            .border_color(theme.border_variant.alpha(0.65))
            .child(virtual_list.track_scroll(&scroll_handle)),
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
    let mut diagnostic_labels = Vec::new();
    if row
        .diagnostics
        .contains(&KeybindingDiagnosticKind::Conflict)
    {
        diagnostic_labels.push(text.get(UiTextKey::SettingsConflict));
    }
    if row
        .diagnostics
        .contains(&KeybindingDiagnosticKind::Shadowed)
    {
        diagnostic_labels.push(text.get(UiTextKey::SettingsKeybindingShadowed));
    }
    if row.diagnostics.contains(&KeybindingDiagnosticKind::Prefix) {
        diagnostic_labels.push(text.get(UiTextKey::SettingsKeybindingPrefixConflict));
    }
    let diagnostic_text = diagnostic_labels.join(" · ");
    let title = div()
        .flex()
        .items_center()
        .gap(style.ui_style.spacing.sm)
        .min_w_0()
        .child(
            div()
                .debug_selector(move || format!("settings-keybinding-title-{row_id}"))
                .min_w_0()
                .flex_1()
                .truncate()
                .text_sm()
                .font_weight(FontWeight::SEMIBOLD)
                .text_color(theme.text)
                .child(row.title),
        )
        .when(!diagnostic_text.is_empty(), |title| {
            title.child(
                div()
                    .flex_none()
                    .text_xs()
                    .text_color(theme.warning)
                    .child(diagnostic_text),
            )
        });
    let label = div()
        .flex()
        .flex_col()
        .gap(style.ui_style.spacing.xs)
        .min_w_0()
        .flex_1()
        .child(title)
        .child(
            div()
                .truncate()
                .text_xs()
                .text_color(theme.text_muted)
                .child(row.description),
        );
    let assignments = settings_keybinding_assignments(
        &row.assignments,
        text.get(UiTextKey::SettingsUnbound),
        theme,
        style.ui_style,
        text,
        style.stack_rows,
    );
    let buttons = div()
        .debug_selector(move || format!("settings-keybinding-actions-{row_id}"))
        .flex()
        .items_center()
        .justify_end()
        .flex_none()
        .child(
            yttt_button(
                format!("settings-keybinding-edit-{row_id}"),
                text.get(UiTextKey::SettingsEdit),
                YtttButtonVariant::Ghost,
                theme,
                style.ui_style,
                cx,
            )
            .on_click(cx.listener(move |this, _, _window, cx| {
                let _ = this.open_keybinding_action_edit_dialog(command);
                cx.notify();
            })),
        );
    let controls = div()
        .flex()
        .items_center()
        .justify_end()
        .gap(style.ui_style.spacing.lg)
        .flex_none()
        .child(
            div()
                .debug_selector(move || format!("settings-keybinding-bindings-{row_id}"))
                .w(px(300.0))
                .child(assignments),
        )
        .child(buttons);
    let binding_row = yttt_row(
        YtttRowKind::Settings,
        SelectableState::Inactive,
        true,
        theme,
        style.ui_style,
    )
    .h_full()
    .w_full()
    .flex();
    let binding_row = if style.stack_rows {
        binding_row
            .flex_col()
            .items_start()
            .gap(style.ui_style.spacing.md)
            .child(label)
            .child(controls.w_full().items_center().justify_between())
    } else {
        binding_row
            .items_center()
            .gap(style.ui_style.spacing.xl)
            .child(label)
            .child(controls)
    };

    binding_row
        .debug_selector(move || format!("settings-keybinding-row-{row_id}"))
        .border_b(style.ui_style.border.hairline)
        .border_color(theme.border_variant.alpha(0.65))
}

fn settings_keybinding_assignments(
    assignments: &[KeybindingAssignment],
    unbound_label: &str,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
    text: UiText,
    align_start: bool,
) -> Div {
    let mut keys = Vec::new();
    let mut metadata = Vec::new();
    let mut shadowed_count = 0;

    for assignment in assignments {
        if assignment.shadowed {
            shadowed_count += 1;
            continue;
        }
        for key in assignment.display_keys() {
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
        let origin = match assignment.origin {
            KeybindingOrigin::Builtin => text.get(UiTextKey::SettingsKeybindingOriginBuiltin),
            KeybindingOrigin::User => text.get(UiTextKey::SettingsKeybindingOriginUser),
            KeybindingOrigin::Inherited => text.get(UiTextKey::SettingsKeybindingOriginInherited),
        };
        let label = format!("{origin} · {}", assignment.scope_label());
        if !metadata.contains(&label) {
            metadata.push(label);
        }
    }
    if shadowed_count > 0 {
        metadata.push(format!(
            "{shadowed_count} {}",
            text.get(UiTextKey::SettingsKeybindingShadowed)
        ));
    }

    let metadata = metadata.join("  /  ");
    div()
        .flex()
        .flex_col()
        .items_end()
        .gap(ui_style.spacing.xxs)
        .when(align_start, |value| value.items_start())
        .child(settings_keybinding_value(
            keys,
            unbound_label,
            theme,
            ui_style,
        ))
        .when(!metadata.is_empty(), |value| {
            value.child(
                div()
                    .max_w_full()
                    .truncate()
                    .text_xs()
                    .text_color(theme.text_subtle)
                    .child(metadata),
            )
        })
}

fn scoped_settings_row(
    root: &WorkbenchView,
    key: &'static str,
    style: YtttSettingsLayout,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    let scope_label = match root.selected_settings_scope() {
        SettingsScope::Device => root.ui_text.get(UiTextKey::SettingsScopeDevice),
        SettingsScope::Host => root.ui_text.get(UiTextKey::SettingsScopeHost),
        SettingsScope::Project => root.ui_text.get(UiTextKey::SettingsScopeProject),
    };
    let apply_label = match setting_apply(key) {
        SettingApply::Immediate => root.ui_text.get(UiTextKey::SettingsApplyImmediate),
        SettingApply::NewSession => root.ui_text.get(UiTextKey::SettingsApplyNewSession),
        SettingApply::ReopenFile => root.ui_text.get(UiTextKey::SettingsApplyReopenFile),
    };
    let description = format!("{} · {scope_label} · {apply_label}", description.into());
    let control = if root.settings_scope_is_editable() {
        control
    } else {
        settings_value(read_only_setting_value(root, key), theme, style.ui_style).into_any_element()
    };
    setting_row(style, theme, title, description, control)
}

fn scoped_settings_block(
    root: &WorkbenchView,
    key: &'static str,
    mut style: YtttSettingsLayout,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    style.stack_rows = true;
    scoped_settings_row(root, key, style, theme, title, description, control)
}

fn read_only_setting_value(root: &WorkbenchView, key: &str) -> String {
    use crate::config::project_settings::{
        ProjectEditorSettingKey as Key, ProjectEditorSettingValue as Value,
    };
    if root.selected_settings_scope() == SettingsScope::Project {
        let project_key = match key {
            "editor.tab_size" => Some(Key::TabSize),
            "editor.auto_detect_language" => Some(Key::AutoDetectLanguage),
            "editor.default_language" => Some(Key::DefaultLanguage),
            _ => None,
        };
        if let Some(setting) = project_key.and_then(|key| root.cached_project_editor_setting(key)) {
            return match &setting.value {
                Value::TabSize(value) => value.to_string(),
                Value::AutoDetectLanguage(value) => value.to_string(),
                Value::DefaultLanguage(value) => value.clone(),
            };
        }
    }
    let settings = &root.app_settings;
    match key {
        "general.new_tab_commands" => settings.general.new_tab_commands.join("\n"),
        "terminal.shell" => settings.terminal.shell.clone(),
        "terminal.custom_shells" => settings.terminal.custom_shells.join("\n"),
        "terminal.environment" => settings
            .terminal
            .environment
            .iter()
            .map(|(key, value)| format!("{key}={value}"))
            .collect::<Vec<_>>()
            .join("\n"),
        "terminal.scrollback" => settings.terminal.scrollback.to_string(),
        "terminal.kitty_keyboard" => settings.terminal.kitty_keyboard.to_string(),
        "editor.tab_size" => settings.editor.tab_size.to_string(),
        "editor.auto_detect_language" => settings.editor.auto_detect_language.to_string(),
        "editor.default_language" => settings.editor.default_language.clone(),
        "editor.lsp.enabled" => settings.editor.lsp.enabled.to_string(),
        "editor.lsp.command" => settings.editor.lsp.command.clone(),
        "agent.primary" => root.primary_agent().display_name().to_string(),
        "agent.sessions_enabled" => settings.agent.sessions_enabled.to_string(),
        _ => root.ui_text.get(UiTextKey::SettingsUnbound).to_string(),
    }
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
        .flex_col()
        .gap(style.ui_style.spacing.sm)
        .when(!first, |this| this.pt(style.section_gap))
        .pb(style.ui_style.spacing.sm)
        .child(
            div()
                .text_xs()
                .text_color(theme.text_muted)
                .child(title.into()),
        )
        .child(
            div()
                .w_full()
                .h(style.ui_style.border.hairline)
                .bg(theme.border_variant),
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

fn settings_bar_modules_control(
    prefix: &'static str,
    inputs: &[Entity<InputState>; 3],
    text: UiText,
    theme: WorkbenchTheme,
    style: YtttSettingsLayout,
) -> Div {
    let labels = [
        text.get(UiTextKey::SettingsBarLeft),
        text.get(UiTextKey::SettingsBarCenter),
        text.get(UiTextKey::SettingsBarRight),
    ];
    let regions = ["left", "center", "right"];

    div()
        .flex()
        .flex_col()
        .gap(style.ui_style.spacing.sm)
        .w_full()
        .max_w(px(720.0))
        .children(
            inputs
                .iter()
                .zip(labels)
                .zip(regions)
                .map(|((input, label), region)| {
                    let selector = format!("{prefix}-{region}");
                    div()
                        .debug_selector(move || selector.clone())
                        .flex()
                        .items_center()
                        .gap(style.ui_style.spacing.md)
                        .w_full()
                        .child(
                            div()
                                .w(px(56.0))
                                .flex_none()
                                .text_xs()
                                .text_color(theme.text_muted)
                                .child(label),
                        )
                        .child(
                            div().flex_1().min_w_0().h(style.control_height).child(
                                yttt_input(input, YtttInputKind::Settings, theme, style.ui_style)
                                    .small(),
                            ),
                        )
                }),
        )
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
        return div()
            .flex()
            .items_center()
            .justify_end()
            .text_xs()
            .text_color(theme.text_subtle)
            .child(unbound_label.into());
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
