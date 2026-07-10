use std::{
    cell::RefCell,
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, SystemTime},
};

use gpui::{AppContext as _, Keystroke, Subscription};
use tempfile::tempdir;
use yttt::{
    commands::CommandId,
    config::{
        paths::AppConfigPaths,
        settings::{AppSettings, LanguageSetting, save_settings},
    },
    model::{
        layout::PaneKind,
        layout::SplitDirection,
        split_tree::ResizeDirection,
        workspace::{AgentStatus, PaneProcessState, Workspace},
    },
    palette::{ActivePalette, PaletteItem, PaletteKind},
    runtime::git_status::{GitFileStatus, GitStatusSummary, parse_git_status_porcelain},
    runtime::notification::{NotificationEvent, NotificationKind},
    runtime::terminal::{ExitReason, ProcessStatus},
    ui::components::SelectableState,
    ui::editor::{
        CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, DiskFingerprint, DocumentId,
        EditorAppearance, EditorDiagnosticSeverity, EditorLanguageId, ProjectEditorDocument,
        ProjectEditorModel, WorkItemId, read_project_file,
    },
    ui::i18n::{Locale, UiText},
    ui::palette::visible_palette_rows,
    ui::primitives::sidebar::SidebarSide,
    ui::project_tree::{DirectorySnapshot, ProjectTreeEntry, ProjectTreeEntryKind},
    ui::sidebar::visible_project_items,
    ui::terminal_pane::{
        PaneLifecycle, TerminalPaneExitInput, TerminalPaneExitedEvent, TerminalSpawnFailure,
        notification_for_terminal_pane_exit, pane_lifecycle_label, spawn_failure_lines,
    },
    ui::toast::{ToastTone, toast_item_for_event, visible_toast_items},
    ui::{
        app::register_workbench_close_guard,
        root::RootView,
        split_view::{
            pointer_resize_for_drag_delta, resize_command_for_drag_delta, root_split_child_basis,
            visible_pane_titles,
        },
        tabs::{
            FileTabSnapshot, WorkbenchTabKind, visible_tab_items, visible_tab_titles,
            visible_work_item_tabs,
        },
    },
    ui::{
        input_owner::{InputOwnerKind, InputOwnerStack, TerminalInputGate},
        interaction::input_owner::{
            InputOwnerRegistration, InputOwnerToken, InputScopeId, TerminalInputPolicy,
        },
        interaction::key_dispatch::workspace_command_for_keystroke,
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
fn git_status_parser_records_file_tones() {
    let parsed = parse_git_status_porcelain(
        "## main\n M src/main.rs\nA  src/lib.rs\n D old.rs\n?? notes.txt\nR  old.txt -> new.txt\n",
    );

    assert_eq!(
        parsed.file_status(Path::new("src/main.rs")),
        Some(GitFileStatus::Modified)
    );
    assert_eq!(
        parsed.file_status(Path::new("src/lib.rs")),
        Some(GitFileStatus::Added)
    );
    assert_eq!(
        parsed.file_status(Path::new("old.rs")),
        Some(GitFileStatus::Deleted)
    );
    assert_eq!(
        parsed.file_status(Path::new("notes.txt")),
        Some(GitFileStatus::Untracked)
    );
    assert_eq!(
        parsed.file_status(Path::new("new.txt")),
        Some(GitFileStatus::Modified)
    );
    assert_eq!(parsed.file_status(Path::new("old.txt")), None);
}

#[test]
fn visible_work_item_tabs_merge_terminal_and_file_items() {
    let workspace = RootView::dev_fixture().workspace().clone();
    let terminal_items = visible_tab_items(&workspace);
    let project_id = workspace.selected_project_id().unwrap().clone();
    let first_file = DocumentId {
        project_id: project_id.clone(),
        canonical_path: PathBuf::from("/tmp/yttt/src/main.rs"),
    };
    let second_file = DocumentId {
        project_id,
        canonical_path: PathBuf::from("/tmp/yttt/README.md"),
    };
    let files = vec![
        FileTabSnapshot {
            id: first_file.clone(),
            relative_path: PathBuf::from("src/main.rs"),
            dirty: true,
        },
        FileTabSnapshot {
            id: second_file.clone(),
            relative_path: PathBuf::from("README.md"),
            dirty: false,
        },
    ];

    let items = visible_work_item_tabs(
        &terminal_items,
        &files,
        Some(&WorkItemId::File(second_file.clone())),
    );

    assert_eq!(items.len(), terminal_items.len() + 2);
    assert!(
        items[..terminal_items.len()]
            .iter()
            .all(|item| item.kind == WorkbenchTabKind::Terminal)
    );
    let first = &items[terminal_items.len()];
    assert_eq!(first.id, WorkItemId::File(first_file));
    assert_eq!(first.kind, WorkbenchTabKind::File);
    assert_eq!(first.title, "main.rs");
    assert_eq!(first.tooltip, "src/main.rs");
    assert!(first.dirty);
    assert_eq!(first.state, SelectableState::Inactive);
    let second = &items[terminal_items.len() + 1];
    assert_eq!(second.id, WorkItemId::File(second_file));
    assert_eq!(second.title, "README.md");
    assert_eq!(second.tooltip, "README.md");
    assert!(!second.dirty);
    assert_eq!(second.state, SelectableState::Active);
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
    let (_temp, root) = english_test_root();

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
fn root_view_toggles_sidebar_collapse_state() {
    let mut root = RootView::dev_fixture();

    assert!(!root.sidebar_is_collapsed());

    root.toggle_sidebar();

    assert!(root.sidebar_is_collapsed());
}

#[test]
fn root_view_right_sidebar_drag_updates_only_selected_project() {
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/sidebar-first"), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(PathBuf::from("/tmp/sidebar-second"), sample_layout())
        .unwrap();
    workspace.select_project(&first).unwrap();
    let (_temp, mut root) = english_test_root_with_workspace(workspace);

    assert_eq!(root.project_sidebar_width(), 216.0);
    assert_eq!(root.selected_project_panel_width(), Some(280.0));
    assert_eq!(
        root.resize_sidebar_from_pointer_delta(SidebarSide::Right, -40.0),
        Some(320.0)
    );
    assert_eq!(root.selected_project_panel_width(), Some(320.0));
    assert_eq!(root.project_sidebar_width(), 216.0);

    root.select_project(&second).unwrap();
    assert_eq!(root.selected_project_panel_width(), Some(280.0));
    root.select_project(&first).unwrap();
    assert_eq!(root.selected_project_panel_width(), Some(320.0));
}

#[test]
fn root_view_sidebar_drag_clamps_both_runtime_widths() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());

    assert_eq!(
        root.resize_sidebar_from_pointer_delta(SidebarSide::Left, 10_000.0),
        Some(420.0)
    );
    assert_eq!(
        root.resize_sidebar_from_pointer_delta(SidebarSide::Left, -10_000.0),
        Some(160.0)
    );
    assert_eq!(
        root.resize_sidebar_from_pointer_delta(SidebarSide::Right, -10_000.0),
        Some(520.0)
    );
    assert_eq!(
        root.resize_sidebar_from_pointer_delta(SidebarSide::Right, 10_000.0),
        Some(200.0)
    );
}

#[test]
fn root_view_sidebar_release_persists_defaults_without_rewriting_other_sessions() {
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/sidebar-persist-first"), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(
            PathBuf::from("/tmp/sidebar-persist-second"),
            sample_layout(),
        )
        .unwrap();
    workspace.select_project(&first).unwrap();
    let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths.clone());

    root.resize_sidebar_from_pointer_delta(SidebarSide::Right, -50.0);
    let before_right_release = yttt::config::settings::load_or_create_settings(&paths).unwrap();
    assert_eq!(before_right_release.settings.project_panel.width, 280.0);
    root.persist_sidebar_width(SidebarSide::Right).unwrap();
    root.resize_sidebar_from_pointer_delta(SidebarSide::Left, 34.0);
    let before_left_release = yttt::config::settings::load_or_create_settings(&paths).unwrap();
    assert_eq!(
        before_left_release
            .settings
            .project_panel
            .project_sidebar_width,
        216.0
    );
    root.toggle_sidebar();
    root.persist_sidebar_width(SidebarSide::Left).unwrap();

    assert_eq!(root.selected_project_panel_width(), Some(330.0));
    assert_eq!(root.project_sidebar_width(), 250.0);
    root.run_command(CommandId::ProjectPanelToggle).unwrap();
    assert!(!root.selected_project_panel_visible());
    assert_eq!(root.selected_project_panel_width(), Some(330.0));
    root.select_project(&second).unwrap();
    assert_eq!(root.selected_project_panel_width(), Some(280.0));

    let future_project = temp.path().join("future-sidebar-project");
    fs::create_dir(&future_project).unwrap();
    root.open_project_path(&future_project).unwrap();
    assert_eq!(root.selected_project_panel_width(), Some(330.0));

    let loaded = yttt::config::settings::load_or_create_settings(&paths).unwrap();
    assert_eq!(loaded.settings.project_panel.width, 330.0);
    assert_eq!(loaded.settings.project_panel.project_sidebar_width, 250.0);
}

#[gpui::test]
fn root_view_renders_sidebar_resize_handles_only_for_visible_expanded_panels(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| RootView::dev_fixture());
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    assert!(cx.debug_bounds("project-sidebar-resize-handle").is_some());
    assert!(
        cx.debug_bounds("project-file-panel-resize-handle")
            .is_some()
    );

    root.update(cx, |root, cx| {
        root.toggle_sidebar();
        cx.notify();
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert!(root.read(app).sidebar_is_collapsed());
        assert!(root.read(app).selected_project_panel_visible());
    });

    root.update(cx, |root, cx| {
        root.run_command(CommandId::ProjectPanelToggle).unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert!(!root.read(app).selected_project_panel_visible());
    });
}

#[test]
fn input_owner_stack_restores_parent_after_nested_owner_closes() {
    let mut stack = InputOwnerStack::default();
    let settings = stack.push_owner(InputOwnerRegistration::blocking(
        InputOwnerKind::Settings,
        InputScopeId::new("settings"),
    ));
    let recorder = stack.push_owner(InputOwnerRegistration::blocking(
        InputOwnerKind::KeybindingRecorder,
        InputScopeId::new("settings.keybindings.command-palette"),
    ));

    assert_eq!(stack.active_kind(), InputOwnerKind::KeybindingRecorder);
    assert_eq!(
        stack.active_scope_id(),
        &InputScopeId::new("settings.keybindings.command-palette")
    );
    let snapshot = stack.pop_owner(recorder);
    assert_eq!(snapshot.active_kind(), InputOwnerKind::Settings);
    assert_eq!(snapshot.active_scope_id(), &InputScopeId::new("settings"));
    assert!(!snapshot.terminal_input_allowed());

    let snapshot = stack.pop_owner(settings);
    assert_eq!(snapshot.active_kind(), InputOwnerKind::Workspace);
    assert_eq!(snapshot.active_scope_id(), &InputScopeId::new("workspace"));
    assert!(snapshot.terminal_input_allowed());
}

#[test]
fn input_owner_stack_records_scope_and_terminal_policy() {
    let mut stack = InputOwnerStack::default();

    let token = stack.push_owner(InputOwnerRegistration::blocking(
        InputOwnerKind::Palette,
        InputScopeId::new("palette.command"),
    ));

    let owner = stack.active_owner();
    assert_eq!(owner.active_kind(), InputOwnerKind::Palette);
    assert_eq!(
        owner.active_scope_id(),
        &InputScopeId::new("palette.command")
    );
    assert_eq!(owner.terminal_input_policy(), TerminalInputPolicy::Block);
    assert!(!owner.terminal_input_allowed());

    let owner = stack.pop_owner(token);
    assert_eq!(owner.active_kind(), InputOwnerKind::Workspace);
    assert_eq!(owner.terminal_input_policy(), TerminalInputPolicy::Allow);
    assert!(owner.terminal_input_allowed());
}

#[test]
fn input_owner_stack_closing_parent_removes_descendants() {
    let mut stack = InputOwnerStack::default();
    let palette = stack.push(InputOwnerKind::Palette);
    let dialog = stack.push(InputOwnerKind::Dialog);
    let recorder = stack.push(InputOwnerKind::KeybindingRecorder);

    assert!(stack.pop(dialog));

    assert_eq!(stack.active_kind(), InputOwnerKind::Palette);
    assert!(!stack.pop(recorder));
    assert_eq!(stack.active_kind(), InputOwnerKind::Palette);
    assert!(stack.pop(palette));
    assert_eq!(stack.active_kind(), InputOwnerKind::Workspace);
}

#[test]
fn input_owner_stack_ignores_unknown_token_without_changing_owner() {
    let mut stack = InputOwnerStack::default();
    let settings = stack.push_owner(InputOwnerRegistration::blocking(
        InputOwnerKind::Settings,
        InputScopeId::new("settings"),
    ));
    let unknown = InputOwnerToken::from_raw_for_test(settings.raw() + 100);

    let snapshot = stack.pop_owner(unknown);

    assert_eq!(snapshot.active_kind(), InputOwnerKind::Settings);
    assert_eq!(snapshot.active_scope_id(), &InputScopeId::new("settings"));
    assert_eq!(stack.active_kind(), InputOwnerKind::Settings);
    assert_eq!(stack.active_scope_id(), &InputScopeId::new("settings"));
}

#[test]
fn terminal_input_gate_allows_only_workspace_owner() {
    let gate = TerminalInputGate::default();
    let mut stack = InputOwnerStack::default();

    gate.sync_from_snapshot(&stack.snapshot());
    assert!(gate.allows_terminal_input());
    assert_eq!(
        stack.snapshot().terminal_input_policy(),
        TerminalInputPolicy::Allow
    );

    let editor = stack.push_owner(InputOwnerRegistration::blocking(
        InputOwnerKind::Editor,
        InputScopeId::new("layout_editor"),
    ));
    gate.sync_from_snapshot(&stack.snapshot());

    assert!(!gate.allows_terminal_input());
    assert_eq!(
        stack.snapshot().terminal_input_policy(),
        TerminalInputPolicy::Block
    );

    stack.pop_owner(editor);
    gate.sync_from_snapshot(&stack.snapshot());

    assert!(gate.allows_terminal_input());
}

#[test]
fn input_owner_stack_focus_restore_target_is_empty_without_registered_focus_handle() {
    let mut stack = InputOwnerStack::default();
    stack.push_owner(InputOwnerRegistration::blocking(
        InputOwnerKind::Settings,
        InputScopeId::new("settings"),
    ));

    assert!(stack.focus_restore_target().is_none());
}

#[test]
fn root_view_double_clicking_tab_opens_rename_dialog() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());

    root.handle_project_tab_click("dev", 2).unwrap();

    assert_eq!(visible_tab_titles(root.workspace())[0], "Dev");
    assert_eq!(
        root.visible_tab_rename_dialog_title().as_deref(),
        Some("Rename tab")
    );
    assert_eq!(root.pending_tab_rename_value().as_deref(), Some("Dev"));
}

#[test]
fn root_view_confirming_tab_rename_uses_entered_title() {
    let mut root = RootView::dev_fixture();

    root.handle_project_tab_click("dev", 2).unwrap();
    root.confirm_tab_rename_dialog("Runtime").unwrap();

    assert_eq!(visible_tab_titles(root.workspace())[0], "Runtime");
    assert!(root.visible_tab_rename_dialog_title().is_none());
}

#[test]
fn root_view_canceling_tab_rename_keeps_title() {
    let mut root = RootView::dev_fixture();

    root.handle_project_tab_click("dev", 2).unwrap();
    root.cancel_tab_rename_dialog();

    assert_eq!(visible_tab_titles(root.workspace())[0], "Dev");
    assert!(root.visible_tab_rename_dialog_title().is_none());
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
    let item = root.visible_error_notification_item().unwrap();
    assert!(item.title.contains("failed to parse project layout"));
    assert_eq!(item.context, "Error");
    assert_eq!(item.tone, ToastTone::Error);
}

#[test]
fn root_view_creates_project_work_item_session_on_open() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("editor-session-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.project_panel.default_open = false;
    settings.project_panel.width = 336.0;
    save_settings(&paths, &settings).unwrap();
    let mut root = RootView::with_config_paths(paths);

    root.open_project_path(&project_dir).unwrap();

    let project_id = root.workspace().selected_project_id().unwrap();
    let project = root.workspace().project(project_id).unwrap();
    let session = root
        .project_editor_runtime()
        .workspace()
        .session(project_id)
        .unwrap();
    assert_eq!(session.file_tree().root(), project.path);
    assert_eq!(
        session.active_work_item(),
        Some(&WorkItemId::Terminal(project.selected_tab_id.clone()))
    );
    assert!(!session.project_panel_visible());
    assert_eq!(session.project_panel_width(), 336.0);
}

#[test]
fn stale_project_tree_result_does_not_replace_refreshed_state() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let stale = root.refresh_project_tree_state(&project_id).unwrap();
    let current = root.refresh_project_tree_state(&project_id).unwrap();

    assert!(!root.apply_project_tree_snapshot(
        &project_id,
        stale.generation,
        DirectorySnapshot {
            relative_directory: PathBuf::new(),
            entries: vec![ProjectTreeEntry {
                name: "stale.rs".into(),
                relative_path: PathBuf::from("stale.rs"),
                kind: ProjectTreeEntryKind::File,
            }],
        },
    ));
    assert!(root.apply_project_tree_snapshot(
        &project_id,
        current.generation,
        DirectorySnapshot {
            relative_directory: PathBuf::new(),
            entries: vec![ProjectTreeEntry {
                name: "current.rs".into(),
                relative_path: PathBuf::from("current.rs"),
                kind: ProjectTreeEntryKind::File,
            }],
        },
    ));

    let rows = root
        .project_editor_runtime()
        .workspace()
        .session(&project_id)
        .unwrap()
        .file_tree()
        .visible_rows();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].relative_path, PathBuf::from("current.rs"));
}

#[test]
fn pending_and_failed_project_file_loads_preserve_active_surface() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let previous = root.active_work_item();

    let stale = root
        .begin_project_file_open(&project_id, Path::new("src/main.rs"))
        .unwrap();
    assert!(
        root.begin_project_file_open(&project_id, Path::new("src/main.rs"))
            .is_none()
    );
    assert!(root.cancel_project_file_open(&stale));
    let current = root
        .begin_project_file_open(&project_id, Path::new("src/main.rs"))
        .unwrap();
    assert!(current.generation > stale.generation);

    assert!(!root.apply_project_file_open_error(&stale, "stale error"));
    assert!(root.apply_project_file_open_error(&current, "unsupported file"));

    assert_eq!(root.active_work_item(), previous);
    assert!(
        root.project_editor_runtime()
            .workspace()
            .session(&project_id)
            .unwrap()
            .file_ids()
            .is_empty()
    );
    assert!(
        root.project_editor_runtime()
            .document(&current.document_id)
            .is_none()
    );
    assert_eq!(root.visible_error_message(), Some("unsupported file"));
}

#[gpui::test]
fn root_view_file_tree_loads_and_opens_a_project_file(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("tree-editor-project");
    fs::create_dir_all(project_dir.join("src")).unwrap();
    fs::write(project_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
    fs::write(project_dir.join("README.md"), "# Tree editor\n").unwrap();
    let paths = english_test_config_paths(&temp);
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(project_dir.clone(), sample_layout())
        .unwrap();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| RootView::with_workspace_for_test_and_config_paths(workspace, paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    cx.run_until_parked();
    assert!(cx.debug_bounds("project-file-panel").is_some());
    let tree = cx
        .read(|app| {
            root.read(app)
                .project_editor_runtime()
                .tree(&project_id)
                .cloned()
        })
        .expect("visible project panel should create a tree entity");
    cx.read(|app| {
        assert!(
            tree.read(app)
                .snapshot()
                .row_for_path(Path::new("src"))
                .is_some()
        );
    });

    tree.update(cx, |tree, tree_cx| {
        assert!(tree.activate_path(Path::new("src"), tree_cx));
    });
    cx.run_until_parked();
    tree.update(cx, |tree, tree_cx| {
        assert!(tree.activate_path(Path::new("src/main.rs"), tree_cx));
    });
    cx.run_until_parked();

    let expected_document_id = DocumentId {
        project_id,
        canonical_path: fs::canonicalize(project_dir.join("src/main.rs")).unwrap(),
    };
    cx.read(|app| {
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::File(expected_document_id.clone()))
        );
        assert!(
            root.read(app)
                .project_editor_runtime()
                .document(&expected_document_id)
                .is_some()
        );
    });
    assert!(cx.debug_bounds("active-file-editor").is_some());
}

#[test]
fn root_view_prebuilt_workspace_creates_all_project_work_item_sessions() {
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/editor-first"), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(PathBuf::from("/tmp/editor-second"), sample_layout())
        .unwrap();
    let (_temp, root) = english_test_root_with_workspace(workspace);

    let state = root.project_editor_runtime().workspace();
    assert_eq!(state.len(), 2);
    assert_eq!(
        state.session(&first).unwrap().active_work_item(),
        Some(&WorkItemId::Terminal("dev".to_string()))
    );
    assert_eq!(
        state.session(&second).unwrap().active_work_item(),
        Some(&WorkItemId::Terminal("dev".to_string()))
    );
}

#[test]
fn root_view_project_switch_preserves_each_editor_session() {
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/editor-first"), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(PathBuf::from("/tmp/editor-second"), sample_layout())
        .unwrap();
    let (_temp, mut root) = english_test_root_with_workspace(workspace);
    let opened_file = {
        let session = root
            .project_editor_runtime_mut()
            .workspace_mut()
            .session_mut(&first)
            .unwrap();
        session.set_project_panel_visible(false);
        session.set_project_panel_width(401.0);
        session.open_file(PathBuf::from("/tmp/editor-first/src/main.rs"))
    };

    root.select_project(&first).unwrap();
    root.select_project(&second).unwrap();
    root.select_project(&first).unwrap();

    let first_session = root
        .project_editor_runtime()
        .workspace()
        .session(&first)
        .unwrap();
    assert_eq!(
        first_session.active_work_item(),
        Some(&WorkItemId::File(opened_file.clone()))
    );
    assert!(!first_session.project_panel_visible());
    assert_eq!(first_session.project_panel_width(), 401.0);
    assert!(
        root.project_editor_runtime()
            .workspace()
            .session(&second)
            .is_some()
    );
    assert_eq!(root.pending_editor_focus_document_id(), Some(&opened_file));
    assert!(root.pending_terminal_focus_pane_id().is_none());
}

#[test]
fn root_view_tab_navigation_crosses_terminal_and_file_work_items() {
    let mut root = RootView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let document_id = root
        .project_editor_runtime_mut()
        .workspace_mut()
        .session_mut(&project_id)
        .unwrap()
        .open_file(PathBuf::from("/tmp/yttt/src/main.rs"));

    root.select_work_item(WorkItemId::Terminal("agent".to_string()))
        .unwrap();
    root.run_command(CommandId::TabNext).unwrap();

    assert_eq!(
        root.active_work_item(),
        Some(WorkItemId::File(document_id.clone()))
    );
    assert_eq!(
        root.workspace()
            .project(&project_id)
            .unwrap()
            .selected_tab_id,
        "agent"
    );

    root.run_command(CommandId::TabNext).unwrap();
    assert_eq!(
        root.active_work_item(),
        Some(WorkItemId::Terminal("dev".to_string()))
    );
    assert_eq!(
        root.workspace()
            .project(&project_id)
            .unwrap()
            .selected_tab_id,
        "dev"
    );

    root.run_command(CommandId::TabPrev).unwrap();
    assert_eq!(root.active_work_item(), Some(WorkItemId::File(document_id)));
}

#[test]
fn root_view_tab_close_dispatches_to_the_active_file_work_item() {
    let mut root = RootView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let document_id = root
        .project_editor_runtime_mut()
        .workspace_mut()
        .session_mut(&project_id)
        .unwrap()
        .open_file(PathBuf::from("/tmp/yttt/src/main.rs"));
    let terminal_count = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .tabs
        .len();

    root.run_command(CommandId::TabClose).unwrap();

    let session = root
        .project_editor_runtime()
        .workspace()
        .session(&project_id)
        .unwrap();
    assert!(!session.file_ids().contains(&document_id));
    assert_eq!(
        session.active_work_item(),
        Some(&WorkItemId::Terminal("agent".to_string()))
    );
    assert_eq!(
        root.workspace()
            .project(&project_id)
            .unwrap()
            .layout
            .tabs
            .len(),
        terminal_count
    );
}

#[test]
fn root_view_tab_palette_lists_and_selects_file_work_items() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let document_id = root
        .project_editor_runtime_mut()
        .workspace_mut()
        .session_mut(&project_id)
        .unwrap()
        .open_file(PathBuf::from("/tmp/yttt/src/main.rs"));
    root.select_work_item(WorkItemId::Terminal("dev".to_string()))
        .unwrap();

    root.open_palette(PaletteKind::Tab);
    let items = root.active_palette_items();
    let file_item = items
        .iter()
        .find(|item| item.id == "file:/tmp/yttt/src/main.rs")
        .unwrap();
    assert_eq!(file_item.title, "main.rs");
    assert_eq!(file_item.subtitle.as_deref(), Some("src/main.rs"));

    root.set_palette_query("main.rs");
    root.confirm_palette_selection().unwrap();

    assert_eq!(root.active_work_item(), Some(WorkItemId::File(document_id)));
    assert_eq!(
        root.workspace()
            .project(&project_id)
            .unwrap()
            .selected_tab_id,
        "dev"
    );
}

#[test]
fn root_view_active_file_owns_input_without_leaking_to_terminal() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let document_id = root
        .project_editor_runtime_mut()
        .workspace_mut()
        .session_mut(&project_id)
        .unwrap()
        .open_file(PathBuf::from("/tmp/yttt/src/main.rs"));

    root.select_work_item(WorkItemId::File(document_id.clone()))
        .unwrap();

    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Editor);
    assert_eq!(
        root.foreground_input_scope_id().as_deref(),
        Some("editor.project_file:/tmp/yttt:/tmp/yttt/src/main.rs")
    );
    assert!(!root.terminal_input_allowed());
    assert_eq!(root.pending_editor_focus_document_id(), Some(&document_id));

    root.select_work_item(WorkItemId::Terminal("dev".to_string()))
        .unwrap();
    assert_eq!(
        root.foreground_input_owner_kind(),
        InputOwnerKind::Workspace
    );
    assert!(root.terminal_input_allowed());
    assert!(root.pending_editor_focus_document_id().is_none());
}

#[gpui::test]
fn root_view_renders_active_file_document_and_consumes_focus(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let mut workspace = workspace_with_sample_project();
    let project_id = workspace.selected_project_id().unwrap().clone();
    workspace.select_project(&project_id).unwrap();
    let document_id = DocumentId {
        project_id: project_id.clone(),
        canonical_path: PathBuf::from("/tmp/yttt/src/main.rs"),
    };
    let expected_document_id = document_id.clone();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|root_cx| {
            let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths);
            root.project_editor_runtime_mut()
                .workspace_mut()
                .session_mut(&project_id)
                .unwrap()
                .open_file(document_id.canonical_path.clone());
            let editor = CodeEditorState::new(
                &document_id.canonical_path,
                CodeEditorConfig::new("main.rs", CodeEditorLanguageMode::Auto),
                "fn main() {}",
            );
            let model = ProjectEditorModel::new(
                document_id.clone(),
                editor,
                DiskFingerprint {
                    exists: true,
                    byte_len: 12,
                    modified: Some(SystemTime::UNIX_EPOCH),
                    content_hash: 1,
                },
            );
            let document = root_cx.new(|document_cx| {
                ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
            });
            root.project_editor_runtime_mut().insert_document(
                document_id.clone(),
                document,
                Subscription::new(|| {}),
            );
            root.select_work_item(WorkItemId::File(document_id))
                .unwrap();
            root
        });
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    assert!(cx.debug_bounds("active-file-editor").is_some());
    assert!(cx.debug_bounds("project-tab-2").is_some());
    cx.read(|app| {
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::File(expected_document_id))
        );
        assert!(root.read(app).pending_editor_focus_document_id().is_none());
    });
}

#[gpui::test]
fn root_view_file_save_preserves_newer_in_flight_edit(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("save-project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join("notes.txt"), "old").unwrap();
    let loaded = read_project_file(&project_dir, Path::new("notes.txt")).unwrap();
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(project_dir.clone(), sample_layout())
        .unwrap();
    let document_id = DocumentId {
        project_id: project_id.clone(),
        canonical_path: loaded.canonical_path.clone(),
    };
    let expected_document_id = document_id.clone();
    let paths = english_test_config_paths(&temp);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|root_cx| {
            let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths);
            root.project_editor_runtime_mut()
                .workspace_mut()
                .session_mut(&project_id)
                .unwrap()
                .open_file(document_id.canonical_path.clone());
            let model = ProjectEditorModel::new(
                document_id.clone(),
                CodeEditorState::new(
                    &document_id.canonical_path,
                    CodeEditorConfig::new("notes.txt", CodeEditorLanguageMode::Auto),
                    loaded.text,
                ),
                loaded.fingerprint,
            );
            let document = root_cx.new(|document_cx| {
                ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
            });
            root.project_editor_runtime_mut().insert_document(
                document_id.clone(),
                document,
                Subscription::new(|| {}),
            );
            root.select_work_item(WorkItemId::File(document_id))
                .unwrap();
            root
        });
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    let document = cx
        .read(|app| {
            root.read(app)
                .project_editor_runtime()
                .document(&expected_document_id)
                .cloned()
        })
        .unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("saved text", window, input_cx);
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert!(document.read(app).model().is_dirty());
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::File(expected_document_id.clone()))
        );
    });

    root.update(cx, |root, cx| {
        root.run_command(CommandId::FileSave).unwrap();
        assert_eq!(root.pending_document_save_count(), 1);
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    cx.read(|app| {
        assert_eq!(root.read(app).pending_document_save_count(), 0);
        assert_eq!(root.read(app).visible_error_message(), None);
    });

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "saved text"
    );
    cx.read(|app| {
        assert!(!document.read(app).model().is_dirty());
    });

    input.update_in(cx, |input, window, input_cx| {
        input.set_value("captured edit", window, input_cx);
    });
    cx.run_until_parked();
    root.update_in(cx, |root, window, root_cx| {
        root.save_active_document(window, root_cx);
    });
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("newer edit", window, input_cx);
    });
    cx.run_until_parked();

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "captured edit"
    );
    cx.read(|app| {
        let model = document.read(app).model();
        assert_eq!(model.value(), "newer edit");
        assert_eq!(model.saved_value(), "captured edit");
        assert!(model.is_dirty());
    });

    fs::remove_dir_all(&project_dir).unwrap();
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("unsaved after failure", window, input_cx);
    });
    cx.run_until_parked();
    root.update_in(cx, |root, window, root_cx| {
        root.save_active_document(window, root_cx);
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert!(document.read(app).model().is_dirty());
        assert!(
            document
                .read(app)
                .model()
                .editor()
                .error()
                .is_some_and(|error| error.contains("Save failed"))
        );
        assert!(
            root.read(app)
                .visible_error_message()
                .is_some_and(|error| error.contains("Save failed"))
        );
    });
}

#[gpui::test]
fn root_view_file_save_requires_resolution_after_external_change(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("conflict-project");
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join("notes.txt"), "old").unwrap();
    let loaded = read_project_file(&project_dir, Path::new("notes.txt")).unwrap();
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(project_dir.clone(), sample_layout())
        .unwrap();
    let document_id = DocumentId {
        project_id: project_id.clone(),
        canonical_path: loaded.canonical_path.clone(),
    };
    let expected_document_id = document_id.clone();
    let paths = english_test_config_paths(&temp);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|root_cx| {
            let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths);
            root.project_editor_runtime_mut()
                .workspace_mut()
                .session_mut(&project_id)
                .unwrap()
                .open_file(document_id.canonical_path.clone());
            let model = ProjectEditorModel::new(
                document_id.clone(),
                CodeEditorState::new(
                    &document_id.canonical_path,
                    CodeEditorConfig::new("notes.txt", CodeEditorLanguageMode::Auto),
                    loaded.text,
                ),
                loaded.fingerprint,
            );
            let document = root_cx.new(|document_cx| {
                ProjectEditorDocument::new(model, EditorAppearance::default(), window, document_cx)
            });
            root.project_editor_runtime_mut().insert_document(
                document_id.clone(),
                document,
                Subscription::new(|| {}),
            );
            root.select_work_item(WorkItemId::File(document_id))
                .unwrap();
            root
        });
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    let document = cx
        .read(|app| {
            root.read(app)
                .project_editor_runtime()
                .document(&expected_document_id)
                .cloned()
        })
        .unwrap();
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("memory text", window, input_cx);
    });
    cx.run_until_parked();
    fs::write(project_dir.join("notes.txt"), "external text").unwrap();

    root.update(cx, |root, cx| {
        root.run_command(CommandId::FileSave).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "external text"
    );
    cx.read(|app| {
        assert!(root.read(app).has_pending_file_conflict());
        assert!(document.read(app).model().is_dirty());
        assert_eq!(
            root.read(app).visible_file_conflict_dialog_actions(),
            vec!["Cancel", "Reload", "Overwrite"]
        );
        assert_eq!(
            root.read(app).foreground_input_owner_kind(),
            InputOwnerKind::Dialog
        );
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    assert!(cx.debug_bounds("file-conflict-dialog").is_some());

    root.update_in(cx, |root, window, cx| {
        root.reload_pending_file_conflict(window, cx);
    });
    cx.run_until_parked();

    cx.read(|app| {
        assert!(!root.read(app).has_pending_file_conflict());
        assert_eq!(document.read(app).model().value(), "external text");
        assert_eq!(input.read(app).value().to_string(), "external text");
        assert!(!document.read(app).model().is_dirty());
    });

    input.update_in(cx, |input, window, input_cx| {
        input.set_value("second memory text", window, input_cx);
    });
    cx.run_until_parked();
    fs::write(project_dir.join("notes.txt"), "second external text").unwrap();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::FileSave).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.read(|app| {
        assert!(root.read(app).has_pending_file_conflict());
    });

    root.update_in(cx, |root, window, cx| {
        root.overwrite_pending_file_conflict(window, cx);
    });
    cx.run_until_parked();

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "second memory text"
    );
    cx.read(|app| {
        assert!(!root.read(app).has_pending_file_conflict());
        assert!(!document.read(app).model().is_dirty());
    });

    fs::write(project_dir.join("notes.txt"), "clean external reload").unwrap();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::FileSave).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.read(|app| {
        assert!(!root.read(app).has_pending_file_conflict());
        assert_eq!(document.read(app).model().value(), "clean external reload");
        assert!(!document.read(app).model().is_dirty());
    });

    input.update_in(cx, |input, window, input_cx| {
        input.set_value("recreated text", window, input_cx);
    });
    cx.run_until_parked();
    fs::remove_file(project_dir.join("notes.txt")).unwrap();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::FileSave).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.read(|app| {
        assert!(root.read(app).pending_file_conflict_is_missing());
        assert_eq!(
            root.read(app).visible_file_conflict_dialog_actions(),
            vec!["Cancel", "Recreate file"]
        );
    });
    root.update_in(cx, |root, window, cx| {
        root.overwrite_pending_file_conflict(window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "recreated text"
    );

    fs::write(project_dir.join("notes.txt"), "refresh boundary text").unwrap();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::ProjectPanelRefresh).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(document.read(app).model().value(), "refresh boundary text");
        assert!(!document.read(app).model().is_dirty());
        assert!(!root.read(app).has_pending_file_conflict());
    });

    input.update_in(cx, |input, window, input_cx| {
        input.set_value("dirty refresh text", window, input_cx);
    });
    cx.run_until_parked();
    fs::write(project_dir.join("notes.txt"), "external refresh conflict").unwrap();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::ProjectPanelRefresh).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.read(|app| {
        assert!(root.read(app).has_pending_file_conflict());
        assert!(document.read(app).model().is_dirty());
    });
    root.update(cx, |root, cx| {
        root.cancel_pending_file_conflict(cx);
        cx.notify();
    });
    cx.read(|app| {
        assert!(!root.read(app).has_pending_file_conflict());
        assert_eq!(document.read(app).model().value(), "dirty refresh text");
        assert!(document.read(app).model().is_dirty());
    });
}

#[gpui::test]
fn root_view_delayed_autosave_discards_stale_generation(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) =
        project_file_autosave_fixture(cx, "after_delay", 50);
    let input = cx.read(|app| document.read(app).input().clone());

    input.update_in(cx, |input, window, input_cx| {
        input.set_value("first delayed edit", window, input_cx);
        input.set_value("latest delayed edit", window, input_cx);
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(
            root.read(app).editor_autosave(),
            yttt::config::settings::EditorAutosave::AfterDelay
        );
        assert!(
            root.read(app)
                .project_editor_runtime()
                .autosave_task_is_scheduled(document.read(app).model().document_id())
        );
    });
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "latest delayed edit"
    );
    cx.read(|app| {
        assert!(!document.read(app).model().is_dirty());
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::File(
                document.read(app).model().document_id().clone()
            ))
        );
    });

    input.update_in(cx, |input, window, input_cx| {
        input.set_value("captured overlapping save", window, input_cx);
    });
    cx.run_until_parked();
    root.update_in(cx, |root, window, root_cx| {
        root.save_active_document(window, root_cx);
    });
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("latest overlapping autosave", window, input_cx);
    });
    cx.run_until_parked();
    cx.executor().advance_clock(Duration::from_millis(50));
    cx.run_until_parked();
    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "latest overlapping autosave"
    );
    cx.read(|app| {
        assert!(!document.read(app).model().is_dirty());
    });
}

#[gpui::test]
fn root_view_focus_change_autosave_saves_dirty_file(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) =
        project_file_autosave_fixture(cx, "on_focus_change", 1000);
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("focus change edit", window, input_cx);
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.select_work_item(WorkItemId::Terminal("dev".to_string()))
            .unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "focus change edit"
    );
    cx.read(|app| {
        assert!(!document.read(app).model().is_dirty());
    });
}

#[gpui::test]
fn root_view_autosave_off_does_not_save_on_focus_change(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 5);
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("manual only edit", window, input_cx);
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.select_work_item(WorkItemId::Terminal("dev".to_string()))
            .unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "old"
    );
    cx.read(|app| {
        assert!(document.read(app).model().is_dirty());
    });
}

#[gpui::test]
fn closing_dirty_file_requires_a_decision(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, _project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("unsaved close edit", window, input_cx);
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.run_command(CommandId::TabClose).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.read(|app| {
        assert!(root.read(app).has_pending_dirty_close());
        assert_eq!(
            root.read(app).visible_dirty_close_actions(),
            vec!["Cancel", "Discard", "Save"]
        );
        assert!(
            root.read(app)
                .project_editor_runtime()
                .document(&document_id)
                .is_some()
        );
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::File(document_id.clone()))
        );
    });
    assert!(cx.debug_bounds("dirty-close-dialog").is_some());

    root.update(cx, |root, cx| {
        root.cancel_pending_dirty_close();
        cx.notify();
    });
    cx.read(|app| {
        assert!(!root.read(app).has_pending_dirty_close());
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::File(document_id.clone()))
        );
    });

    root.update(cx, |root, cx| {
        root.run_command(CommandId::TabClose).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    root.update_in(cx, |root, window, cx| {
        root.discard_pending_dirty_close(window, cx);
    });
    cx.read(|app| {
        assert!(!root.read(app).has_pending_dirty_close());
        assert!(
            root.read(app)
                .project_editor_runtime()
                .document(&document_id)
                .is_none()
        );
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::Terminal("agent".to_string()))
        );
    });
}

#[gpui::test]
fn saving_a_dirty_file_continues_the_pending_close(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("save before close", window, input_cx);
    });
    cx.run_until_parked();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::TabClose).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    root.update_in(cx, |root, window, cx| {
        root.save_pending_dirty_close(window, cx);
    });
    cx.run_until_parked();

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "save before close"
    );
    cx.read(|app| {
        assert!(!root.read(app).has_pending_dirty_close());
        assert!(
            root.read(app)
                .project_editor_runtime()
                .document(&document_id)
                .is_none()
        );
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::Terminal("agent".to_string()))
        );
    });
}

#[gpui::test]
fn failed_save_keeps_dirty_file_and_pending_close_open(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("cannot save this edit", window, input_cx);
    });
    cx.run_until_parked();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::TabClose).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    fs::remove_dir_all(&project_dir).unwrap();

    root.update_in(cx, |root, window, cx| {
        root.save_pending_dirty_close(window, cx);
    });
    cx.run_until_parked();
    cx.refresh().unwrap();

    cx.read(|app| {
        let root = root.read(app);
        assert!(root.has_pending_dirty_close());
        assert!(
            root.project_editor_runtime()
                .document(&document_id)
                .is_some()
        );
        assert!(document.read(app).model().is_dirty());
        assert!(
            root.visible_error_message()
                .is_some_and(|error| error.contains("Save failed"))
        );
    });
    assert!(cx.debug_bounds("dirty-close-dialog").is_some());
}

#[gpui::test]
fn closing_project_combines_dirty_files_and_running_processes(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, _project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let project_id = cx.read(|app| document.read(app).model().document_id().project_id.clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("unsaved project edit", window, input_cx);
    });
    root.update(cx, |root, cx| {
        root.workspace_mut()
            .mark_pane_running(&project_id, "dev", "server")
            .unwrap();
        root.run_command(CommandId::ProjectClose).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    cx.read(|app| {
        let root = root.read(app);
        assert!(root.workspace().project(&project_id).is_some());
        assert!(root.has_pending_project_close());
        assert!(root.has_pending_dirty_close());
        assert_eq!(
            root.visible_dirty_close_dialog_text().as_deref(),
            Some("Close project?\n1 unsaved file: notes.txt\n1 running process")
        );
        assert_eq!(
            root.visible_dirty_close_actions(),
            vec!["Cancel", "Discard and Continue", "Save All and Continue"]
        );
    });

    root.update_in(cx, |root, window, cx| {
        root.discard_pending_dirty_close(window, cx);
    });

    cx.read(|app| {
        let root = root.read(app);
        assert!(root.workspace().project(&project_id).is_none());
        assert!(!root.has_pending_project_close());
        assert!(!root.has_pending_dirty_close());
    });
}

#[gpui::test]
fn saving_dirty_project_files_continues_the_pending_project_close(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let project_id = cx.read(|app| document.read(app).model().document_id().project_id.clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("save project before close", window, input_cx);
    });
    root.update(cx, |root, cx| {
        root.workspace_mut()
            .mark_pane_running(&project_id, "dev", "server")
            .unwrap();
        root.run_command(CommandId::ProjectClose).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    root.update_in(cx, |root, window, cx| {
        root.save_pending_dirty_close(window, cx);
    });
    cx.run_until_parked();

    assert_eq!(
        fs::read_to_string(project_dir.join("notes.txt")).unwrap(),
        "save project before close"
    );
    cx.read(|app| {
        let root = root.read(app);
        assert!(root.workspace().project(&project_id).is_none());
        assert!(!root.has_pending_project_close());
        assert!(!root.has_pending_dirty_close());
    });
}

#[gpui::test]
fn closing_window_with_dirty_file_is_blocked_by_the_workbench_guard(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, _project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        input.set_value("unsaved window edit", window, input_cx);
    });
    cx.run_until_parked();

    assert!(!cx.simulate_close());
    cx.run_until_parked();

    cx.read(|app| {
        let root = root.read(app);
        assert!(root.has_pending_dirty_close());
        assert_eq!(
            root.visible_dirty_close_actions(),
            vec!["Cancel", "Discard and Continue", "Save All and Continue"]
        );
    });

    let allowed_retry = root.update_in(cx, |root, window, cx| {
        root.discard_pending_dirty_close(window, cx);
        root.request_window_close(cx)
    });
    assert!(allowed_retry);
    cx.read(|app| assert!(!root.read(app).has_pending_dirty_close()));
}

#[test]
fn root_view_file_surface_blocks_terminal_only_commands() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let document_id = root
        .project_editor_runtime_mut()
        .workspace_mut()
        .session_mut(&project_id)
        .unwrap()
        .open_file(PathBuf::from("/tmp/yttt/src/main.rs"));
    let before = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .clone();

    root.run_command(CommandId::TabNew).unwrap();
    root.run_command(CommandId::TabRename).unwrap();
    root.run_command(CommandId::PaneSplitVertical).unwrap();

    assert_eq!(
        root.workspace().project(&project_id).unwrap().layout,
        before
    );
    assert_eq!(root.active_work_item(), Some(WorkItemId::File(document_id)));
    assert!(root.visible_tab_rename_dialog_title().is_none());
    assert_eq!(
        root.visible_error_message(),
        Some("Switch to a terminal tab first")
    );
}

#[test]
fn root_view_file_surface_has_no_pane_palette_items() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let document_id = root
        .project_editor_runtime_mut()
        .workspace_mut()
        .session_mut(&project_id)
        .unwrap()
        .open_file(PathBuf::from("/tmp/yttt/src/main.rs"));
    root.select_work_item(WorkItemId::File(document_id))
        .unwrap();

    root.open_palette(PaletteKind::Pane);

    assert!(root.active_palette_items().is_empty());
}

#[test]
fn root_view_confirmed_project_close_removes_only_its_editor_runtime() {
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/editor-first"), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(PathBuf::from("/tmp/editor-second"), sample_layout())
        .unwrap();
    workspace.select_project(&first).unwrap();
    workspace
        .mark_pane_running(&first, "dev", "server")
        .unwrap();
    let (_temp, mut root) = english_test_root_with_workspace(workspace);
    let first_document = DocumentId {
        project_id: first.clone(),
        canonical_path: PathBuf::from("/tmp/editor-first/src/main.rs"),
    };
    let second_document = DocumentId {
        project_id: second.clone(),
        canonical_path: PathBuf::from("/tmp/editor-second/src/main.rs"),
    };
    root.project_editor_runtime_mut()
        .track_tree_load(first.clone(), 7);
    root.project_editor_runtime_mut()
        .track_tree_load(second.clone(), 11);
    root.project_editor_runtime_mut()
        .track_file_load(first_document.clone(), 13);
    root.project_editor_runtime_mut()
        .track_file_load(second_document.clone(), 17);

    root.run_command(CommandId::ProjectClose).unwrap();
    assert!(root.has_pending_project_close());
    root.confirm_pending_project_close().unwrap();

    let runtime = root.project_editor_runtime();
    assert!(runtime.workspace().session(&first).is_none());
    assert!(runtime.workspace().session(&second).is_some());
    assert!(!runtime.tree_load_is_current(&first, 7));
    assert!(runtime.tree_load_is_current(&second, 11));
    assert!(!runtime.file_load_is_current(&first_document, 13));
    assert!(runtime.file_load_is_current(&second_document, 17));
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
fn root_view_exposes_global_default_layout_source() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("source-message-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    root.open_project_path(&project_dir).unwrap();

    assert_eq!(
        root.visible_layout_source_message(),
        Some("Layout source: global default")
    );
}

#[test]
fn root_view_project_open_surfaces_personal_layout_warning() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("personal-warning-project");
    fs::create_dir_all(project_dir.join(".yttt")).unwrap();
    fs::write(
        project_dir.join(".yttt/layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let local = paths.local_layout_file(&project_dir.canonicalize().unwrap());
    fs::create_dir_all(local.parent().unwrap()).unwrap();
    fs::write(
        &local,
        "version = 1\nmode = \"patch\"\nunknown = true\nlayout = {}",
    )
    .unwrap();
    let mut root = RootView::with_config_paths(paths);

    root.open_project_path(&project_dir).unwrap();

    let notification = root.visible_error_notification_item().unwrap();
    assert!(notification.title.contains("invalid personal layout"));
    assert!(notification.title.contains(&local.display().to_string()));
    assert_eq!(notification.tone, ToastTone::Error);
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
    root.run_command(CommandId::LayoutSaveCurrent).unwrap();

    root.run_command(CommandId::LayoutOpenFile).unwrap();

    assert_eq!(
        root.last_opened_layout_file(),
        Some(expected_layout_file.as_path())
    );
    assert_eq!(root.visible_error_message(), None);
    assert!(
        root.pending_status_notification_titles()
            .iter()
            .any(|title| title.contains("layout.toml"))
    );
}

#[test]
fn root_view_layout_default_editor_opens_without_project() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let expected_layout_file = paths.default_layout_file();
    let mut root = RootView::with_config_paths(paths);

    root.run_command(CommandId::LayoutDefaultEdit).unwrap();

    assert!(root.layout_toml_editor_is_open());
    assert_eq!(
        root.layout_toml_editor_path(),
        Some(expected_layout_file.as_path())
    );
    assert!(
        root.layout_toml_editor_value()
            .unwrap()
            .contains("[project]")
    );
    assert_eq!(root.layout_editor_target_kind(), Some("default"));
    assert_eq!(
        root.foreground_input_scope_id().as_deref(),
        Some("editor.default_layout")
    );
    assert_eq!(
        root.visible_layout_toml_editor_language_id(),
        Some(EditorLanguageId::Toml)
    );
    assert_eq!(root.visible_layout_toml_editor_error(), None);
}

#[test]
fn root_view_persists_editor_language_settings() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    assert!(root.editor_auto_detect_language());
    assert_eq!(root.editor_default_language(), "plain_text");
    assert!(!root.editor_lsp_enabled());

    root.set_editor_auto_detect_language(false).unwrap();
    root.set_editor_default_language("toml").unwrap();
    root.set_editor_lsp_enabled(true).unwrap();
    root.set_editor_lsp_command("taplo lsp stdio").unwrap();

    let loaded = yttt::config::settings::load_or_create_settings(&paths).unwrap();
    assert!(!loaded.settings.editor.auto_detect_language);
    assert_eq!(loaded.settings.editor.default_language, "toml");
    assert!(loaded.settings.editor.lsp.enabled);
    assert_eq!(loaded.settings.editor.lsp.command, "taplo lsp stdio");
}

#[test]
fn root_view_layout_default_editor_saves_valid_toml() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let layout_file = paths.default_layout_file();
    let mut root = RootView::with_config_paths(paths);
    root.run_command(CommandId::LayoutDefaultEdit).unwrap();

    let updated = root
        .layout_toml_editor_value()
        .unwrap()
        .replace("title = \"Shell\"", "title = \"Saved Shell\"");
    root.set_layout_toml_editor_value(updated);
    root.save_layout_toml_editor().unwrap();

    assert!(!root.layout_toml_editor_is_open());
    assert!(
        fs::read_to_string(layout_file)
            .unwrap()
            .contains("title = \"Saved Shell\"")
    );
}

#[test]
fn root_view_layout_default_editor_keeps_invalid_toml_open() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);
    root.run_command(CommandId::LayoutDefaultEdit).unwrap();

    root.set_layout_toml_editor_value("[project\n");
    root.save_layout_toml_editor().unwrap();

    assert!(root.layout_toml_editor_is_open());
    assert!(
        root.visible_layout_toml_editor_error()
            .unwrap()
            .contains("failed to parse layout TOML")
    );
}

#[test]
fn root_view_layout_default_editor_records_parse_diagnostic() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);
    root.run_command(CommandId::LayoutDefaultEdit).unwrap();

    root.set_layout_toml_editor_value("[project\n");
    root.save_layout_toml_editor().unwrap();

    let diagnostics = root.visible_layout_toml_editor_diagnostics();
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].severity, EditorDiagnosticSeverity::Error);
    assert_eq!(diagnostics[0].source, "toml");
    assert!(
        diagnostics[0]
            .message
            .contains("failed to parse layout TOML")
    );
}

#[test]
fn root_view_layout_default_editor_clears_diagnostics_after_valid_save() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);
    root.run_command(CommandId::LayoutDefaultEdit).unwrap();

    let valid = root.layout_toml_editor_value().unwrap().to_string();
    root.set_layout_toml_editor_value("[project\n");
    root.save_layout_toml_editor().unwrap();
    assert!(!root.visible_layout_toml_editor_diagnostics().is_empty());

    root.set_layout_toml_editor_value(valid);
    root.save_layout_toml_editor().unwrap();

    assert!(root.visible_layout_toml_editor_diagnostics().is_empty());
}

#[test]
fn root_view_layout_project_editor_selects_project_and_personal_formats() {
    use yttt::config::layout_loader::{LayoutOverride, serialize_personal_patch};

    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let shared_project = temp.path().join("shared-project");
    fs::create_dir_all(shared_project.join(".yttt")).unwrap();
    fs::write(
        shared_project.join(".yttt/layout.toml"),
        toml::to_string_pretty(&sample_layout()).unwrap(),
    )
    .unwrap();
    let mut root = RootView::with_config_paths(paths.clone());
    root.open_project_path(&shared_project).unwrap();
    root.run_command(CommandId::LayoutProjectEdit).unwrap();
    assert_eq!(root.layout_editor_target_kind(), Some("project_config"));
    root.cancel_layout_toml_editor();

    let personal_patch = paths.local_layout_file(&shared_project.canonicalize().unwrap());
    fs::create_dir_all(personal_patch.parent().unwrap()).unwrap();
    fs::write(
        &personal_patch,
        serialize_personal_patch(&LayoutOverride::default()).unwrap(),
    )
    .unwrap();
    root.run_command(CommandId::LayoutProjectEdit).unwrap();
    assert_eq!(root.layout_editor_target_kind(), Some("personal_patch"));
    root.cancel_layout_toml_editor();

    fs::write(
        &personal_patch,
        "version = 1\nmode = \"patch\"\nunknown = true\nlayout = {}",
    )
    .unwrap();
    root.run_command(CommandId::LayoutProjectEdit).unwrap();
    assert_eq!(root.layout_editor_target_kind(), Some("invalid_personal"));
    assert!(!root.visible_layout_toml_editor_diagnostics().is_empty());

    root.set_layout_toml_editor_value(
        serialize_personal_patch(&LayoutOverride::default()).unwrap(),
    );
    root.save_layout_toml_editor().unwrap();
    assert!(!root.layout_toml_editor_is_open());
    assert!(
        fs::read_to_string(personal_patch)
            .unwrap()
            .contains("mode = \"patch\"")
    );
}

#[test]
fn root_view_layout_project_editor_creates_personal_replace_for_inherited_project() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("inherited-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let expected = paths.local_layout_file(&project_dir.canonicalize().unwrap());
    let mut root = RootView::with_config_paths(paths);
    root.open_project_path(&project_dir).unwrap();

    root.run_command(CommandId::LayoutProjectEdit).unwrap();

    assert_eq!(root.layout_editor_target_kind(), Some("personal_replace"));
    assert_eq!(root.layout_toml_editor_path(), Some(expected.as_path()));
    assert!(
        fs::read_to_string(expected)
            .unwrap()
            .contains("mode = \"replace\"")
    );
    assert_eq!(
        root.foreground_input_scope_id().as_deref(),
        Some("editor.project_layout")
    );
}

#[test]
fn root_view_layout_project_commands_without_project_show_localized_reason() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);
    root.set_language(LanguageSetting::Chinese).unwrap();

    root.run_command(CommandId::LayoutProjectEdit).unwrap();

    assert_eq!(root.visible_error_message(), Some("请先打开项目"));
    assert!(!root.layout_toml_editor_is_open());
}

#[test]
fn root_view_project_close_command_requires_confirmation_for_running_project() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
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
    assert_eq!(root.visible_error_message(), None);
    assert!(
        root.pending_status_notification_titles()
            .iter()
            .any(|title| title.contains("keybindings.toml"))
    );
    assert!(paths.keybindings_file().exists());
}

#[test]
fn root_view_status_reveals_settings_paths_without_error_banner() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    root.show_settings_file_path_status();
    root.show_themes_directory_status();

    assert_eq!(root.visible_error_message(), None);
    let titles = root.pending_status_notification_titles();
    assert!(titles.iter().any(|title| title.contains("settings.toml")));
    assert!(titles.iter().any(|title| title.contains("themes")));
}

#[test]
fn root_view_settings_open_command_opens_settings_page() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    root.run_command(CommandId::SettingsOpen).unwrap();

    assert!(root.settings_is_open());
    assert_eq!(
        root.visible_settings_group_titles(),
        vec![
            "General",
            "Appearance",
            "Languages",
            "Terminal",
            "Default Layout",
            "Keybindings"
        ]
    );
    assert_eq!(root.selected_settings_group_title(), Some("General"));
}

#[test]
fn root_view_settings_search_filters_groups() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);
    root.open_settings();

    root.set_settings_search_query("shell");

    assert_eq!(root.visible_settings_group_titles(), vec!["Terminal"]);
    assert_eq!(root.selected_settings_group_title(), Some("Terminal"));
}

#[test]
fn root_view_settings_can_select_and_close_group() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);
    root.open_settings();

    root.select_settings_group("terminal").unwrap();
    root.close_settings();

    assert!(!root.settings_is_open());
    assert_eq!(root.selected_settings_group_title(), Some("Terminal"));
}

#[test]
fn root_view_toggles_system_notifications() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    assert!(!root.system_notifications_enabled());
    assert_eq!(
        root.visible_notification_settings_message(),
        "System notifications: disabled"
    );

    root.run_command(CommandId::SettingsNotifications).unwrap();

    assert!(root.system_notifications_enabled());
    assert_eq!(root.visible_error_message(), None);
    assert_eq!(
        root.visible_notification_settings_message(),
        "System notifications: enabled"
    );
    assert_eq!(
        root.pending_status_notification_titles(),
        vec!["System notifications: enabled".to_string()]
    );

    let reloaded = RootView::with_config_paths(paths);
    assert!(reloaded.system_notifications_enabled());
    assert_eq!(
        reloaded.visible_notification_settings_message(),
        "System notifications: enabled"
    );
}

#[test]
fn root_view_language_setting_persists_and_updates_visible_text() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    root.set_language(LanguageSetting::Chinese).unwrap();

    assert_eq!(
        root.visible_empty_workspace_actions(),
        vec!["打开目录", "打开最近项目", "命令面板"]
    );

    let reloaded = RootView::with_config_paths(paths);
    assert_eq!(
        reloaded.visible_empty_workspace_actions(),
        vec!["打开目录", "打开最近项目", "命令面板"]
    );
}

#[test]
fn root_view_status_notifications_use_selected_language() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    root.set_language(LanguageSetting::Chinese).unwrap();
    root.run_command(CommandId::SettingsNotifications).unwrap();
    root.run_command(CommandId::SettingsKeybindings).unwrap();

    let titles = root.pending_status_notification_titles();
    assert!(titles.iter().any(|title| title == "系统通知：已启用"));
    assert!(titles.iter().any(|title| title.starts_with("快捷键文件: ")));
    assert_eq!(root.visible_error_message(), None);
}

#[test]
fn root_view_language_setting_updates_settings_labels() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);
    root.open_settings();

    root.set_language(LanguageSetting::Chinese).unwrap();

    assert_eq!(
        root.visible_settings_group_titles(),
        vec!["通用", "外观", "语言", "终端", "默认布局", "快捷键"]
    );
    assert_eq!(root.selected_settings_group_title(), Some("通用"));

    root.set_settings_search_query("Shell");

    assert_eq!(root.visible_settings_group_titles(), vec!["终端"]);
    assert_eq!(root.selected_settings_group_title(), Some("终端"));
}

#[test]
fn root_view_language_setting_updates_command_palette_labels() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    root.set_language(LanguageSetting::Chinese).unwrap();
    root.open_palette(PaletteKind::Command);

    let items = root.active_palette_items();
    let open_project = items
        .iter()
        .find(|item| item.command == CommandId::ProjectOpen)
        .unwrap();
    assert_eq!(open_project.title, "打开项目");
    assert_eq!(open_project.subtitle.as_deref(), Some("选择一个项目目录"));
    assert!(open_project.enabled);
    assert_eq!(open_project.disabled_reason.as_deref(), None);

    let new_tab = items
        .iter()
        .find(|item| item.command == CommandId::TabNew)
        .unwrap();
    assert_eq!(new_tab.title, "新建标签页");
    assert_eq!(new_tab.disabled_reason.as_deref(), Some("请先打开项目"));
}

#[test]
fn root_view_command_palette_items_show_current_platform_keybindings() {
    let (_temp, mut root) = english_test_root();

    root.open_palette(PaletteKind::Command);

    let items = root.active_palette_items();
    let command_palette = items
        .iter()
        .find(|item| item.command == CommandId::CommandPaletteOpen)
        .unwrap();
    let tab_new = items
        .iter()
        .find(|item| item.command == CommandId::TabNew)
        .unwrap();

    if cfg!(target_os = "macos") {
        assert_eq!(command_palette.keybinding.as_deref(), Some("cmd-p"));
        assert_eq!(tab_new.keybinding.as_deref(), Some("cmd-t"));
    } else {
        assert_eq!(command_palette.keybinding.as_deref(), Some("ctrl-p"));
        assert_eq!(tab_new.keybinding.as_deref(), Some("ctrl-t"));
    }
}

#[test]
fn root_view_command_palette_can_request_open_project() {
    let (_temp, mut root) = english_test_root();

    root.open_palette(PaletteKind::Command);
    root.set_palette_query("Open Project");
    root.confirm_palette_selection().unwrap();

    assert!(root.active_palette().is_none());
    assert!(root.take_pending_open_project_request());
    assert_eq!(root.visible_error_message(), None);
}

#[test]
fn root_view_closes_requested_tab_by_id() {
    let mut root = RootView::dev_fixture();

    root.close_project_tab("agent").unwrap();

    assert_eq!(visible_tab_titles(root.workspace()), vec!["Dev"]);
}

#[test]
fn root_view_terminal_close_on_exit_setting_persists() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    assert!(root.terminal_close_on_exit());

    root.set_terminal_close_on_exit(false).unwrap();

    assert!(!root.terminal_close_on_exit());
    assert!(!RootView::with_config_paths(paths).terminal_close_on_exit());
}

#[test]
fn root_view_terminal_shell_setting_changes_new_shell_tabs() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("shell-settings-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);
    root.open_project_path(&project_dir).unwrap();

    root.set_terminal_shell("/bin/bash").unwrap();
    root.run_command(CommandId::TabNew).unwrap();

    let project_id = root.workspace().selected_project_id().unwrap();
    let project = root.workspace().project(project_id).unwrap();
    let tab = project.layout.tab(&project.selected_tab_id).unwrap();
    let pane = tab.layout.find_pane("shell").unwrap();
    assert_eq!(pane.command, "/bin/bash");
}

#[test]
fn root_view_terminal_display_settings_persist() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    root.set_terminal_font_family("JetBrains Mono").unwrap();
    root.set_terminal_font_size(14.5).unwrap();
    root.set_terminal_line_height(1.2).unwrap();
    root.set_terminal_padding(8.0).unwrap();
    root.set_terminal_scrollback(20000).unwrap();
    root.set_terminal_show_scrollbar(false).unwrap();

    let runtime = &root.theme_runtime().terminal_settings;
    assert_eq!(runtime.font_family, "JetBrains Mono");
    assert_eq!(runtime.font_size, 14.5);
    assert_eq!(runtime.line_height, 1.2);
    assert_eq!(runtime.padding, 8.0);
    assert_eq!(runtime.scrollback, 20000);
    assert!(!runtime.show_scrollbar);

    let reloaded = RootView::with_config_paths(paths);
    let terminal = &reloaded.theme_runtime().terminal_settings;
    assert_eq!(terminal.font_family, "JetBrains Mono");
    assert_eq!(terminal.font_size, 14.5);
    assert_eq!(terminal.line_height, 1.2);
    assert_eq!(terminal.padding, 8.0);
    assert_eq!(terminal.scrollback, 20000);
    assert!(!terminal.show_scrollbar);
}

#[test]
fn root_view_does_not_auto_focus_workspace_while_settings_is_open() {
    let mut root = RootView::new();

    assert!(root.should_auto_focus_workspace());
    root.open_settings();
    assert!(!root.should_auto_focus_workspace());
    root.close_settings();
    assert!(root.should_auto_focus_workspace());
}

#[test]
fn root_view_terminal_input_owner_tracks_foreground_overlays() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths);

    assert_eq!(
        root.foreground_input_owner_kind(),
        InputOwnerKind::Workspace
    );
    assert!(root.terminal_input_allowed());

    root.open_palette(PaletteKind::Command);
    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Palette);
    assert!(!root.terminal_input_allowed());

    root.close_palette();
    assert_eq!(
        root.foreground_input_owner_kind(),
        InputOwnerKind::Workspace
    );
    assert!(root.terminal_input_allowed());

    root.open_settings();
    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Settings);
    assert!(!root.terminal_input_allowed());

    root.open_layout_toml_editor().unwrap();
    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Dialog);
    assert!(!root.terminal_input_allowed());

    root.cancel_layout_toml_editor();
    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Settings);
    assert!(!root.terminal_input_allowed());

    root.close_settings();
    assert_eq!(
        root.foreground_input_owner_kind(),
        InputOwnerKind::Workspace
    );
    assert!(root.terminal_input_allowed());
}

#[test]
fn root_view_exposes_foreground_input_scope_id() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths);

    assert_eq!(
        root.foreground_input_scope_id().as_deref(),
        Some("workspace")
    );

    root.open_palette(PaletteKind::Command);
    assert_eq!(
        root.foreground_input_scope_id().as_deref(),
        Some("palette.command")
    );

    root.close_palette();
    root.open_settings();
    assert_eq!(
        root.foreground_input_scope_id().as_deref(),
        Some("settings")
    );

    root.open_layout_toml_editor().unwrap();
    assert_eq!(
        root.foreground_input_scope_id().as_deref(),
        Some("editor.default_layout")
    );
}

#[test]
fn root_view_workspace_keybindings_are_blocked_by_foreground_owner() {
    let mut root = RootView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let initial_tab_count = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .tabs
        .len();

    root.open_settings();

    assert!(!root.dispatch_runtime_keybinding(&Keystroke::parse("cmd-t").unwrap()));
    assert_eq!(
        root.workspace()
            .project(&project_id)
            .unwrap()
            .layout
            .tabs
            .len(),
        initial_tab_count
    );
}

#[test]
fn root_view_layout_editor_blocks_project_file_save_binding() {
    let mut root = RootView::dev_fixture();
    root.open_layout_toml_editor().unwrap();

    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Dialog);
    assert!(!root.dispatch_runtime_keybinding(&Keystroke::parse("cmd-s").unwrap()));
}

#[test]
fn key_dispatch_blocks_workspace_commands_for_foreground_owner() {
    let command = workspace_command_for_keystroke(
        InputOwnerKind::Settings,
        &Keystroke::parse("cmd-t").unwrap(),
        |_| Some(CommandId::TabNew),
        |_| false,
    );

    assert_eq!(command, None);
}

#[test]
fn key_dispatch_leaves_terminal_bytes_for_terminal_input() {
    let command = workspace_command_for_keystroke(
        InputOwnerKind::Workspace,
        &Keystroke::parse("ctrl-c").unwrap(),
        |_| Some(CommandId::TabNew),
        |_| true,
    );

    assert_eq!(command, None);
}

#[test]
fn key_dispatch_allows_workspace_command_when_terminal_does_not_need_key() {
    let command = workspace_command_for_keystroke(
        InputOwnerKind::Workspace,
        &Keystroke::parse("cmd-t").unwrap(),
        |_| Some(CommandId::TabNew),
        |_| false,
    );

    assert_eq!(command, Some(CommandId::TabNew));
}

#[test]
fn root_view_dialog_owner_blocks_terminal_input() {
    let mut root = RootView::dev_fixture();

    root.handle_project_tab_click("dev", 2).unwrap();

    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Dialog);
    assert!(!root.terminal_input_allowed());
}

#[test]
fn root_view_does_not_consume_terminal_focus_while_overlay_is_open() {
    let mut root = RootView::dev_fixture();

    root.focus_visible_terminal_pane("shell").unwrap();
    assert_eq!(root.pending_terminal_focus_pane_id(), Some("shell"));

    root.open_settings();
    assert!(!root.take_pending_terminal_focus_for_render("shell"));
    assert_eq!(root.pending_terminal_focus_pane_id(), Some("shell"));

    root.close_settings();
    assert!(root.take_pending_terminal_focus_for_render("shell"));
    assert_eq!(root.pending_terminal_focus_pane_id(), None);
}

#[test]
fn root_view_does_not_use_palette_text_fallback_when_input_is_focused() {
    let mut root = RootView::new();

    root.open_palette(PaletteKind::Command);

    assert!(!root.should_use_palette_text_fallback(true));
    assert!(root.should_use_palette_text_fallback(false));
}

#[test]
fn root_view_notification_settings_can_be_disabled_again() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    root.run_command(CommandId::SettingsNotifications).unwrap();
    root.run_command(CommandId::SettingsNotifications).unwrap();

    assert!(!root.system_notifications_enabled());
    assert_eq!(root.visible_error_message(), None);
    assert_eq!(
        root.visible_notification_settings_message(),
        "System notifications: disabled"
    );
    assert_eq!(
        root.pending_status_notification_titles(),
        vec![
            "System notifications: enabled".to_string(),
            "System notifications: disabled".to_string()
        ]
    );

    let reloaded = RootView::with_config_paths(paths);
    assert!(!reloaded.system_notifications_enabled());
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
fn root_view_keybindings_editor_updates_and_persists_command_keys() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths.clone());

    root.set_keybinding_command_keys(CommandId::TabPalette, vec!["cmd-l".to_string()])
        .unwrap();

    let row = root
        .visible_keybinding_rows()
        .into_iter()
        .find(|row| row.command == CommandId::TabPalette)
        .unwrap();
    assert_eq!(row.keys, vec!["cmd-l".to_string()]);

    let reloaded = RootView::with_config_paths(paths);
    let row = reloaded
        .visible_keybinding_rows()
        .into_iter()
        .find(|row| row.command == CommandId::TabPalette)
        .unwrap();
    assert_eq!(row.keys, vec!["cmd-l".to_string()]);
}

#[test]
fn root_view_runtime_keybindings_follow_edited_settings() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    root.set_keybinding_command_keys(CommandId::TabPalette, vec!["cmd-l".to_string()])
        .unwrap();

    assert_eq!(
        root.runtime_command_for_keystroke(&Keystroke::parse("cmd-l").unwrap()),
        Some(CommandId::TabPalette)
    );
    assert_eq!(
        root.runtime_command_for_keystroke(&Keystroke::parse("cmd-j").unwrap()),
        None
    );
}

#[test]
fn root_view_keybindings_editor_rejects_conflicts() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    let error = root
        .set_keybinding_command_keys(CommandId::TabPalette, vec!["cmd-p".to_string()])
        .unwrap_err();

    assert!(error.to_string().contains("conflicting keybindings"));
}

#[test]
fn root_view_keybinding_edit_dialog_updates_command_keys() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = RootView::with_config_paths(paths);

    root.open_keybinding_edit_dialog(CommandId::TabPalette)
        .unwrap();

    assert_eq!(
        root.pending_keybinding_edit_value().as_deref(),
        Some("cmd-j, ctrl-j")
    );
    assert_eq!(
        root.foreground_input_owner_kind(),
        InputOwnerKind::KeybindingRecorder
    );

    root.confirm_keybinding_edit_dialog("cmd-l, ctrl-l")
        .unwrap();

    assert!(root.pending_keybinding_edit_value().is_none());
    assert_eq!(
        root.visible_keybinding_rows()
            .into_iter()
            .find(|row| row.command == CommandId::TabPalette)
            .unwrap()
            .keys,
        vec!["cmd-l".to_string(), "ctrl-l".to_string()]
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
    let (_temp, mut root) = english_test_root();

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
    let (_temp, mut root) = english_test_root();

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
fn root_view_leaves_terminal_control_keybindings_for_focused_terminal() {
    let mut root = RootView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let initial_tab_count = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .tabs
        .len();

    root.focus_visible_terminal_pane("shell").unwrap();

    assert!(!root.dispatch_runtime_keybinding(&Keystroke::parse("ctrl-t").unwrap()));
    assert_eq!(
        root.workspace()
            .project(&project_id)
            .unwrap()
            .layout
            .tabs
            .len(),
        initial_tab_count
    );
}

#[test]
fn root_view_routes_terminal_special_keys_to_focused_terminal() {
    let mut root = RootView::dev_fixture();

    root.focus_visible_terminal_pane("shell").unwrap();

    for keys in ["ctrl-c", "tab", "enter", "escape", "up"] {
        assert!(
            root.terminal_should_receive_keystroke(&Keystroke::parse(keys).unwrap()),
            "{keys} should be routed to the focused terminal"
        );
    }
}

#[test]
fn root_view_keeps_platform_shortcuts_available_when_terminal_is_focused() {
    let mut root = RootView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let initial_tab_count = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .tabs
        .len();

    root.focus_visible_terminal_pane("shell").unwrap();

    assert!(root.dispatch_runtime_keybinding(&Keystroke::parse("cmd-t").unwrap()));
    assert_eq!(
        root.workspace()
            .project(&project_id)
            .unwrap()
            .layout
            .tabs
            .len(),
        initial_tab_count + 1
    );
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
fn root_view_terminal_exit_reconciles_active_work_item() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    root.select_work_item(WorkItemId::Terminal("agent".to_string()))
        .unwrap();

    root.handle_terminal_pane_exit(terminal_pane_exited_event("agent", "codex"))
        .unwrap();

    assert_eq!(
        root.active_work_item(),
        Some(WorkItemId::Terminal("dev".to_string()))
    );
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
fn root_view_terminal_exit_keeps_split_pane_when_close_on_exit_is_disabled() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.settings_file(),
        r#"
[terminal]
close_on_exit = false
"#,
    )
    .unwrap();
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths);

    let outcome = root
        .handle_terminal_pane_exit(terminal_pane_exited_event("dev", "server"))
        .unwrap();

    assert_eq!(
        outcome,
        yttt::model::workspace::PaneExitCloseOutcome::PaneKept
    );
    assert_eq!(
        visible_pane_titles(root.workspace()),
        vec!["server", "shell"]
    );
    let project_id = root.workspace().selected_project_id().unwrap();
    let project = root.workspace().project(project_id).unwrap();
    let server = project
        .tab_state("dev")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "server")
        .unwrap();
    assert_eq!(server.process_state, PaneProcessState::Exited);
}

#[test]
fn root_view_terminal_exit_keeps_single_pane_tab_when_close_on_exit_is_disabled() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.settings_file(),
        r#"
[terminal]
close_on_exit = false
"#,
    )
    .unwrap();
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths);
    root.workspace_mut().select_tab("agent").unwrap();

    root.handle_terminal_pane_exit(terminal_pane_exited_event("agent", "codex"))
        .unwrap();

    assert_eq!(visible_tab_titles(root.workspace()), vec!["Dev", "Agent"]);
    let project_id = root.workspace().selected_project_id().unwrap();
    let project = root.workspace().project(project_id).unwrap();
    assert_eq!(project.selected_tab_id, "agent");
    let codex = project
        .tab_state("agent")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "codex")
        .unwrap();
    assert_eq!(codex.process_state, PaneProcessState::Exited);
}

#[test]
fn root_view_terminal_exit_keeps_last_tab_when_close_on_exit_is_disabled() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.settings_file(),
        r#"
[terminal]
close_on_exit = false
"#,
    )
    .unwrap();
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/single"), single_tab_layout())
        .unwrap();
    let mut root = RootView::with_workspace_for_test_and_config_paths(workspace, paths);

    root.handle_terminal_pane_exit(TerminalPaneExitedEvent {
        project_id: "/tmp/single".to_string(),
        tab_id: "dev".to_string(),
        pane_id: "shell".to_string(),
        status: ProcessStatus::Exited { code: Some(0) },
        exit_reason: ExitReason::Completed,
    })
    .unwrap();

    assert_eq!(root.workspace().opened_projects().len(), 1);
    assert_eq!(visible_tab_titles(root.workspace()), vec!["Dev"]);
    assert!(!root.selected_project_is_empty());
    let project_id = root.workspace().selected_project_id().unwrap();
    let project = root.workspace().project(project_id).unwrap();
    assert_eq!(
        project.tab_state("dev").unwrap().pane_states[0].process_state,
        PaneProcessState::Exited
    );
}

#[test]
fn root_view_focus_notification_target_queues_terminal_focus() {
    let mut root = RootView::dev_fixture();
    let event = notification_event();

    root.focus_notification_target(&event).unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("codex"));
}

#[test]
fn root_view_focus_notification_target_leaves_active_file_for_terminal() {
    let mut root = RootView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let document_id = root
        .project_editor_runtime_mut()
        .workspace_mut()
        .session_mut(&project_id)
        .unwrap()
        .open_file(PathBuf::from("/tmp/yttt/src/main.rs"));
    root.select_work_item(WorkItemId::File(document_id))
        .unwrap();

    root.focus_notification_target(&notification_event())
        .unwrap();

    assert_eq!(
        root.active_work_item(),
        Some(WorkItemId::Terminal("agent".to_string()))
    );
    assert_eq!(
        root.foreground_input_owner_kind(),
        InputOwnerKind::Workspace
    );
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
    assert_eq!(
        RootView::workspace_arrow_keydown_command_for_owner(
            InputOwnerKind::Editor,
            "right",
            true,
            false,
            true,
            false,
        ),
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
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    let mut event = notification_event();
    event.pane_id = "missing-pane".to_string();

    let err = root.focus_notification_target(&event).unwrap_err();

    assert!(err.to_string().contains("pane not found: missing-pane"));
    assert_eq!(
        root.visible_error_message(),
        Some("pane not found: missing-pane")
    );
    let item = root.visible_error_notification_item().unwrap();
    assert_eq!(item.title, "pane not found: missing-pane");
    assert_eq!(item.context, "Error");
    assert_eq!(item.tone, ToastTone::Error);
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
        keybinding: None,
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

fn english_test_root() -> (tempfile::TempDir, RootView) {
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    (temp, RootView::with_config_paths(paths))
}

fn english_test_root_with_workspace(workspace: Workspace) -> (tempfile::TempDir, RootView) {
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    (
        temp,
        RootView::with_workspace_for_test_and_config_paths(workspace, paths),
    )
}

fn english_test_config_paths(temp: &tempfile::TempDir) -> AppConfigPaths {
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::create_dir_all(paths.config_dir()).unwrap();
    fs::write(
        paths.settings_file(),
        r#"
[general]
language = "en"
"#,
    )
    .unwrap();
    paths
}

fn project_file_autosave_fixture<'a>(
    cx: &'a mut gpui::TestAppContext,
    autosave: &str,
    delay_ms: u64,
) -> (
    tempfile::TempDir,
    PathBuf,
    gpui::Entity<RootView>,
    gpui::Entity<ProjectEditorDocument>,
    &'a mut gpui::VisualTestContext,
) {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join(format!("autosave-{autosave}"));
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join("notes.txt"), "old").unwrap();
    let paths = english_test_config_paths(&temp);
    fs::write(
        paths.settings_file(),
        format!(
            r#"
[general]
language = "en"

[editor]
autosave = "{autosave}"
autosave_delay_ms = {delay_ms}
"#
        ),
    )
    .unwrap();
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(project_dir.clone(), sample_layout())
        .unwrap();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| RootView::with_workspace_for_test_and_config_paths(workspace, paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        register_workbench_close_guard(window, cx, &root);
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    cx.run_until_parked();
    let tree = cx
        .read(|app| {
            root.read(app)
                .project_editor_runtime()
                .tree(&project_id)
                .cloned()
        })
        .unwrap();
    tree.update(cx, |tree, tree_cx| {
        assert!(tree.activate_path(Path::new("notes.txt"), tree_cx));
    });
    cx.run_until_parked();
    let document_id = DocumentId {
        project_id,
        canonical_path: fs::canonicalize(project_dir.join("notes.txt")).unwrap(),
    };
    let document = cx
        .read(|app| {
            root.read(app)
                .project_editor_runtime()
                .document(&document_id)
                .cloned()
        })
        .unwrap();

    (temp, project_dir, root, document, cx)
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
