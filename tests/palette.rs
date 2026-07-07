use std::path::PathBuf;

use yttt::{
    commands::{default_registry, CommandId},
    model::workspace::Workspace,
    palette::{
        command_palette_items, pane_palette_items, project_palette_items, tab_palette_items,
        RecentProject,
    },
};

#[test]
fn command_palette_contains_all_registered_commands() {
    let registry = default_registry();

    let items = command_palette_items(&registry);

    assert_eq!(items.len(), registry.commands().len());
    assert!(items
        .iter()
        .any(|item| item.id == "command_palette.open" && item.command == CommandId::CommandPaletteOpen));
}

#[test]
fn project_palette_contains_opened_and_recent_projects() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    let recent = vec![RecentProject {
        title: "zed".to_string(),
        path: PathBuf::from("/tmp/zed"),
    }];

    let items = project_palette_items(&workspace, &recent);

    assert!(items
        .iter()
        .any(|item| item.title == "yttt" && item.status.as_deref() == Some("open")));
    assert!(items
        .iter()
        .any(|item| item.title == "zed" && item.status.as_deref() == Some("recent")));
}

#[test]
fn tab_palette_contains_current_project_tabs() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();

    let items = tab_palette_items(&workspace).unwrap();

    assert!(items
        .iter()
        .any(|item| item.title == "Dev" && item.status.as_deref() == Some("started")));
    assert!(items
        .iter()
        .any(|item| item.title == "Agent" && item.status.as_deref() == Some("lazy")));
}

#[test]
fn pane_palette_contains_current_tab_panes() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();

    let items = pane_palette_items(&workspace).unwrap();

    assert!(items
        .iter()
        .any(|item| item.title == "server" && item.status.as_deref() == Some("idle")));
    assert!(items
        .iter()
        .any(|item| item.title == "shell" && item.status.as_deref() == Some("idle")));
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
