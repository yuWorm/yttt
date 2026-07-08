use std::{fs, path::PathBuf};

use tempfile::tempdir;
use yttt::{
    commands::CommandId,
    config::paths::AppConfigPaths,
    model::{layout::PaneKind, workspace::Workspace},
    palette::{ActivePalette, PaletteItem, PaletteKind},
    runtime::notification::{NotificationEvent, NotificationKind},
    runtime::terminal::{ExitReason, ProcessStatus},
    ui::components::SelectableState,
    ui::palette::visible_palette_rows,
    ui::sidebar::visible_project_items,
    ui::terminal_pane::{
        PaneLifecycle, TerminalPaneExitInput, TerminalSpawnFailure,
        notification_for_terminal_pane_exit, pane_lifecycle_label, spawn_failure_lines,
    },
    ui::toast::{ToastTone, visible_toast_items},
    ui::{
        root::RootView,
        split_view::{root_split_child_basis, visible_pane_titles},
        tabs::{visible_tab_items, visible_tab_titles},
    },
};

#[test]
fn root_view_starts_with_empty_workspace() {
    let root = RootView::new();

    assert!(root.workspace().opened_projects().is_empty());
}

#[test]
fn root_view_empty_workspace_exposes_visible_actions() {
    let root = RootView::new();

    assert_eq!(
        root.visible_empty_workspace_actions(),
        vec!["Open Directory", "Open Recent", "Command Palette"]
    );
}

#[test]
fn root_view_dev_fixture_contains_sample_project() {
    let root = RootView::dev_fixture();

    assert_eq!(root.workspace().opened_projects().len(), 1);
}

#[test]
fn root_view_agent_exit_fixture_contains_sample_project() {
    let root = RootView::agent_exit_fixture();

    assert_eq!(root.workspace().opened_projects().len(), 1);
}

#[test]
fn root_view_open_project_path_records_visible_load_error() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("broken-project");
    let project_config_dir = project_dir.join(".yttt");
    fs::create_dir_all(&project_config_dir).unwrap();
    fs::write(project_config_dir.join("layout.toml"), "[project\n").unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    let err = root.open_project_path(&project_dir).unwrap_err();

    assert!(err.to_string().contains("failed to parse project layout"));
    assert!(
        root.visible_error_message()
            .unwrap()
            .contains("failed to parse project layout")
    );
}

#[test]
fn root_view_layout_commands_write_current_project_files() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("layout-command-project");
    let project_config_dir = project_dir.join(".yttt");
    fs::create_dir_all(&project_config_dir).unwrap();
    fs::write(
        project_config_dir.join("layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());
    root.open_project_path(&project_dir).unwrap();

    root.run_command(CommandId::LayoutSaveCurrent).unwrap();
    root.run_command(CommandId::LayoutExportProjectConfig)
        .unwrap();

    assert!(paths.local_layout_file(&project_dir).exists());
    assert!(project_config_dir.join("layout.toml").exists());
}

#[test]
fn root_view_project_close_command_requires_confirmation_for_running_project() {
    let mut root = RootView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    root.workspace_mut()
        .mark_pane_running(&project_id, "dev", "server")
        .unwrap();

    root.run_command(CommandId::ProjectClose).unwrap();

    assert!(root.has_pending_project_close());
    assert_eq!(
        root.visible_close_project_dialog_text(),
        Some("Close project?\nRunning terminal processes will be stopped.")
    );
    assert_eq!(
        root.visible_close_project_dialog_actions(),
        vec!["Cancel", "Close Project"]
    );

    root.confirm_pending_project_close().unwrap();

    assert!(root.workspace().opened_projects().is_empty());
    assert!(!root.has_pending_project_close());
}

#[test]
fn root_view_settings_keybindings_reveals_keybindings_file_path() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    root.run_command(CommandId::SettingsKeybindings).unwrap();

    assert_eq!(
        root.last_opened_keybindings_file(),
        Some(paths.keybindings_file().as_path())
    );
    assert!(
        root.visible_error_message()
            .unwrap()
            .contains("keybindings.toml")
    );
    assert!(paths.keybindings_file().exists());
}

#[test]
fn visible_tab_titles_come_from_selected_project() {
    let workspace = workspace_with_sample_project();

    assert_eq!(visible_tab_titles(&workspace), vec!["Dev", "Agent"]);
}

#[test]
fn visible_project_items_mark_selected_project() {
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/one"), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(PathBuf::from("/tmp/two"), sample_layout())
        .unwrap();

    workspace.select_project(&first).unwrap();

    let items = visible_project_items(&workspace);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, first.as_str());
    assert_eq!(items[0].state, SelectableState::Active);
    assert_eq!(items[1].id, second.as_str());
    assert_eq!(items[1].state, SelectableState::Inactive);
}

#[test]
fn visible_tab_items_mark_selected_tab() {
    let mut workspace = workspace_with_sample_project();
    workspace.select_tab("agent").unwrap();

    let items = visible_tab_items(&workspace);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, "dev");
    assert_eq!(items[0].state, SelectableState::Inactive);
    assert_eq!(items[1].id, "agent");
    assert_eq!(items[1].state, SelectableState::Active);
}

#[test]
fn visible_pane_titles_come_from_selected_tab() {
    let workspace = workspace_with_sample_project();

    assert_eq!(visible_pane_titles(&workspace), vec!["server", "shell"]);
}

#[test]
fn visible_split_basis_comes_from_layout_ratio() {
    let workspace = workspace_with_sample_project();

    let basis = root_split_child_basis(&workspace).unwrap();

    assert_ratio(basis.left, 0.65);
    assert_ratio(basis.right, 0.35);
}

#[test]
fn root_view_terminal_pane_contexts_include_project_path() {
    let root = RootView::dev_fixture();

    let contexts = root.visible_terminal_pane_contexts();

    assert_eq!(contexts.len(), 2);
    assert!(
        contexts
            .iter()
            .all(|context| context.project_path == PathBuf::from("/tmp/yttt"))
    );
}

#[test]
fn root_view_tab_palette_scopes_to_current_project_tabs() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Tab);

    assert_eq!(root.visible_palette_titles(), vec!["Dev", "Agent"]);
}

#[test]
fn root_view_pane_palette_scopes_to_current_tab_panes() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Pane);

    assert_eq!(root.visible_palette_titles(), vec!["server", "shell"]);
}

#[test]
fn visible_palette_rows_mark_selected_filtered_item() {
    let mut palette = ActivePalette::new(PaletteKind::Command);
    palette.query = "tab".to_string();
    palette.selected_index = 1;
    let items = vec![
        palette_item("project.open", "Open Project"),
        palette_item("tab.next", "Next Tab"),
        palette_item("tab.prev", "Previous Tab"),
    ];

    let rows = visible_palette_rows(&palette, &items);

    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].id, "tab.next");
    assert_eq!(rows[0].state, SelectableState::Inactive);
    assert_eq!(rows[1].id, "tab.prev");
    assert_eq!(rows[1].state, SelectableState::Active);
}

#[test]
fn root_view_confirming_tab_palette_selection_switches_tabs() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Tab);
    root.set_palette_query("agent");
    root.confirm_palette_selection().unwrap();

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "agent");
}

#[test]
fn root_view_confirming_pane_palette_selection_focuses_pane() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Pane);
    root.set_palette_query("shell");
    root.confirm_palette_selection().unwrap();

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    let tab = project.tab_state("dev").unwrap();
    assert_eq!(tab.focused_pane_id.as_deref(), Some("shell"));
}

#[test]
fn root_view_focus_visible_terminal_pane_updates_focused_pane() {
    let mut root = RootView::dev_fixture();

    root.focus_visible_terminal_pane("shell").unwrap();

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    let tab = project.tab_state("dev").unwrap();
    assert_eq!(tab.focused_pane_id.as_deref(), Some("shell"));
}

#[test]
fn root_view_enqueues_agent_toast_notifications() {
    let mut root = RootView::new();

    root.handle_terminal_notification(notification_event());

    assert_eq!(root.visible_toast_titles(), vec!["Codex completed"]);
}

#[test]
fn root_view_formats_failed_agent_toast_notifications() {
    let mut root = RootView::new();
    let mut event = notification_event();
    event.kind = NotificationKind::AgentFailed;

    root.handle_terminal_notification(event);

    assert_eq!(root.visible_toast_titles(), vec!["Codex failed"]);
}

#[test]
fn visible_toast_items_show_three_recent_events_with_tone() {
    let mut root = RootView::new();
    root.handle_terminal_notification(notification_event_for(
        "first",
        NotificationKind::AgentCompleted,
    ));
    root.handle_terminal_notification(notification_event_for(
        "second",
        NotificationKind::AgentFailed,
    ));
    root.handle_terminal_notification(notification_event_for(
        "third",
        NotificationKind::AgentCompleted,
    ));
    root.handle_terminal_notification(notification_event_for(
        "fourth",
        NotificationKind::AgentFailed,
    ));

    let items = visible_toast_items(root.toast_queue());

    assert_eq!(items.len(), 3);
    assert_eq!(items[0].title, "fourth failed");
    assert_eq!(items[0].tone, ToastTone::Error);
    assert_eq!(items[1].title, "third completed");
    assert_eq!(items[1].tone, ToastTone::Success);
    assert_eq!(items[2].title, "second failed");
}

#[test]
fn terminal_pane_agent_exit_builds_notification_event() {
    let event = notification_for_terminal_pane_exit(terminal_pane_exit_input(
        ProcessStatus::Exited { code: Some(0) },
        ExitReason::Completed,
    ))
    .unwrap();

    assert_eq!(event.kind, NotificationKind::AgentCompleted);
    assert_eq!(event.project_title, "yttt");
    assert_eq!(event.tab_title, "Agent");
    assert_eq!(event.pane_title, "Codex");
}

#[test]
fn terminal_pane_lifecycle_labels_are_visible() {
    assert_eq!(pane_lifecycle_label(&PaneLifecycle::Running), "running");
    assert_eq!(
        pane_lifecycle_label(&PaneLifecycle::Exited {
            code: Some(0),
            reason: ExitReason::Completed,
        }),
        "exited 0"
    );
    assert_eq!(
        pane_lifecycle_label(&PaneLifecycle::SpawnFailed {
            message: "no such command".to_string(),
        }),
        "spawn failed"
    );
    assert_eq!(
        pane_lifecycle_label(&PaneLifecycle::Exited {
            code: None,
            reason: ExitReason::KilledByUser,
        }),
        "killed"
    );
}

#[test]
fn terminal_spawn_failure_summary_includes_command_and_cwd() {
    let lines = spawn_failure_lines(&TerminalSpawnFailure {
        command: "missing-command".to_string(),
        cwd: PathBuf::from("/tmp/yttt"),
        message: "not found".to_string(),
    });

    assert_eq!(lines[0], "Failed to start terminal");
    assert_eq!(lines[1], "command: missing-command");
    assert_eq!(lines[2], "cwd: /tmp/yttt");
    assert_eq!(lines[3], "error: not found");
}

#[test]
fn terminal_pane_user_kill_emits_no_notification_event() {
    let event = notification_for_terminal_pane_exit(terminal_pane_exit_input(
        ProcessStatus::Exited { code: None },
        ExitReason::KilledByUser,
    ));

    assert!(event.is_none());
}

fn terminal_pane_exit_input(
    status: ProcessStatus,
    exit_reason: ExitReason,
) -> TerminalPaneExitInput {
    TerminalPaneExitInput {
        project_title: "yttt".to_string(),
        tab_title: "Agent".to_string(),
        pane_title: "Codex".to_string(),
        command: "codex".to_string(),
        kind: PaneKind::Agent,
        notify_on_exit: true,
        status,
        exit_reason,
    }
}

fn notification_event() -> NotificationEvent {
    notification_event_for("Codex", NotificationKind::AgentCompleted)
}

fn notification_event_for(pane_title: &str, kind: NotificationKind) -> NotificationEvent {
    NotificationEvent {
        kind,
        project_title: "yttt".to_string(),
        tab_title: "Agent".to_string(),
        pane_title: pane_title.to_string(),
    }
}

fn palette_item(id: &str, title: &str) -> PaletteItem {
    PaletteItem {
        id: id.to_string(),
        title: title.to_string(),
        subtitle: None,
        status: None,
        command: CommandId::CommandPaletteOpen,
        enabled: true,
        disabled_reason: None,
    }
}

fn assert_ratio(actual: f32, expected: f32) {
    assert!(
        (actual - expected).abs() < 0.001,
        "expected ratio {expected}, got {actual}"
    );
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
