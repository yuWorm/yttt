use std::path::PathBuf;

use tempfile::tempdir;
use yttt::commands::{CommandId, default_registry, dispatch_workspace_command};
use yttt::config::{
    keybindings::{
        KeybindingLoadWarning, KeybindingsConfig, default_keybindings, load_keybindings,
    },
    paths::AppConfigPaths,
};
use yttt::model::layout::LayoutNode;
use yttt::model::workspace::{TabStartState, Workspace};
use yttt::ui::actions::default_ui_keybinding_specs;
use yttt::ui::split_view::visible_pane_titles;

#[test]
fn default_registry_contains_core_commands() {
    let registry = default_registry();

    assert!(registry.contains(CommandId::ProjectOpen));
    assert!(registry.contains(CommandId::PaneSplitVertical));
    assert!(registry.contains(CommandId::TabRename));
    assert!(registry.contains(CommandId::CommandPaletteOpen));
}

#[test]
fn notification_settings_command_is_available_without_project() {
    let availability = CommandId::SettingsNotifications.availability(false);

    assert!(availability.enabled);
    assert!(availability.disabled_reason.is_none());
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
fn missing_keybindings_file_writes_defaults() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let loaded = load_keybindings(&paths, &default_registry()).unwrap();

    assert_eq!(loaded.config, default_keybindings());
    assert!(loaded.warnings.is_empty());
    assert!(paths.keybindings_file().exists());
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
