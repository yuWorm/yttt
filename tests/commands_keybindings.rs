use std::path::PathBuf;

use gpui::Keystroke;
use tempfile::tempdir;
use yttt::commands::{
    ActiveSurface, CommandContext, CommandId, default_registry, dispatch_workspace_command,
};
use yttt::config::{
    keybindings::{
        KEYBINDINGS_SCHEMA_VERSION, Keybinding, KeybindingLoadWarning, KeybindingsConfig,
        default_keybindings, load_keybindings, save_keybindings,
    },
    paths::AppConfigPaths,
};
use yttt::model::layout::LayoutNode;
use yttt::model::workspace::{TabStartState, Workspace};
use yttt::ui::interaction::actions::{
    app_startup_keybindings, default_ui_keybinding_specs, runtime_command_for_keystroke,
};
use yttt::ui::interaction::{
    input_owner::InputOwnerKind, key_dispatch::workspace_command_for_keystroke,
};
use yttt::ui::settings::keybinding_display::{
    KeybindingDisplayPlatform, display_keybindings_for_platform,
};
use yttt::ui::settings::keybindings::{KeybindingEditError, KeybindingsEditorState};
use yttt::ui::workbench::shell::split_view::visible_pane_titles;

#[test]
fn default_registry_contains_core_commands() {
    let registry = default_registry();

    assert!(registry.contains(CommandId::ProjectOpen));
    assert!(registry.contains(CommandId::PaneSplitVertical));
    assert!(registry.contains(CommandId::TabRename));
    assert!(registry.contains(CommandId::CommandPaletteOpen));
    assert!(registry.contains(CommandId::SettingsOpen));
}

#[test]
fn file_and_project_panel_commands_are_registered() {
    let registry = default_registry();

    for (command, id) in [
        (CommandId::FileSave, "file.save"),
        (CommandId::ProjectPanelToggle, "project_panel.toggle"),
        (CommandId::ProjectPanelRefresh, "project_panel.refresh"),
    ] {
        assert!(registry.contains(command));
        assert_eq!(command.as_str(), id);
    }

    let config = default_keybindings();
    assert_has_config_binding(&config, "cmd-s", "file.save");
    assert_has_config_binding(&config, "ctrl-s", "file.save");
    assert_has_config_binding(&config, "cmd-shift-e", "project_panel.toggle");
    assert_has_config_binding(&config, "ctrl-shift-e", "project_panel.toggle");
    assert_has_ui_binding("cmd-s", "file.save");
    assert_has_ui_binding("ctrl-s", "file.save");
    assert_has_ui_binding("cmd-shift-e", "project_panel.toggle");
    assert_has_ui_binding("ctrl-shift-e", "project_panel.toggle");
    assert!(config.conflicts().is_empty());
}

#[test]
fn command_availability_tracks_active_surface() {
    let no_project = CommandContext {
        has_selected_project: false,
        active_surface: ActiveSurface::None,
    };
    let no_surface = CommandContext {
        has_selected_project: true,
        active_surface: ActiveSurface::None,
    };
    let terminal = CommandContext {
        has_selected_project: true,
        active_surface: ActiveSurface::Terminal,
    };
    let file = CommandContext {
        has_selected_project: true,
        active_surface: ActiveSurface::File,
    };

    assert!(
        !CommandId::FileSave
            .availability_for_context(no_project)
            .enabled
    );
    assert!(
        !CommandId::FileSave
            .availability_for_context(no_surface)
            .enabled
    );
    assert!(
        !CommandId::FileSave
            .availability_for_context(terminal)
            .enabled
    );
    assert!(CommandId::FileSave.availability_for_context(file).enabled);
    assert!(!CommandId::FileSave.availability(true).enabled);

    for command in [
        CommandId::ProjectPanelToggle,
        CommandId::ProjectPanelRefresh,
    ] {
        assert!(!command.availability_for_context(no_project).enabled);
        assert!(command.availability_for_context(no_surface).enabled);
        assert!(command.availability_for_context(terminal).enabled);
        assert!(command.availability_for_context(file).enabled);
    }

    assert!(
        !CommandId::TabNew
            .availability_for_context(no_project)
            .enabled
    );
    assert!(
        CommandId::TabNew
            .availability_for_context(no_surface)
            .enabled
    );
    assert!(CommandId::TabNew.availability_for_context(terminal).enabled);
    assert!(!CommandId::TabNew.availability_for_context(file).enabled);

    for command in [CommandId::TabClose, CommandId::TabNext, CommandId::TabPrev] {
        assert!(!command.availability_for_context(no_surface).enabled);
        assert!(command.availability_for_context(terminal).enabled);
        assert!(command.availability_for_context(file).enabled);
    }

    for command in [
        CommandId::TabRename,
        CommandId::PaneSplitHorizontal,
        CommandId::PaneSplitVertical,
        CommandId::PaneClose,
        CommandId::PaneFocusLeft,
        CommandId::PaneFocusRight,
        CommandId::PaneFocusUp,
        CommandId::PaneFocusDown,
        CommandId::PaneResizeLeft,
        CommandId::PaneResizeRight,
        CommandId::PaneResizeUp,
        CommandId::PaneResizeDown,
        CommandId::PaneRename,
        CommandId::PanePalette,
    ] {
        assert!(command.availability_for_context(terminal).enabled);
        assert!(!command.availability_for_context(file).enabled);
    }
}

#[test]
fn editor_owner_allows_only_safe_workspace_commands() {
    for command in [
        CommandId::FileSave,
        CommandId::TabClose,
        CommandId::TabNext,
        CommandId::TabPrev,
        CommandId::ProjectPanelToggle,
        CommandId::ProjectPanelRefresh,
        CommandId::CommandPaletteOpen,
        CommandId::ProjectPalette,
        CommandId::TabPalette,
    ] {
        let actual = workspace_command_for_keystroke(
            InputOwnerKind::Editor,
            &Keystroke::parse("cmd-s").unwrap(),
            |_| Some(command),
            |_| true,
        );
        assert_eq!(actual, Some(command), "{command:?} should be editor-safe");
    }

    for command in [
        CommandId::TabNew,
        CommandId::TabRename,
        CommandId::PaneSplitHorizontal,
        CommandId::PaneSplitVertical,
        CommandId::PaneClose,
        CommandId::PaneFocusLeft,
        CommandId::PaneResizeRight,
        CommandId::PaneRename,
        CommandId::PanePalette,
    ] {
        let actual = workspace_command_for_keystroke(
            InputOwnerKind::Editor,
            &Keystroke::parse("cmd-d").unwrap(),
            |_| Some(command),
            |_| false,
        );
        assert_eq!(actual, None, "{command:?} must stay editor-owned");
    }
}

#[test]
fn modal_input_owners_block_project_file_save() {
    for owner in [
        InputOwnerKind::Settings,
        InputOwnerKind::Dialog,
        InputOwnerKind::Palette,
    ] {
        let actual = workspace_command_for_keystroke(
            owner,
            &Keystroke::parse("cmd-s").unwrap(),
            |_| Some(CommandId::FileSave),
            |_| false,
        );
        assert_eq!(actual, None, "{owner:?} must block project-file save");
    }
}

#[test]
fn notification_settings_command_is_available_without_project() {
    let availability = CommandId::SettingsNotifications.availability(false);

    assert!(availability.enabled);
    assert!(availability.disabled_reason.is_none());
}

#[test]
fn settings_open_command_is_available_without_project() {
    let availability = CommandId::SettingsOpen.availability(false);

    assert!(availability.enabled);
    assert!(availability.disabled_reason.is_none());
    assert_eq!(CommandId::SettingsOpen.as_str(), "settings.open");
    assert_eq!(
        CommandId::SettingsOpen.presentation().title,
        "Open Settings"
    );
}

#[test]
fn layout_default_commands_are_registered_and_available_without_project() {
    let registry = default_registry();

    for (command, id) in [
        (CommandId::LayoutDefaultEdit, "layout.default.edit"),
        (CommandId::LayoutDefaultReset, "layout.default.reset"),
        (CommandId::LayoutDefaultReload, "layout.default.reload"),
    ] {
        assert!(registry.contains(command));
        assert_eq!(command.as_str(), id);
        assert!(command.availability(false).enabled);
    }
}

#[test]
fn layout_project_commands_require_selected_project() {
    for (command, id) in [
        (CommandId::LayoutProjectEdit, "layout.project.edit"),
        (
            CommandId::LayoutResetLocalOverride,
            "layout.reset_local_override",
        ),
    ] {
        assert_eq!(command.as_str(), id);
        assert!(!command.availability(false).enabled);
        assert_eq!(
            command.availability(false).disabled_reason,
            Some("Open a project first")
        );
        assert!(command.availability(true).enabled);
    }
}

#[test]
fn parses_keybinding_toml() {
    let source = r#"
        [[bindings]]
        keys = "cmd-p"
        command = "command_palette.open"
    "#;

    let config: KeybindingsConfig = toml::from_str(source).unwrap();

    assert_eq!(config.bindings.len(), 1);
    assert_eq!(config.bindings[0].keys, "cmd-p");
    assert_eq!(config.bindings[0].command, "command_palette.open");
}

#[test]
fn detects_duplicate_keybindings() {
    let source = r#"
        [[bindings]]
        keys = "cmd-p"
        command = "command_palette.open"

        [[bindings]]
        keys = "CMD-P"
        command = "project.palette"
    "#;
    let config: KeybindingsConfig = toml::from_str(source).unwrap();

    let conflicts = config.conflicts();

    assert_eq!(conflicts.len(), 1);
    assert_eq!(conflicts[0].keys, "cmd-p");
}

#[test]
fn default_keybindings_include_palette_shortcuts() {
    let config = default_keybindings();

    assert_has_config_binding(&config, "cmd-p", "command_palette.open");
    assert_has_config_binding(&config, "ctrl-k", "pane.palette");
}

#[test]
fn default_keybindings_include_settings_shortcuts() {
    let config = default_keybindings();

    assert_has_config_binding(&config, "cmd-,", "settings.open");
    assert_has_config_binding(&config, "ctrl-,", "settings.open");
    assert_has_ui_binding("cmd-,", "settings.open");
    assert_has_ui_binding("ctrl-,", "settings.open");
}

#[test]
fn keybinding_display_filters_shortcuts_by_platform() {
    let keys = vec!["cmd-p".to_string(), "ctrl-p".to_string()];

    assert_eq!(
        display_keybindings_for_platform(&keys, KeybindingDisplayPlatform::Mac),
        vec!["cmd-p".to_string()]
    );
    assert_eq!(
        display_keybindings_for_platform(&keys, KeybindingDisplayPlatform::Other),
        vec!["ctrl-p".to_string()]
    );
}

#[test]
fn keybinding_display_keeps_neutral_or_fallback_shortcuts() {
    let neutral = vec!["enter".to_string(), "escape".to_string()];
    assert_eq!(
        display_keybindings_for_platform(&neutral, KeybindingDisplayPlatform::Mac),
        neutral
    );

    let fallback = vec!["cmd-p".to_string()];
    assert_eq!(
        display_keybindings_for_platform(&fallback, KeybindingDisplayPlatform::Other),
        fallback
    );
}

#[test]
fn default_keybindings_include_pane_navigation_shortcuts() {
    let config = default_keybindings();

    for (keys, command) in [
        ("cmd-alt-left", "pane.focus_left"),
        ("cmd-alt-right", "pane.focus_right"),
        ("cmd-alt-up", "pane.focus_up"),
        ("cmd-alt-down", "pane.focus_down"),
        ("ctrl-alt-left", "pane.focus_left"),
        ("ctrl-alt-right", "pane.focus_right"),
        ("ctrl-alt-up", "pane.focus_up"),
        ("ctrl-alt-down", "pane.focus_down"),
        ("cmd-alt-shift-left", "pane.resize_left"),
        ("cmd-alt-shift-right", "pane.resize_right"),
        ("cmd-alt-shift-up", "pane.resize_up"),
        ("cmd-alt-shift-down", "pane.resize_down"),
        ("ctrl-alt-shift-left", "pane.resize_left"),
        ("ctrl-alt-shift-right", "pane.resize_right"),
        ("ctrl-alt-shift-up", "pane.resize_up"),
        ("ctrl-alt-shift-down", "pane.resize_down"),
    ] {
        assert_has_config_binding(&config, keys, command);
        assert_has_ui_binding(keys, command);
    }
}

#[test]
fn default_keybindings_include_tab_new_shortcuts() {
    let config = default_keybindings();

    assert_has_config_binding(&config, "cmd-t", "tab.new");
    assert_has_config_binding(&config, "ctrl-t", "tab.new");
    assert_has_ui_binding("cmd-t", "tab.new");
    assert_has_ui_binding("ctrl-t", "tab.new");
}

#[test]
fn default_keybindings_include_context_close_shortcuts() {
    let config = default_keybindings();

    assert_has_config_binding(&config, "cmd-w", "pane.close");
    assert_has_config_binding(&config, "ctrl-w", "pane.close");
    assert_has_ui_binding("cmd-w", "pane.close");
    assert_has_ui_binding("ctrl-w", "pane.close");
}

#[test]
fn user_keybindings_specs_override_default_ui_bindings() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-l"
        command = "tab.palette"
    "#,
    )
    .unwrap();

    let specs = yttt::ui::interaction::actions::ui_keybinding_specs_from_config(
        &config,
        &default_registry(),
    );

    assert!(
        specs
            .iter()
            .any(|spec| spec.keys == "cmd-l" && spec.command == CommandId::TabPalette)
    );
    assert!(
        !specs
            .iter()
            .any(|spec| spec.keys == "cmd-j" && spec.command == CommandId::TabPalette)
    );
}

#[test]
fn user_keybindings_specs_skip_conflicting_keys_and_invalid_commands() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-l"
        command = "tab.palette"

        [[bindings]]
        keys = "CMD-L"
        command = "pane.palette"

        [[bindings]]
        keys = "cmd-x"
        command = "missing.command"
    "#,
    )
    .unwrap();

    let specs = yttt::ui::interaction::actions::ui_keybinding_specs_from_config(
        &config,
        &default_registry(),
    );

    assert!(specs.is_empty());
}

#[test]
fn user_keybindings_specs_map_non_default_command_actions() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-alt-k"
        command = "settings.keybindings"
    "#,
    )
    .unwrap();

    let specs = yttt::ui::interaction::actions::ui_keybinding_specs_from_config(
        &config,
        &default_registry(),
    );

    assert_eq!(specs.len(), 1);
    assert_eq!(specs[0].keys, "cmd-alt-k");
    assert_eq!(specs[0].command, CommandId::SettingsKeybindings);
}

#[test]
fn runtime_keybinding_matcher_uses_current_config_specs_only() {
    let config: KeybindingsConfig = toml::from_str(
        r#"
        [[bindings]]
        keys = "cmd-l"
        command = "tab.palette"
    "#,
    )
    .unwrap();
    let specs = yttt::ui::interaction::actions::ui_keybinding_specs_from_config(
        &config,
        &default_registry(),
    );

    assert_eq!(
        runtime_command_for_keystroke(&specs, &Keystroke::parse("cmd-l").unwrap()),
        Some(CommandId::TabPalette)
    );
    assert_eq!(
        runtime_command_for_keystroke(&specs, &Keystroke::parse("cmd-j").unwrap()),
        None
    );
}

#[test]
fn app_startup_keybindings_keep_user_editable_bindings_out_of_gpui_keymap() {
    assert_eq!(app_startup_keybindings().len(), 4);
}

#[test]
fn load_app_keybindings_missing_file_writes_defaults_without_registering_editable_keys() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let bindings =
        yttt::ui::interaction::actions::load_app_keybindings(&paths, &default_registry());

    assert!(paths.keybindings_file().exists());
    assert_eq!(bindings.len(), app_startup_keybindings().len());
}

#[test]
fn missing_keybindings_file_writes_defaults() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config, default_keybindings());
    assert!(loaded.warnings.is_empty());
    assert!(paths.keybindings_file().exists());
}

#[test]
fn legacy_default_keybindings_are_upgraded_with_editor_shortcuts() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    let mut legacy = default_keybindings();
    legacy.schema_version = 0;
    legacy.bindings.retain(|binding| {
        !matches!(
            binding.command.as_str(),
            "file.save" | "project_panel.toggle"
        )
    });
    std::fs::write(
        paths.keybindings_file(),
        toml::to_string_pretty(&legacy).unwrap(),
    )
    .unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config.schema_version, KEYBINDINGS_SCHEMA_VERSION);
    assert_has_config_binding(&loaded.config, "cmd-s", "file.save");
    assert_has_config_binding(&loaded.config, "ctrl-s", "file.save");
    let persisted: KeybindingsConfig =
        toml::from_str(&std::fs::read_to_string(paths.keybindings_file()).unwrap()).unwrap();
    assert_eq!(persisted, loaded.config);
}

#[test]
fn custom_legacy_keybindings_are_versioned_without_restoring_defaults() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let legacy = KeybindingsConfig {
        schema_version: 0,
        bindings: vec![Keybinding {
            keys: "cmd-l".to_string(),
            command: "tab.palette".to_string(),
        }],
    };
    save_keybindings(&paths, &legacy).unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config.schema_version, KEYBINDINGS_SCHEMA_VERSION);
    assert_eq!(loaded.config.bindings, legacy.bindings);
    assert!(
        loaded
            .config
            .bindings
            .iter()
            .all(|binding| binding.command != "file.save")
    );
    assert_eq!(
        load_keybindings(&paths, &default_registry())
            .unwrap()
            .config,
        loaded.config
    );
}

#[test]
fn save_keybindings_writes_user_toml() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let config = KeybindingsConfig {
        schema_version: KEYBINDINGS_SCHEMA_VERSION,
        bindings: vec![Keybinding {
            keys: "cmd-l".to_string(),
            command: "tab.palette".to_string(),
        }],
    };

    save_keybindings(&paths, &config).unwrap();

    let saved = std::fs::read_to_string(paths.keybindings_file()).unwrap();
    assert!(saved.contains("cmd-l"));
    assert!(saved.contains("tab.palette"));
}

#[test]
fn keybindings_editor_lists_commands_with_current_keys() {
    let editor = KeybindingsEditorState::new(default_keybindings(), default_registry());

    let row = editor
        .rows()
        .into_iter()
        .find(|row| row.command == CommandId::CommandPaletteOpen)
        .unwrap();

    assert_eq!(row.title, "Open Command Palette");
    assert!(row.keys.contains(&"cmd-p".to_string()));
    assert!(row.keys.contains(&"ctrl-p".to_string()));
    if cfg!(target_os = "macos") {
        assert_eq!(row.display_keys(), vec!["cmd-p".to_string()]);
    } else {
        assert_eq!(row.display_keys(), vec!["ctrl-p".to_string()]);
    }
}

#[test]
fn keybindings_editor_edits_deletes_and_resets_command_keys() {
    let mut editor = KeybindingsEditorState::new(default_keybindings(), default_registry());

    editor.set_command_keys(CommandId::TabPalette, vec!["cmd-l".to_string()]);
    assert_eq!(
        editor.command_keys(CommandId::TabPalette),
        vec!["cmd-l".to_string()]
    );

    editor.delete_command_keys(CommandId::TabPalette);
    assert!(editor.command_keys(CommandId::TabPalette).is_empty());

    editor.reset_command_keys(CommandId::TabPalette);
    assert!(
        editor
            .command_keys(CommandId::TabPalette)
            .contains(&"cmd-j".to_string())
    );
}

#[test]
fn keybindings_editor_blocks_conflicting_save() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut editor = KeybindingsEditorState::new(default_keybindings(), default_registry());
    editor.set_command_keys(CommandId::TabPalette, vec!["cmd-p".to_string()]);

    let error = editor.save(&paths).unwrap_err();

    assert_eq!(
        error,
        KeybindingEditError::ConflictingBindings(vec!["cmd-p".to_string()])
    );
    assert!(!paths.keybindings_file().exists());
}

#[test]
fn conflicting_user_bindings_are_reported() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.keybindings_file(),
        r#"
        [[bindings]]
        keys = "cmd-p"
        command = "command_palette.open"

        [[bindings]]
        keys = "CMD-P"
        command = "project.palette"
    "#,
    )
    .unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.warnings.len(), 1);
    assert!(matches!(
        &loaded.warnings[0],
        KeybindingLoadWarning::Conflict(conflict) if conflict.keys == "cmd-p"
    ));
}

#[test]
fn invalid_command_id_is_reported() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    std::fs::create_dir_all(paths.config_dir()).unwrap();
    std::fs::write(
        paths.keybindings_file(),
        r#"
        [[bindings]]
        keys = "cmd-x"
        command = "missing.command"
    "#,
    )
    .unwrap();

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(
        loaded.warnings,
        vec![KeybindingLoadWarning::InvalidCommand(
            "missing.command".to_string()
        )]
    );
}

#[test]
fn tab_next_command_selects_next_tab_and_marks_it_started() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::TabNext).unwrap();
    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();

    assert_eq!(project.selected_tab_id, "agent");
    assert_eq!(
        project.tab_state("agent").unwrap().start_state,
        TabStartState::Started
    );
}

#[test]
fn tab_new_command_adds_shell_tab_and_selects_it() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::TabNew).unwrap();
    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    let tab = project
        .layout
        .tabs
        .iter()
        .find(|tab| tab.id == project.selected_tab_id)
        .unwrap();

    assert_eq!(project.layout.tabs.len(), 3);
    assert_eq!(tab.id, "tab-1");
    assert_eq!(tab.title, "Tab 1");
    assert_eq!(tab.layout.pane_id(), Some("shell"));
    assert_eq!(
        project.tab_state("tab-1").unwrap().start_state,
        TabStartState::Started
    );
    assert_focused_pane(&workspace, "shell");
}

#[test]
fn tab_close_command_removes_selected_tab() {
    let mut workspace = workspace_with_sample_project();
    workspace.select_tab("agent").unwrap();

    dispatch_workspace_command(&mut workspace, CommandId::TabClose).unwrap();

    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "dev");
    assert!(project.layout.tab("agent").is_none());
}

#[test]
fn tab_rename_command_changes_selected_tab_title() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::TabRename).unwrap();

    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    assert_eq!(project.layout.tab("dev").unwrap().title, "Renamed Tab");
    assert!(project.layout.tab("dev").is_some());
}

#[test]
fn pane_split_vertical_command_adds_pane_to_current_tab() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::PaneSplitVertical).unwrap();

    assert_eq!(visible_pane_titles(&workspace).len(), 3);
}

#[test]
fn pane_close_command_removes_focused_pane() {
    let mut workspace = workspace_with_sample_project();
    dispatch_workspace_command(&mut workspace, CommandId::PaneSplitVertical).unwrap();

    dispatch_workspace_command(&mut workspace, CommandId::PaneClose).unwrap();

    assert_eq!(visible_pane_titles(&workspace).len(), 2);
}

#[test]
fn pane_close_command_closes_single_pane_tab_by_context() {
    let mut workspace = workspace_with_sample_project();
    workspace.select_tab("agent").unwrap();

    dispatch_workspace_command(&mut workspace, CommandId::PaneClose).unwrap();

    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "dev");
    assert!(project.layout.tab("agent").is_none());
}

#[test]
fn pane_rename_command_changes_focused_pane_title() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::PaneRename).unwrap();

    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    let pane = project
        .layout
        .tab("dev")
        .unwrap()
        .layout
        .find_pane("server")
        .unwrap();
    assert_eq!(pane.id, "server");
    assert_eq!(pane.title, "Renamed Pane");
}

#[test]
fn pane_focus_commands_move_between_adjacent_panes() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::PaneFocusRight).unwrap();
    assert_focused_pane(&workspace, "shell");

    dispatch_workspace_command(&mut workspace, CommandId::PaneFocusLeft).unwrap();
    assert_focused_pane(&workspace, "server");
}

#[test]
fn pane_resize_commands_adjust_current_split_ratio() {
    let mut workspace = workspace_with_sample_project();

    dispatch_workspace_command(&mut workspace, CommandId::PaneResizeRight).unwrap();
    assert_ratio(root_split_ratio(&workspace), 0.7);

    dispatch_workspace_command(&mut workspace, CommandId::PaneResizeLeft).unwrap();
    assert_ratio(root_split_ratio(&workspace), 0.65);
}

fn workspace_with_sample_project() -> Workspace {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace
}

fn root_split_ratio(workspace: &Workspace) -> f32 {
    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    let tab = project
        .layout
        .tabs
        .iter()
        .find(|tab| tab.id == project.selected_tab_id)
        .unwrap();
    match &tab.layout {
        LayoutNode::Split(split) => split.ratio,
        LayoutNode::Pane(_) => panic!("sample tab should be split"),
    }
}

fn assert_ratio(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.001,
        "expected ratio {expected}, got {actual}"
    );
}

fn assert_has_config_binding(config: &KeybindingsConfig, keys: &str, command: &str) {
    assert!(
        config
            .bindings
            .iter()
            .any(|binding| binding.keys == keys && binding.command == command),
        "expected default keybindings to include {keys} -> {command}"
    );
}

fn assert_has_ui_binding(keys: &str, command: &str) {
    assert!(
        default_ui_keybinding_specs()
            .iter()
            .any(|binding| binding.keys == keys && binding.command.as_str() == command),
        "expected GPUI keybindings to include {keys} -> {command}"
    );
}

fn assert_focused_pane(workspace: &Workspace, expected_pane_id: &str) {
    let project_id = workspace.selected_project_id().unwrap().clone();
    let project = workspace.project(&project_id).unwrap();
    let tab = project.tab_state(&project.selected_tab_id).unwrap();
    assert_eq!(tab.focused_pane_id.as_deref(), Some(expected_pane_id));
}

fn sample_layout() -> yttt::model::layout::ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "yttt"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"

        [tabs.layout]
        type = "split"
        direction = "horizontal"
        ratio = 0.65
        left = { type = "pane", id = "server", title = "server", command = "npm run dev" }
        right = { type = "pane", id = "shell", title = "shell", command = "$SHELL" }

        [[tabs]]
        id = "agent"
        title = "Agent"
        layout = { type = "pane", id = "codex", title = "Codex", command = "codex", kind = "agent", notify_on_exit = true }
    "#,
    )
    .unwrap()
}
