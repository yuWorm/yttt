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
        CommandId::CommandPaletteOpen
            | CommandId::ProjectOpenRecent
            | CommandId::ProjectPalette
            | CommandId::TabPalette
            | CommandId::PanePalette
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
            terminal_input_gate: TerminalInputGate::default(),
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
                contexts,
            );
        }
    }
}

pub(super) fn recent_projects_for_palette(config: RecentProjectsConfig) -> Vec<RecentProject> {
    config
        .projects
        .into_iter()
        .map(|project| RecentProject {
            title: project.title,
            path: project.path,
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

pub(super) fn language_setting_labels() -> Vec<String> {
    [
        LanguageSetting::System,
        LanguageSetting::English,
        LanguageSetting::Chinese,
    ]
    .into_iter()
    .map(language_setting_label)
    .map(ToString::to_string)
    .collect()
}

pub(super) fn language_setting_label(language: LanguageSetting) -> &'static str {
    match language {
        LanguageSetting::System => "System",
        LanguageSetting::English => "English",
        LanguageSetting::Chinese => "中文",
    }
}

pub(super) fn language_setting_from_label(label: &str) -> Option<LanguageSetting> {
    match label {
        "System" => Some(LanguageSetting::System),
        "English" => Some(LanguageSetting::English),
        "中文" => Some(LanguageSetting::Chinese),
        _ => None,
    }
}

pub(super) fn parse_keybinding_edit_value(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(ToString::to_string)
        .collect()
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

pub(super) fn layout_source_message(source: &LayoutSource) -> String {
    let source_name = match source {
        LayoutSource::GlobalDefault(_) => "global default",
        LayoutSource::GlobalDefaultWithPersonalPatch { .. } => "global default + personal patch",
        LayoutSource::ProjectConfig(_) => "project config",
        LayoutSource::ProjectConfigWithPersonalPatch { .. } => "project config + personal patch",
        LayoutSource::PersonalReplace(_) => "personal replacement",
    };

    format!("Layout source: {source_name}")
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
) -> (Option<String>, Vec<String>) {
    match load_keybindings(paths, registry) {
        Ok(loaded) if loaded.warnings.is_empty() => (None, Vec::new()),
        Ok(loaded) => {
            let lines = format_keybinding_warning_lines(&loaded.warnings);
            (Some(format!("Keybindings: {}", lines.join("; "))), lines)
        }
        Err(error) => (Some(error.to_string()), Vec::new()),
    }
}

pub(super) fn load_keybindings_editor_state(
    paths: &AppConfigPaths,
    registry: &CommandRegistry,
) -> KeybindingsEditorState {
    let config = load_keybindings(paths, registry)
        .map(|loaded| loaded.config)
        .unwrap_or_else(|_| crate::config::keybindings::default_keybindings());
    KeybindingsEditorState::new(config, registry.clone())
}

pub(super) fn load_app_settings_messages(paths: &AppConfigPaths) -> (AppSettings, Vec<String>) {
    let mut warnings = Vec::new();
    let settings = match load_or_create_settings(paths) {
        Ok(loaded) => {
            warnings.extend(loaded.warnings.iter().map(format_settings_warning_line));
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
    let theme_store = match load_theme_store(paths) {
        Ok(loaded) => {
            warnings.extend(loaded.warnings.iter().map(format_theme_warning_line));
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

pub(super) fn format_settings_warning_line(warning: &SettingsLoadWarning) -> String {
    match warning {
        SettingsLoadWarning::InvalidToml { path, message } => {
            format!("Settings {}: {message}", path.display())
        }
        SettingsLoadWarning::InvalidGeneralValue { field } => {
            format!("Settings general.{field} is invalid; using default")
        }
        SettingsLoadWarning::InvalidTerminalValue { field } => {
            format!("Settings terminal.{field} is invalid; using default")
        }
        SettingsLoadWarning::InvalidEditorValue { field } => {
            format!("Settings editor.{field} is invalid; using default")
        }
        SettingsLoadWarning::InvalidProjectPanelValue { field } => {
            format!("Settings project_panel.{field} is invalid; using default")
        }
    }
}

pub(super) fn format_theme_warning_line(warning: &ThemeLoadWarning) -> String {
    match warning {
        ThemeLoadWarning::ReadDir { path, message } => {
            format!("Themes {}: {message}", path.display())
        }
        ThemeLoadWarning::ReadFile { path, message } => {
            format!("Theme {}: {message}", path.display())
        }
        ThemeLoadWarning::ParseFile { path, message } => {
            format!("Theme {}: {message}", path.display())
        }
        ThemeLoadWarning::InvalidColor { theme, field } => {
            format!("Theme {theme} has invalid color {field}; using fallback")
        }
    }
}

pub(super) fn ui_text_for_language(language: LanguageSetting) -> UiText {
    match language {
        LanguageSetting::System | LanguageSetting::English => UiText::english(),
        LanguageSetting::Chinese => UiText::new(Locale::Chinese),
    }
}

pub(super) fn format_keybinding_warning_lines(warnings: &[KeybindingLoadWarning]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| match warning {
            KeybindingLoadWarning::Conflict(conflict) => {
                format!("Conflicting keybinding: {}", conflict.keys)
            }
            KeybindingLoadWarning::InvalidCommand(command) => {
                format!("Invalid command id: {command}")
            }
        })
        .collect()
}
