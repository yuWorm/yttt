use std::path::PathBuf;

use yttt::{
    commands::{CommandId, default_registry},
    model::workspace::{AgentStatus, Workspace},
    palette::{
        ActivePalette, CommandPaletteContext, PaletteItem, PaletteKind, RecentProject,
        command_palette_items, pane_palette_items, project_palette_items, tab_palette_items,
    },
};

#[test]
fn command_palette_contains_all_registered_commands() {
    let registry = default_registry();
    let workspace = Workspace::new();

    let items = command_palette_items(&registry, CommandPaletteContext::from_workspace(&workspace));

    assert_eq!(items.len(), registry.commands().len());
    assert!(
        items.iter().any(|item| item.id == "command_palette.open"
            && item.command == CommandId::CommandPaletteOpen)
    );
}

#[test]
fn command_palette_uses_readable_titles_and_descriptions() {
    let registry = default_registry();
    let workspace = Workspace::new();

    let items = command_palette_items(&registry, CommandPaletteContext::from_workspace(&workspace));
    let command_palette = items
        .iter()
        .find(|item| item.command == CommandId::CommandPaletteOpen)
        .unwrap();

    assert_eq!(command_palette.title, "Open Command Palette");
    assert_eq!(
        command_palette.subtitle.as_deref(),
        Some("Search and run commands")
    );
}

#[test]
fn command_palette_disables_workspace_commands_without_project() {
    let registry = default_registry();
    let workspace = Workspace::new();

    let items = command_palette_items(&registry, CommandPaletteContext::from_workspace(&workspace));
    let split = items
        .iter()
        .find(|item| item.command == CommandId::PaneSplitVertical)
        .unwrap();

    assert!(!split.enabled);
    assert_eq!(
        split.disabled_reason.as_deref(),
        Some("Open a project first")
    );
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

    assert!(
        items
            .iter()
            .any(|item| item.title == "yttt" && item.status.as_deref() == Some("open"))
    );
    assert!(
        items
            .iter()
            .any(|item| item.title == "zed" && item.status.as_deref() == Some("recent"))
    );
}

#[test]
fn project_palette_shows_open_project_agent_status() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace
        .record_agent_status(&project_id, "agent", "codex", AgentStatus::Failed)
        .unwrap();

    let items = project_palette_items(&workspace, &[]);

    assert!(items.iter().any(|item| {
        item.title == "yttt" && item.status.as_deref() == Some("open · agent failed")
    }));
}

#[test]
fn tab_palette_contains_current_project_tabs() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();

    let items = tab_palette_items(&workspace).unwrap();

    assert!(items.iter().any(|item| item.title == "Dev"
        && item.subtitle.as_deref() == Some("2 panes")
        && item.status.as_deref() == Some("active · started")));
    assert!(items.iter().any(|item| item.title == "Agent"
        && item.subtitle.as_deref() == Some("1 pane")
        && item.status.as_deref() == Some("lazy")));
}

#[test]
fn tab_palette_shows_agent_status() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace.select_tab("agent").unwrap();
    workspace
        .record_agent_status(&project_id, "agent", "codex", AgentStatus::Completed)
        .unwrap();

    let items = tab_palette_items(&workspace).unwrap();

    assert!(items.iter().any(|item| item.title == "Agent"
        && item.status.as_deref() == Some("active · started · agent completed")));
}

#[test]
fn pane_palette_contains_current_tab_panes() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();

    let items = pane_palette_items(&workspace).unwrap();

    assert!(
        items
            .iter()
            .any(|item| item.title == "server" && item.status.as_deref() == Some("active · idle"))
    );
    assert!(
        items
            .iter()
            .any(|item| item.title == "shell" && item.status.as_deref() == Some("idle"))
    );
}

#[test]
fn pane_palette_marks_agent_panes() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace.select_tab("agent").unwrap();

    let items = pane_palette_items(&workspace).unwrap();

    assert!(items.iter().any(
        |item| item.title == "Codex" && item.status.as_deref() == Some("active · idle · agent")
    ));
}

#[test]
fn pane_palette_shows_agent_exit_result() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace.select_tab("agent").unwrap();
    workspace
        .record_agent_status(&project_id, "agent", "codex", AgentStatus::Failed)
        .unwrap();

    let items = pane_palette_items(&workspace).unwrap();

    assert!(items.iter().any(|item| {
        item.title == "Codex" && item.status.as_deref() == Some("active · exited · agent failed")
    }));
}

#[test]
fn active_palette_filters_items_case_insensitively() {
    let palette = ActivePalette {
        kind: PaletteKind::Command,
        query: "agent".to_string(),
        selected_index: 0,
    };
    let items = sample_palette_items();

    let titles: Vec<_> = palette
        .filtered_items(&items)
        .into_iter()
        .map(|item| item.title.as_str())
        .collect();

    assert_eq!(titles, vec!["Codex Agent"]);
}

#[test]
fn active_palette_moves_selection_within_filtered_items() {
    let mut palette = ActivePalette::new(PaletteKind::Pane);
    let items = sample_palette_items();

    palette.select_next(&items);
    assert_eq!(palette.selected_item(&items).unwrap().id, "shell");

    palette.select_prev(&items);
    assert_eq!(palette.selected_item(&items).unwrap().id, "server");
}

fn sample_palette_items() -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "server".to_string(),
            title: "Server".to_string(),
            subtitle: Some("Dev".to_string()),
            status: Some("running".to_string()),
            command: CommandId::PanePalette,
            enabled: true,
            disabled_reason: None,
        },
        PaletteItem {
            id: "shell".to_string(),
            title: "Shell".to_string(),
            subtitle: Some("Dev".to_string()),
            status: Some("idle".to_string()),
            command: CommandId::PanePalette,
            enabled: true,
            disabled_reason: None,
        },
        PaletteItem {
            id: "codex".to_string(),
            title: "Codex Agent".to_string(),
            subtitle: Some("Agent".to_string()),
            status: Some("lazy".to_string()),
            command: CommandId::TabPalette,
            enabled: true,
            disabled_reason: None,
        },
    ]
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
