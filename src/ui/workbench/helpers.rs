use super::*;

pub(super) fn terminal_pane_key(project_id: &str, tab_id: &str, pane_id: &str) -> String {
    format!("{project_id}:{tab_id}:{pane_id}")
}

pub(super) fn collect_terminal_pane_keys(
    project_id: &str,
    tab_id: &str,
    layout: &LayoutNode,
    keys: &mut HashSet<String>,
) {
    match layout {
        LayoutNode::Pane(pane) => {
            keys.insert(terminal_pane_key(project_id, tab_id, &pane.id));
        }
        LayoutNode::Split(split) => {
            collect_terminal_pane_keys(project_id, tab_id, &split.left, keys);
            collect_terminal_pane_keys(project_id, tab_id, &split.right, keys);
        }
    }
}

pub(super) fn opens_palette_command(command_id: CommandId) -> bool {
    matches!(
        command_id,
        CommandId::FileFind
            | CommandId::CommandPaletteOpen
            | CommandId::ProjectOpenRecent
            | CommandId::ProjectPalette
            | CommandId::ProjectOpenedPalette
            | CommandId::TabPalette
            | CommandId::PanePalette
            | CommandId::GitBranchSwitch
    )
}

pub(super) fn collect_terminal_pane_contexts(
    project_id: &str,
    project_path: &Path,
    project_title: &str,
    tab_id: &str,
    tab_title: &str,
    layout: &LayoutNode,
    focused_pane_id: Option<&str>,
    terminal_input_gate: &TerminalInputGate,
    contexts: &mut Vec<TerminalPaneContext>,
) {
    match layout {
        LayoutNode::Pane(pane) => contexts.push(TerminalPaneContext {
            project_id: project_id.to_string(),
            project_path: project_path.to_path_buf(),
            project_title: project_title.to_string(),
            tab_id: tab_id.to_string(),
            tab_title: tab_title.to_string(),
            pane: pane.clone(),
            is_focused: focused_pane_id == Some(pane.id.as_str()),
            terminal_input_gate: terminal_input_gate.clone(),
        }),
        LayoutNode::Split(split) => {
            collect_terminal_pane_contexts(
                project_id,
                project_path,
                project_title,
                tab_id,
                tab_title,
                &split.left,
                focused_pane_id,
                terminal_input_gate,
                contexts,
            );
            collect_terminal_pane_contexts(
                project_id,
                project_path,
                project_title,
                tab_id,
                tab_title,
                &split.right,
                focused_pane_id,
                terminal_input_gate,
                contexts,
            );
        }
    }
}

pub(super) fn recent_projects_for_palette(config: &RecentProjectsConfig) -> Vec<RecentProject> {
    config
        .projects
        .iter()
        .map(|project| RecentProject {
            id: project.id.clone(),
            title: project.title.clone(),
            location: project.location.clone(),
        })
        .collect()
}

pub(super) fn push_unique_string(values: &mut Vec<String>, value: String) {
    if values.iter().all(|existing| existing != &value) {
        values.push(value);
    }
}

pub(super) fn selected_index_for_settings_option(
    items: &[String],
    selected: &str,
) -> Option<IndexPath> {
    items
        .iter()
        .position(|item| item == selected)
        .map(|index| IndexPath::default().row(index))
}

pub(super) fn language_setting_labels(text: UiText) -> Vec<String> {
    [
        LanguageSetting::System,
        LanguageSetting::English,
        LanguageSetting::Chinese,
    ]
    .into_iter()
    .map(|language| language_setting_label(language, text).to_string())
    .collect()
}

pub(super) fn language_setting_label(language: LanguageSetting, text: UiText) -> &'static str {
    match language {
        LanguageSetting::System => text.get(UiTextKey::SettingsLanguageSystem),
        LanguageSetting::English => "English",
        LanguageSetting::Chinese => "中文",
    }
}

pub(super) fn language_setting_from_label(label: &str, text: UiText) -> Option<LanguageSetting> {
    [
        LanguageSetting::System,
        LanguageSetting::English,
        LanguageSetting::Chinese,
    ]
    .into_iter()
    .find(|language| language_setting_label(*language, text) == label)
}

pub(super) fn terminal_cursor_shape_labels(text: UiText) -> Vec<String> {
    [
        TerminalCursorShape::Block,
        TerminalCursorShape::Underline,
        TerminalCursorShape::Beam,
    ]
    .into_iter()
    .map(|shape| terminal_cursor_shape_label(shape, text).to_string())
    .collect()
}

pub(super) fn terminal_cursor_shape_label(
    shape: TerminalCursorShape,
    text: UiText,
) -> &'static str {
    text.get(match shape {
        TerminalCursorShape::Block => UiTextKey::SettingsCursorBlock,
        TerminalCursorShape::Underline => UiTextKey::SettingsCursorUnderline,
        TerminalCursorShape::Beam => UiTextKey::SettingsCursorBeam,
    })
}

pub(super) fn terminal_cursor_shape_from_label(
    label: &str,
    text: UiText,
) -> Option<TerminalCursorShape> {
    [
        TerminalCursorShape::Block,
        TerminalCursorShape::Underline,
        TerminalCursorShape::Beam,
    ]
    .into_iter()
    .find(|shape| terminal_cursor_shape_label(*shape, text) == label)
}

pub(super) fn terminal_osc52_policy_labels(text: UiText) -> Vec<String> {
    [
        TerminalOsc52Policy::Disabled,
        TerminalOsc52Policy::CopyOnly,
        TerminalOsc52Policy::ReadWrite,
    ]
    .into_iter()
    .map(|policy| terminal_osc52_policy_label(policy, text).to_string())
    .collect()
}

pub(super) fn terminal_osc52_policy_label(
    policy: TerminalOsc52Policy,
    text: UiText,
) -> &'static str {
    text.get(match policy {
        TerminalOsc52Policy::Disabled => UiTextKey::SettingsOptionDisabled,
        TerminalOsc52Policy::CopyOnly => UiTextKey::SettingsOsc52CopyOnly,
        TerminalOsc52Policy::ReadWrite => UiTextKey::SettingsOsc52ReadWrite,
    })
}

pub(super) fn terminal_osc52_policy_from_label(
    label: &str,
    text: UiText,
) -> Option<TerminalOsc52Policy> {
    [
        TerminalOsc52Policy::Disabled,
        TerminalOsc52Policy::CopyOnly,
        TerminalOsc52Policy::ReadWrite,
    ]
    .into_iter()
    .find(|policy| terminal_osc52_policy_label(*policy, text) == label)
}

pub(super) fn vim_mode_label(mode: VimModeSetting, text: UiText) -> &'static str {
    text.get(match mode {
        VimModeSetting::Global => UiTextKey::SettingsVimGlobal,
        VimModeSetting::Editor => UiTextKey::SettingsVimEditorOnly,
        VimModeSetting::Disabled => UiTextKey::SettingsOptionDisabled,
    })
}

pub(super) fn should_focus_terminal_after_command(command_id: CommandId) -> bool {
    matches!(
        command_id,
        CommandId::TabNew
            | CommandId::TabClose
            | CommandId::TabNext
            | CommandId::TabPrev
            | CommandId::PaneSplitVertical
            | CommandId::PaneSplitHorizontal
            | CommandId::PaneClose
            | CommandId::PaneFocusLeft
            | CommandId::PaneFocusRight
            | CommandId::PaneFocusUp
            | CommandId::PaneFocusDown
    )
}

pub(super) fn layout_source_message(source: &LayoutSource, text: UiText) -> String {
    let key = match source {
        LayoutSource::GlobalDefault(_) => UiTextKey::LayoutSourceGlobal,
        LayoutSource::GlobalDefaultWithPersonalPatch { .. } => UiTextKey::LayoutSourceGlobalPatched,
        LayoutSource::ProjectConfig(_) => UiTextKey::LayoutSourceProject,
        LayoutSource::ProjectConfigWithPersonalPatch { .. } => {
            UiTextKey::LayoutSourceProjectPatched
        }
        LayoutSource::PersonalReplace(_) => UiTextKey::LayoutSourcePersonal,
    };
    text.get(UiTextKey::LayoutSourceLabel)
        .replace("{source}", text.get(key))
}

pub(super) fn layout_load_warning_message(warnings: &[LayoutLoadWarning]) -> Option<String> {
    (!warnings.is_empty()).then(|| {
        warnings
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join("; ")
    })
}

pub(super) fn load_keybindings_messages(
    paths: &AppConfigPaths,
    registry: &CommandRegistry,
    text: &UiText,
) -> (Option<String>, Vec<String>) {
    match load_keybindings(paths, registry) {
        Ok(loaded) if loaded.warnings.is_empty() => (None, Vec::new()),
        Ok(loaded) => {
            let lines = format_keybinding_warning_lines(&loaded.warnings, text);
            (
                Some(format!(
                    "{}: {}",
                    text.get(UiTextKey::StatusKeybindingsFile),
                    lines.join("; ")
                )),
                lines,
            )
        }
        Err(error) => (
            Some(format!(
                "{}: {error}",
                text.get(UiTextKey::StatusKeybindingsFile)
            )),
            Vec::new(),
        ),
    }
}

pub(super) fn load_keybindings_editor_state(
    paths: &AppConfigPaths,
    registry: &CommandRegistry,
    text: &UiText,
) -> KeybindingsEditorState {
    match load_keybindings(paths, registry) {
        Ok(loaded) if loaded.warnings.is_empty() => {
            KeybindingsEditorState::new(loaded.config, registry.clone())
        }
        Ok(loaded) => {
            let warning = format_keybinding_warning_lines(&loaded.warnings, text).join("; ");
            KeybindingsEditorState::with_load_error(
                crate::config::keybindings::default_keybindings(),
                registry.clone(),
                warning,
            )
        }
        Err(error) => KeybindingsEditorState::with_load_error(
            crate::config::keybindings::default_keybindings(),
            registry.clone(),
            error.to_string(),
        ),
    }
}

pub(super) fn load_app_settings_messages(paths: &AppConfigPaths) -> (AppSettings, Vec<String>) {
    let mut warnings = Vec::new();
    let settings = match load_settings(paths) {
        Ok(loaded) => {
            let text = UiText::from_language(loaded.settings.general.language);
            warnings.extend(
                loaded
                    .warnings
                    .iter()
                    .map(|warning| format_settings_warning_line(warning, text)),
            );
            loaded.settings
        }
        Err(error) => {
            warnings.push(error.to_string());
            AppSettings::default()
        }
    };

    (settings, warnings)
}

pub(super) fn load_theme_runtime_messages(
    paths: &AppConfigPaths,
    settings: &AppSettings,
) -> (ThemeRuntime, Vec<String>) {
    let mut warnings = Vec::new();
    let text = UiText::from_language(settings.general.language);
    let theme_store = match load_theme_store(paths) {
        Ok(loaded) => {
            warnings.extend(
                loaded
                    .warnings
                    .iter()
                    .map(|warning| format_theme_warning_line(warning, text)),
            );
            loaded.store
        }
        Err(error) => {
            warnings.push(error.to_string());
            ThemeStore::builtin()
        }
    };

    (ThemeRuntime::resolve(settings, &theme_store), warnings)
}

pub(super) fn combine_load_messages(left: Option<String>, right: Option<String>) -> Option<String> {
    match (left, right) {
        (Some(left), Some(right)) => Some(format!("{left}; {right}")),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

pub(super) fn format_settings_warning_line(warning: &SettingsLoadWarning, text: UiText) -> String {
    let (group, field) = match warning {
        SettingsLoadWarning::InvalidToml { path, message } => {
            return text
                .get(UiTextKey::SettingsWarningFile)
                .replace("{path}", &path.display().to_string())
                .replace("{error}", message);
        }
        SettingsLoadWarning::InvalidBarsValue { field, value } => {
            return text
                .get(UiTextKey::SettingsWarningBars)
                .replace("{field}", field)
                .replace("{value}", &format!("{value:?}"));
        }
        SettingsLoadWarning::InvalidGeneralValue { field } => ("general", field),
        SettingsLoadWarning::InvalidWindowValue { field } => ("window", field),
        SettingsLoadWarning::InvalidThemeValue { field } => ("theme", field),
        SettingsLoadWarning::InvalidTerminalValue { field } => ("terminal", field),
        SettingsLoadWarning::InvalidEditorValue { field } => ("editor", field),
        SettingsLoadWarning::InvalidVimValue { field } => ("vim", field),
        SettingsLoadWarning::InvalidProjectPanelValue { field } => ("project_panel", field),
    };
    text.get(UiTextKey::SettingsWarningInvalidValue)
        .replace("{group}", group)
        .replace("{field}", field)
}

pub(super) fn format_theme_warning_line(warning: &ThemeLoadWarning, text: UiText) -> String {
    match warning {
        ThemeLoadWarning::ReadDir { path, message }
        | ThemeLoadWarning::ReadFile { path, message }
        | ThemeLoadWarning::ParseFile { path, message } => text
            .get(UiTextKey::ThemeWarningFile)
            .replace("{path}", &path.display().to_string())
            .replace("{error}", message),
        ThemeLoadWarning::InvalidColor { theme, field } => text
            .get(UiTextKey::ThemeWarningColor)
            .replace("{theme}", theme)
            .replace("{field}", field),
    }
}

pub(super) fn ui_text_for_language(language: LanguageSetting) -> UiText {
    UiText::from_language(language)
}

pub(super) fn markdown_editor_strings_for_language(
    language: LanguageSetting,
) -> Arc<gpui_markdown_editor::MarkdownEditorStrings> {
    Arc::new(match UiText::from_language(language).locale() {
        Locale::English => gpui_markdown_editor::MarkdownEditorStrings::en_us(),
        Locale::Chinese => gpui_markdown_editor::MarkdownEditorStrings::zh_cn(),
    })
}

pub(super) fn format_keybinding_warning_lines(
    warnings: &[KeybindingLoadWarning],
    text: &UiText,
) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| match warning {
            KeybindingLoadWarning::Conflict(conflict) => format!(
                "{}: {}",
                text.get(UiTextKey::SettingsConflictingKeybinding),
                conflict.keys
            ),
            KeybindingLoadWarning::InvalidCommand(command) => format!(
                "{}: {command}",
                text.get(UiTextKey::SettingsInvalidCommandId)
            ),
            KeybindingLoadWarning::InvalidBinding(message) => message.clone(),
        })
        .collect()
}
