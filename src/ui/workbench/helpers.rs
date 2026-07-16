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
    shell: &str,
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
            shell: shell.to_string(),
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
                shell,
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
                shell,
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
            title: project.title.clone(),
            path: project.path.clone(),
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

pub(super) fn terminal_cursor_shape_labels() -> Vec<String> {
    [
        TerminalCursorShape::Block,
        TerminalCursorShape::Underline,
        TerminalCursorShape::Beam,
    ]
    .into_iter()
    .map(terminal_cursor_shape_label)
    .map(ToString::to_string)
    .collect()
}

pub(super) fn terminal_cursor_shape_label(shape: TerminalCursorShape) -> &'static str {
    match shape {
        TerminalCursorShape::Block => "Block",
        TerminalCursorShape::Underline => "Underline",
        TerminalCursorShape::Beam => "Beam",
    }
}

pub(super) fn terminal_cursor_shape_from_label(label: &str) -> Option<TerminalCursorShape> {
    match label {
        "Block" => Some(TerminalCursorShape::Block),
        "Underline" => Some(TerminalCursorShape::Underline),
        "Beam" => Some(TerminalCursorShape::Beam),
        _ => None,
    }
}

pub(super) fn terminal_osc52_policy_labels() -> Vec<String> {
    [
        TerminalOsc52Policy::Disabled,
        TerminalOsc52Policy::CopyOnly,
        TerminalOsc52Policy::ReadWrite,
    ]
    .into_iter()
    .map(terminal_osc52_policy_label)
    .map(ToString::to_string)
    .collect()
}

pub(super) fn terminal_osc52_policy_label(policy: TerminalOsc52Policy) -> &'static str {
    match policy {
        TerminalOsc52Policy::Disabled => "Disabled",
        TerminalOsc52Policy::CopyOnly => "Copy only",
        TerminalOsc52Policy::ReadWrite => "Read and write",
    }
}

pub(super) fn terminal_osc52_policy_from_label(label: &str) -> Option<TerminalOsc52Policy> {
    match label {
        "Disabled" => Some(TerminalOsc52Policy::Disabled),
        "Copy only" => Some(TerminalOsc52Policy::CopyOnly),
        "Read and write" => Some(TerminalOsc52Policy::ReadWrite),
        _ => None,
    }
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
        SettingsLoadWarning::InvalidWindowValue { field } => {
            format!("Settings window.{field} is invalid; using default")
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

pub(super) fn markdown_editor_strings_for_language(
    language: LanguageSetting,
) -> Arc<gpui_markdown_editor::MarkdownEditorStrings> {
    Arc::new(match language {
        LanguageSetting::System | LanguageSetting::English => {
            gpui_markdown_editor::MarkdownEditorStrings::en_us()
        }
        LanguageSetting::Chinese => gpui_markdown_editor::MarkdownEditorStrings::zh_cn(),
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
        })
        .collect()
}
