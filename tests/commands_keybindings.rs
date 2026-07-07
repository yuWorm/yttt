use std::path::PathBuf;

use yttt::commands::{CommandId, default_registry, dispatch_workspace_command};
use yttt::config::keybindings::{KeybindingsConfig, default_keybindings};
use yttt::model::workspace::{TabStartState, Workspace};
use yttt::ui::split_view::visible_pane_titles;

#[test]
fn default_registry_contains_core_commands() {
    let registry = default_registry();

    assert!(registry.contains(CommandId::ProjectOpen));
    assert!(registry.contains(CommandId::PaneSplitVertical));
    assert!(registry.contains(CommandId::CommandPaletteOpen));
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

    assert!(
        config
            .bindings
            .iter()
            .any(|binding| binding.keys == "cmd-p" && binding.command == "command_palette.open")
    );
    assert!(
        config
            .bindings
            .iter()
            .any(|binding| binding.keys == "ctrl-k" && binding.command == "pane.palette")
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

fn workspace_with_sample_project() -> Workspace {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace
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
