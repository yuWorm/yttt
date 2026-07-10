use std::path::PathBuf;

use yttt::{
    commands::{ActiveSurface, CommandContext, CommandId, default_registry},
    model::{
        ids::ProjectId,
        workspace::{AgentStatus, Workspace},
    },
    palette::{
        ActivePalette, CommandPaletteContext, PaletteItem, PaletteKind, RecentProject,
        TabPaletteSnapshot, command_palette_items, pane_palette_items, project_palette_items,
        tab_palette_items, unified_tab_palette_items,
    },
    ui::{
        components::SelectableState,
        editor::DocumentId,
        palette::visible_palette_rows,
        picker::{PalettePickerDelegate, PickerDelegate, PickerItem, PickerState},
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
fn command_palette_uses_active_file_surface_availability() {
    let registry = default_registry();
    let context = CommandPaletteContext::from_command_context(CommandContext {
        has_selected_project: true,
        active_surface: ActiveSurface::File,
    });

    let items = command_palette_items(&registry, context);

    for command in [
        CommandId::FileSave,
        CommandId::TabClose,
        CommandId::ProjectPanelToggle,
        CommandId::ProjectPanelRefresh,
    ] {
        assert!(
            items
                .iter()
                .find(|item| item.command == command)
                .unwrap()
                .enabled,
            "{command:?} should be enabled for an active file"
        );
    }
    for command in [
        CommandId::TabNew,
        CommandId::TabRename,
        CommandId::PaneSplitVertical,
        CommandId::PaneFocusLeft,
    ] {
        assert!(
            !items
                .iter()
                .find(|item| item.command == command)
                .unwrap()
                .enabled,
            "{command:?} should be disabled for an active file"
        );
    }
    assert_eq!(
        items
            .iter()
            .find(|item| item.command == CommandId::TabNew)
            .unwrap()
            .disabled_reason
            .as_deref(),
        Some("Switch to a terminal tab first")
    );
}

#[test]
fn layout_default_palette_commands_are_enabled_without_project() {
    let registry = default_registry();
    let workspace = Workspace::new();
    let items = command_palette_items(&registry, CommandPaletteContext::from_workspace(&workspace));

    for command in [
        CommandId::LayoutDefaultEdit,
        CommandId::LayoutDefaultReset,
        CommandId::LayoutDefaultReload,
    ] {
        let item = items.iter().find(|item| item.command == command).unwrap();
        assert!(item.enabled);
        assert!(item.disabled_reason.is_none());
    }
}

#[test]
fn layout_project_palette_commands_are_disabled_without_project() {
    let registry = default_registry();
    let workspace = Workspace::new();
    let items = command_palette_items(&registry, CommandPaletteContext::from_workspace(&workspace));

    for command in [
        CommandId::LayoutProjectEdit,
        CommandId::LayoutResetLocalOverride,
        CommandId::LayoutOpenFile,
    ] {
        let item = items.iter().find(|item| item.command == command).unwrap();
        assert!(!item.enabled);
        assert_eq!(
            item.disabled_reason.as_deref(),
            Some("Open a project first")
        );
    }
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
fn unified_tab_palette_prefixes_ids_and_shows_file_relative_paths() {
    let document_id = DocumentId {
        project_id: ProjectId::new("project-a"),
        canonical_path: PathBuf::from("/tmp/yttt/src/main.rs"),
    };
    let snapshots = vec![
        TabPaletteSnapshot::terminal(
            "dev",
            "Dev",
            Some("2 panes".to_string()),
            Some("active · started".to_string()),
        ),
        TabPaletteSnapshot::file(
            document_id,
            PathBuf::from("src/main.rs"),
            Some("unsaved".to_string()),
        ),
    ];

    let items = unified_tab_palette_items(&snapshots);

    assert_eq!(items[0].id, "terminal:dev");
    assert_eq!(items[0].title, "Dev");
    assert_eq!(items[0].subtitle.as_deref(), Some("2 panes"));
    assert_eq!(items[1].id, "file:/tmp/yttt/src/main.rs");
    assert_eq!(items[1].title, "main.rs");
    assert_eq!(items[1].subtitle.as_deref(), Some("src/main.rs"));
    assert_eq!(items[1].status.as_deref(), Some("unsaved"));

    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    let legacy_items = tab_palette_items(&workspace).unwrap();
    assert!(legacy_items.iter().any(|item| item.id == "dev"));
    assert!(
        legacy_items
            .iter()
            .all(|item| !item.id.starts_with("terminal:"))
    );
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

#[test]
fn picker_state_filters_items_case_insensitively() {
    let mut state = PickerState::new();
    state.set_query("open");
    let items = vec![
        PickerItem::new("open", "Open Project"),
        PickerItem::new("close", "Close Project"),
    ];

    let rows = state.filtered_items(&items);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "open");
}

#[test]
fn picker_state_filters_items_by_keybinding() {
    let mut state = PickerState::new();
    state.set_query("cmd-p");
    let mut open = PickerItem::new("command_palette.open", "Open Command Palette");
    open.keybinding = Some("cmd-p".to_string());
    let items = vec![PickerItem::new("project.open", "Open Project"), open];

    let rows = state.filtered_items(&items);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].id, "command_palette.open");
}

#[test]
fn picker_state_clamps_selected_index_to_filtered_rows() {
    let mut state = PickerState::new();
    state.selected_index = 99;
    state.set_query("project");
    state.selected_index = 99;
    let items = vec![
        PickerItem::new("open", "Open Project"),
        PickerItem::new("close", "Close Project"),
    ];

    assert_eq!(state.clamped_selected_index(&items), Some(1));
}

#[test]
fn picker_item_preserves_disabled_reason_from_palette_item() {
    let item = PaletteItem {
        id: "tab.new".to_string(),
        title: "New Tab".to_string(),
        subtitle: Some("Create tab".to_string()),
        status: None,
        keybinding: Some("cmd-t".to_string()),
        command: CommandId::TabNew,
        enabled: false,
        disabled_reason: Some("Open a project first".to_string()),
    };

    let picker = PickerItem::from_palette_item(&item);

    assert_eq!(
        picker.disabled_reason.as_deref(),
        Some("Open a project first")
    );
    assert_eq!(picker.keybinding.as_deref(), Some("cmd-t"));
    assert!(!picker.enabled);
}

#[test]
fn visible_palette_rows_preserve_active_selection_after_picker_migration() {
    let mut active = ActivePalette::new(PaletteKind::Command);
    active.query = "project".to_string();
    active.selected_index = 1;
    let items = vec![
        PaletteItem {
            id: "project.open".to_string(),
            title: "Open Project".to_string(),
            subtitle: Some("Choose a project directory".to_string()),
            status: None,
            keybinding: Some("cmd-o".to_string()),
            command: CommandId::ProjectOpen,
            enabled: true,
            disabled_reason: None,
        },
        PaletteItem {
            id: "project.close".to_string(),
            title: "Close Project".to_string(),
            subtitle: Some("Close the selected project".to_string()),
            status: None,
            keybinding: None,
            command: CommandId::ProjectClose,
            enabled: true,
            disabled_reason: None,
        },
    ];

    let rows = visible_palette_rows(&active, &items);

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[1].state, SelectableState::Active);
    assert_eq!(rows[0].keybinding.as_deref(), Some("cmd-o"));
}

#[test]
fn palette_picker_delegate_exposes_picker_items_for_all_palette_kinds() {
    for kind in [
        PaletteKind::Command,
        PaletteKind::Project,
        PaletteKind::Tab,
        PaletteKind::Pane,
    ] {
        let delegate = PalettePickerDelegate::new(
            kind,
            vec![PaletteItem {
                id: "item".to_string(),
                title: "Item".to_string(),
                subtitle: None,
                status: None,
                keybinding: Some("cmd-p".to_string()),
                command: CommandId::CommandPaletteOpen,
                enabled: true,
                disabled_reason: None,
            }],
        );

        assert_eq!(delegate.kind(), kind);
        assert_eq!(delegate.items()[0].id, "item");
        assert_eq!(delegate.items()[0].keybinding.as_deref(), Some("cmd-p"));
    }
}

fn sample_palette_items() -> Vec<PaletteItem> {
    vec![
        PaletteItem {
            id: "server".to_string(),
            title: "Server".to_string(),
            subtitle: Some("Dev".to_string()),
            status: Some("running".to_string()),
            keybinding: None,
            command: CommandId::PanePalette,
            enabled: true,
            disabled_reason: None,
        },
        PaletteItem {
            id: "shell".to_string(),
            title: "Shell".to_string(),
            subtitle: Some("Dev".to_string()),
            status: Some("idle".to_string()),
            keybinding: None,
            command: CommandId::PanePalette,
            enabled: true,
            disabled_reason: None,
        },
        PaletteItem {
            id: "codex".to_string(),
            title: "Codex Agent".to_string(),
            subtitle: Some("Agent".to_string()),
            status: Some("lazy".to_string()),
            keybinding: None,
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
