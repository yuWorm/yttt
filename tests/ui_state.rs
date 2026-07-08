use std::{fs, path::PathBuf};

use tempfile::tempdir;
use yttt::{
    commands::CommandId,
    config::paths::AppConfigPaths,
    model::{
        layout::PaneKind,
        layout::SplitDirection,
        split_tree::ResizeDirection,
        workspace::{AgentStatus, Workspace},
    },
    palette::{ActivePalette, PaletteItem, PaletteKind},
    runtime::git_status::{GitStatusSummary, parse_git_status_porcelain},
    runtime::notification::{NotificationEvent, NotificationKind},
    runtime::terminal::{ExitReason, ProcessStatus},
    ui::components::SelectableState,
    ui::i18n::{Locale, UiText},
    ui::palette::visible_palette_rows,
    ui::sidebar::visible_project_items,
    ui::terminal_pane::{
        PaneLifecycle, TerminalPaneExitInput, TerminalPaneExitedEvent, TerminalSpawnFailure,
        notification_for_terminal_pane_exit, pane_lifecycle_label, spawn_failure_lines,
    },
    ui::toast::{ToastTone, toast_item_for_event, visible_toast_items},
    ui::{
        root::RootView,
        split_view::{
            pointer_resize_for_drag_delta, resize_command_for_drag_delta, root_split_child_basis,
            visible_pane_titles,
        },
        tabs::{visible_tab_items, visible_tab_titles},
    },
};

#[test]
fn git_status_summary_parses_branch_and_dirty_counts() {
    let parsed = parse_git_status_porcelain(
        "## main...origin/main\n M src/main.rs\nA  src/lib.rs\n D old.rs\n?? new.rs\n",
    );

    assert_eq!(parsed.branch.as_deref(), Some("main"));
    assert_eq!(parsed.summary.added, 2);
    assert_eq!(parsed.summary.modified, 1);
    assert_eq!(parsed.summary.deleted, 1);
}

#[test]
fn git_status_summary_formats_compact_counters() {
    let summary = GitStatusSummary {
        added: 2,
        modified: 4,
        deleted: 1,
        untracked: 0,
    };

    assert_eq!(summary.compact_counters(), Some("+2 ~4 -1".to_string()));
}

#[test]
fn git_status_summary_counts_each_dirty_file_once() {
    let parsed = parse_git_status_porcelain("## main\nMM src/main.rs\nAM src/new.rs\n");

    assert_eq!(parsed.summary.added, 1);
    assert_eq!(parsed.summary.modified, 1);
}

#[test]
fn root_view_titlebar_info_describes_empty_workspace() {
    let root = RootView::new();

    let info = root.visible_titlebar_info();

    assert_eq!(info.project_name, "yttt");
    assert!(info.compact_path.is_none());
    assert!(info.git_branch.is_none());
}

#[test]
fn root_view_titlebar_info_describes_selected_project() {
    let root = RootView::dev_fixture();

    let info = root.visible_titlebar_info();

    assert_eq!(info.project_name, "yttt");
    assert_eq!(info.compact_path.as_deref(), Some("/tmp/yttt"));
}

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
fn root_view_exposes_created_app_local_layout_source() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("source-message-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    root.open_project_path(&project_dir).unwrap();

    assert_eq!(
        root.visible_layout_source_message(),
        Some("Layout source: created app-local default")
    );
}

#[test]
fn root_view_layout_open_file_falls_back_to_app_local_layout() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("local-layout-open-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let expected_layout_file = paths.local_layout_file(&project_dir.canonicalize().unwrap());
    let mut root = RootView::with_config_paths(paths);
    root.open_project_path(&project_dir).unwrap();

    root.run_command(CommandId::LayoutOpenFile).unwrap();

    assert_eq!(
        root.last_opened_layout_file(),
        Some(expected_layout_file.as_path())
    );
    assert_eq!(root.visible_error_message(), None);
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
        root.visible_close_project_dialog_text().as_deref(),
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
fn root_view_toggles_system_notifications() {
    let mut root = RootView::new();

    assert!(!root.system_notifications_enabled());
    assert_eq!(
        root.visible_notification_settings_message(),
        "System notifications: disabled"
    );

    root.run_command(CommandId::SettingsNotifications).unwrap();

    assert!(root.system_notifications_enabled());
    assert_eq!(
        root.visible_notification_settings_message(),
        "System notifications: enabled"
    );

    root.run_command(CommandId::SettingsNotifications).unwrap();

    assert!(!root.system_notifications_enabled());
    assert_eq!(
        root.visible_notification_settings_message(),
        "System notifications: disabled"
    );
}

#[test]
fn root_view_exposes_keybinding_warning_lines() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.keybindings_file(),
        r#"
        [[bindings]]
        keys = "cmd-p"
        command = "command_palette.open"

        [[bindings]]
        keys = "CMD-P"
        command = "project.palette"

        [[bindings]]
        keys = "cmd-x"
        command = "missing.command"
    "#,
    )
    .unwrap();

    let root = RootView::with_config_paths(paths);

    assert_eq!(
        root.visible_keybinding_warning_lines(),
        vec![
            "Conflicting keybinding: cmd-p",
            "Invalid command id: missing.command"
        ]
    );
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
fn visible_project_items_show_agent_running_status() {
    let mut workspace = workspace_with_sample_project();
    let project_id = workspace.selected_project_id().unwrap().clone();
    workspace
        .mark_pane_running(&project_id, "agent", "codex")
        .unwrap();

    let items = visible_project_items(&workspace);

    assert_eq!(items[0].agent_status.as_deref(), Some("agent running"));
}

#[test]
fn visible_project_items_prioritize_failed_agent_status() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/multi-agent"), multi_agent_layout())
        .unwrap();
    workspace
        .mark_pane_running(&project_id, "agent", "codex")
        .unwrap();
    workspace
        .record_agent_status(&project_id, "agent", "claude", AgentStatus::Failed)
        .unwrap();

    let items = visible_project_items(&workspace);

    assert_eq!(items[0].agent_status.as_deref(), Some("agent failed"));
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
fn visible_tab_items_show_agent_completed_status() {
    let mut workspace = workspace_with_sample_project();
    let project_id = workspace.selected_project_id().unwrap().clone();
    workspace.select_tab("agent").unwrap();
    workspace
        .record_agent_status(&project_id, "agent", "codex", AgentStatus::Completed)
        .unwrap();

    let items = visible_tab_items(&workspace);
    let agent = items.iter().find(|item| item.id == "agent").unwrap();

    assert_eq!(agent.status.as_deref(), Some("started · agent completed"));
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
fn split_drag_delta_maps_to_resize_commands() {
    assert_eq!(
        resize_command_for_drag_delta(SplitDirection::Horizontal, 8.0, 0.0),
        Some(CommandId::PaneResizeRight)
    );
    assert_eq!(
        resize_command_for_drag_delta(SplitDirection::Horizontal, -8.0, 0.0),
        Some(CommandId::PaneResizeLeft)
    );
    assert_eq!(
        resize_command_for_drag_delta(SplitDirection::Vertical, 0.0, 8.0),
        Some(CommandId::PaneResizeDown)
    );
    assert_eq!(
        resize_command_for_drag_delta(SplitDirection::Vertical, 0.0, -8.0),
        Some(CommandId::PaneResizeUp)
    );
    assert_eq!(
        resize_command_for_drag_delta(SplitDirection::Horizontal, 2.0, 0.0),
        None
    );
}

#[test]
fn split_pointer_drag_delta_maps_to_continuous_resize() {
    let resize = pointer_resize_for_drag_delta(SplitDirection::Horizontal, -120.0, 0.0).unwrap();
    assert_eq!(resize.direction, ResizeDirection::Left);
    assert_ratio(resize.delta, 0.2);

    let resize = pointer_resize_for_drag_delta(SplitDirection::Horizontal, 120.0, 0.0).unwrap();
    assert_eq!(resize.direction, ResizeDirection::Right);
    assert_ratio(resize.delta, 0.2);

    let resize = pointer_resize_for_drag_delta(SplitDirection::Vertical, 0.0, -60.0).unwrap();
    assert_eq!(resize.direction, ResizeDirection::Up);
    assert_ratio(resize.delta, 0.1);

    assert!(pointer_resize_for_drag_delta(SplitDirection::Horizontal, 2.0, 0.0).is_none());
}

#[test]
fn root_view_pointer_drag_resize_changes_split_ratio_visibly() {
    let mut root = RootView::dev_fixture();
    let before = root_split_child_basis(root.workspace()).unwrap();

    let resized_ratio = root
        .resize_focused_split_from_pointer_delta(SplitDirection::Horizontal, -120.0, 0.0)
        .unwrap()
        .unwrap();
    let after = root_split_child_basis(root.workspace()).unwrap();

    assert_ratio(before.left, 0.65);
    assert_ratio(resized_ratio, 0.45);
    assert_ratio(after.left, 0.45);
    assert_ratio(after.right, 0.55);
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
fn root_view_syncs_palette_query_from_input_value() {
    let mut root = RootView::dev_fixture();
    root.open_palette(PaletteKind::Tab);
    root.set_palette_query("agent");

    assert!(root.sync_palette_query_from_input_value("dev"));

    assert_eq!(root.visible_palette_titles(), vec!["Dev"]);
}

#[test]
fn root_view_ignores_palette_input_value_without_active_palette() {
    let mut root = RootView::dev_fixture();

    assert!(!root.sync_palette_query_from_input_value("dev"));
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
fn visible_palette_rows_mark_disabled_items() {
    let registry = yttt::commands::default_registry();
    let workspace = Workspace::new();
    let items = yttt::palette::command_palette_items(
        &registry,
        yttt::palette::CommandPaletteContext::from_workspace(&workspace),
    );
    let mut palette = ActivePalette::new(PaletteKind::Command);
    palette.query = "Split Pane Vertically".to_string();

    let rows = visible_palette_rows(&palette, &items);

    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].title, "Split Pane Vertically");
    assert!(!rows[0].enabled);
    assert_eq!(
        rows[0].disabled_reason.as_deref(),
        Some("Open a project first")
    );
}

#[test]
fn palette_empty_label_uses_localized_text() {
    let text = UiText::new(Locale::Chinese);

    assert_eq!(yttt::ui::palette::palette_empty_label(&text), "无结果");
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
fn root_view_confirming_tab_palette_selection_queues_terminal_focus() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Tab);
    root.set_palette_query("agent");
    root.confirm_palette_selection().unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("codex"));
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
fn root_view_confirming_pane_palette_selection_queues_terminal_focus() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Pane);
    root.set_palette_query("shell");
    root.confirm_palette_selection().unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("shell"));
}

#[test]
fn root_view_confirming_disabled_command_palette_item_keeps_palette_open() {
    let mut root = RootView::new();

    root.open_palette(PaletteKind::Command);
    root.set_palette_query("Split Pane Vertically");
    root.confirm_palette_selection().unwrap();

    assert!(root.active_palette().is_some());
    assert_eq!(
        root.visible_error_message(),
        Some("Command unavailable: Open a project first")
    );
}

#[test]
fn root_view_command_palette_can_open_project_palette() {
    let mut root = RootView::new();

    root.open_palette(PaletteKind::Command);
    root.set_palette_query("Open Project Palette");
    root.confirm_palette_selection().unwrap();

    assert!(matches!(
        root.active_palette().map(|palette| palette.kind),
        Some(PaletteKind::Project)
    ));
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
fn root_view_marks_focused_terminal_pane_context() {
    let mut root = RootView::dev_fixture();

    root.focus_visible_terminal_pane("shell").unwrap();
    let contexts = root.visible_terminal_pane_contexts();

    assert!(
        contexts
            .iter()
            .any(|context| context.pane.id == "shell" && context.is_focused)
    );
}

#[test]
fn root_view_focus_visible_terminal_pane_queues_terminal_focus() {
    let mut root = RootView::dev_fixture();

    root.focus_visible_terminal_pane("shell").unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("shell"));
}

#[test]
fn root_view_pane_focus_command_queues_target_terminal_focus() {
    let mut root = RootView::dev_fixture();

    root.run_command(CommandId::PaneFocusRight).unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("shell"));
}

#[test]
fn root_view_split_command_queues_new_terminal_focus() {
    let mut root = RootView::dev_fixture();

    root.run_command(CommandId::PaneSplitVertical).unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("pane-1"));
}

#[test]
fn root_view_terminal_exit_closes_exact_split_pane() {
    let mut root = RootView::dev_fixture();

    root.handle_terminal_pane_exit(terminal_pane_exited_event("dev", "server"))
        .unwrap();

    assert_eq!(visible_pane_titles(root.workspace()), vec!["shell"]);
    assert!(
        root.visible_terminal_pane_contexts()
            .iter()
            .all(|context| context.pane.id != "server")
    );
}

#[test]
fn root_view_terminal_exit_closes_single_pane_tab() {
    let mut root = RootView::dev_fixture();
    root.workspace_mut().select_tab("agent").unwrap();

    root.handle_terminal_pane_exit(terminal_pane_exited_event("agent", "codex"))
        .unwrap();

    assert_eq!(visible_tab_titles(root.workspace()), vec!["Dev"]);
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "dev");
}

#[test]
fn root_view_terminal_exit_keeps_project_open_when_last_tab_closes() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/single"), single_tab_layout())
        .unwrap();
    let mut root = RootView::with_workspace_for_test(workspace);

    root.handle_terminal_pane_exit(TerminalPaneExitedEvent {
        project_id: "/tmp/single".to_string(),
        tab_id: "dev".to_string(),
        pane_id: "shell".to_string(),
        status: ProcessStatus::Exited { code: Some(0) },
        exit_reason: ExitReason::Completed,
    })
    .unwrap();

    assert_eq!(root.workspace().opened_projects().len(), 1);
    assert!(visible_tab_titles(root.workspace()).is_empty());
    assert!(root.visible_terminal_pane_contexts().is_empty());
    assert!(root.selected_project_is_empty());
}

#[test]
fn root_view_focus_notification_target_queues_terminal_focus() {
    let mut root = RootView::dev_fixture();
    let event = notification_event();

    root.focus_notification_target(&event).unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("codex"));
}

#[test]
fn workspace_arrow_keydown_fallback_maps_to_pane_commands() {
    assert_eq!(
        RootView::workspace_arrow_keydown_command("right", true, false, true, false),
        Some(CommandId::PaneFocusRight)
    );
    assert_eq!(
        RootView::workspace_arrow_keydown_command("left", false, true, true, false),
        Some(CommandId::PaneFocusLeft)
    );
    assert_eq!(
        RootView::workspace_arrow_keydown_command("down", true, false, true, true),
        Some(CommandId::PaneResizeDown)
    );
    assert_eq!(
        RootView::workspace_arrow_keydown_command("right", true, false, false, false),
        None
    );
}

#[test]
fn root_view_enqueues_agent_toast_notifications() {
    let mut root = RootView::new();

    root.handle_terminal_notification(notification_event());

    assert_eq!(root.visible_toast_titles(), vec!["Codex completed"]);
}

#[test]
fn root_view_records_agent_status_from_notification() {
    let mut root = RootView::dev_fixture();

    root.handle_terminal_notification(notification_event());

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let pane = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .tab_state("agent")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "codex")
        .unwrap();
    assert_eq!(pane.agent_status, Some(AgentStatus::Completed));
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
fn root_view_focuses_notification_target() {
    let mut root = RootView::dev_fixture();
    let event = notification_event();

    root.focus_notification_target(&event).unwrap();

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    assert_eq!(project_id.as_str(), "/tmp/yttt");
    let project = root.workspace().project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "agent");
    assert_eq!(
        project
            .tab_state("agent")
            .unwrap()
            .focused_pane_id
            .as_deref(),
        Some("codex")
    );
}

#[test]
fn root_view_reports_missing_notification_target() {
    let mut root = RootView::dev_fixture();
    let mut event = notification_event();
    event.pane_id = "missing-pane".to_string();

    let err = root.focus_notification_target(&event).unwrap_err();

    assert!(err.to_string().contains("pane not found: missing-pane"));
    assert_eq!(
        root.visible_error_message(),
        Some("pane not found: missing-pane")
    );
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
fn toast_item_for_event_maps_agent_result_to_component_ready_item() {
    let completed = toast_item_for_event(&notification_event_for(
        "Codex",
        NotificationKind::AgentCompleted,
    ));
    let failed = toast_item_for_event(&notification_event_for(
        "Claude",
        NotificationKind::AgentFailed,
    ));

    assert_eq!(completed.title, "Codex completed");
    assert_eq!(completed.context, "yttt / Agent");
    assert_eq!(completed.tone, ToastTone::Success);
    assert_eq!(failed.title, "Claude failed");
    assert_eq!(failed.context, "yttt / Agent");
    assert_eq!(failed.tone, ToastTone::Error);
}

#[test]
fn terminal_pane_agent_exit_builds_notification_event() {
    let event = notification_for_terminal_pane_exit(terminal_pane_exit_input(
        ProcessStatus::Exited { code: Some(0) },
        ExitReason::Completed,
    ))
    .unwrap();

    assert_eq!(event.kind, NotificationKind::AgentCompleted);
    assert_eq!(event.project_id, "/tmp/yttt");
    assert_eq!(event.tab_id, "agent");
    assert_eq!(event.pane_id, "codex");
    assert_eq!(event.project_title, "yttt");
    assert_eq!(event.tab_title, "Agent");
    assert_eq!(event.pane_title, "Codex");
}

#[test]
fn terminal_pane_exit_event_preserves_process_identity() {
    let event = TerminalPaneExitedEvent {
        project_id: "/tmp/yttt".to_string(),
        tab_id: "dev".to_string(),
        pane_id: "server".to_string(),
        status: ProcessStatus::Exited { code: Some(0) },
        exit_reason: ExitReason::Completed,
    };

    assert_eq!(event.project_id, "/tmp/yttt");
    assert_eq!(event.tab_id, "dev");
    assert_eq!(event.pane_id, "server");
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
        project_id: "/tmp/yttt".to_string(),
        tab_title: "Agent".to_string(),
        tab_id: "agent".to_string(),
        pane_title: "Codex".to_string(),
        pane_id: "codex".to_string(),
        command: "codex".to_string(),
        kind: PaneKind::Agent,
        notify_on_exit: true,
        status,
        exit_reason,
    }
}

fn terminal_pane_exited_event(tab_id: &str, pane_id: &str) -> TerminalPaneExitedEvent {
    TerminalPaneExitedEvent {
        project_id: "/tmp/yttt".to_string(),
        tab_id: tab_id.to_string(),
        pane_id: pane_id.to_string(),
        status: ProcessStatus::Exited { code: Some(0) },
        exit_reason: ExitReason::Completed,
    }
}

fn notification_event() -> NotificationEvent {
    notification_event_for("Codex", NotificationKind::AgentCompleted)
}

fn notification_event_for(pane_title: &str, kind: NotificationKind) -> NotificationEvent {
    NotificationEvent {
        kind,
        project_id: "/tmp/yttt".to_string(),
        tab_id: "agent".to_string(),
        pane_id: "codex".to_string(),
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

fn single_tab_layout() -> yttt::model::layout::ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "single"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"
        layout = { type = "pane", id = "shell", title = "shell", command = "$SHELL" }
    "#,
    )
    .unwrap()
}

fn multi_agent_layout() -> yttt::model::layout::ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "multi-agent"
        default_tab = "agent"

        [[tabs]]
        id = "agent"
        title = "Agent"

        [tabs.layout]
        type = "split"
        direction = "horizontal"
        ratio = 0.5
        left = { type = "pane", id = "codex", title = "Codex", command = "codex", kind = "agent", notify_on_exit = true }
        right = { type = "pane", id = "claude", title = "Claude", command = "claude", notify_on_exit = true }
    "#,
    )
    .unwrap()
}
