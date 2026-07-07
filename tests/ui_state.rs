use std::{fs, path::PathBuf};

use tempfile::tempdir;
use yttt::{
    commands::CommandId,
    config::paths::AppConfigPaths,
    model::{layout::PaneKind, workspace::Workspace},
    palette::PaletteKind,
    runtime::notification::{NotificationEvent, NotificationKind},
    runtime::terminal::{ExitReason, ProcessStatus},
    ui::terminal_pane::{TerminalPaneExitInput, notification_for_terminal_pane_exit},
    ui::{root::RootView, split_view::visible_pane_titles, tabs::visible_tab_titles},
};

#[test]
fn root_view_starts_with_empty_workspace() {
    let root = RootView::new();

    assert!(root.workspace().opened_projects().is_empty());
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
fn visible_pane_titles_come_from_selected_tab() {
    let workspace = workspace_with_sample_project();

    assert_eq!(visible_pane_titles(&workspace), vec!["server", "shell"]);
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
    NotificationEvent {
        kind: NotificationKind::AgentCompleted,
        project_title: "yttt".to_string(),
        tab_title: "Agent".to_string(),
        pane_title: "Codex".to_string(),
    }
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
