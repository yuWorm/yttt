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
        default_layout::{BuiltinAgent, DefaultLayoutKind, DefaultLayoutTemplate},
        keybindings::default_keybindings,
        paths::AppConfigPaths,
        settings::{
            AppSettings, EditorAutosave, LanguageSetting, WindowBackgroundEffect,
            load_or_create_settings, save_settings,
        },
    },
    model::{
        layout::{
            LayoutNode, PaneKind, ProcessExitBehavior, SplitDirection, TerminalExecutionMode,
        },
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
    ui::notifications::{ToastTone, toast_item_for_event, visible_toast_items},
    ui::palette::visible_palette_rows,
    ui::primitives::sidebar::SidebarSide,
    ui::project_tree::{DirectorySnapshot, ProjectTreeEntry, ProjectTreeEntryKind},
    ui::terminal::pane::{
        PaneLifecycle, TerminalPaneExitInput, TerminalPaneExitedEvent, TerminalPaneStartedEvent,
        TerminalSpawnFailure, notification_for_terminal_pane_exit, pane_lifecycle_label,
        spawn_failure_lines,
    },
    ui::workbench::shell::sidebar::visible_project_items,
    ui::{
        app::{register_workbench_close_guard, register_workbench_keybinding_interceptor},
        workbench::WorkbenchView,
        workbench::shell::split_view::{
            pointer_resize_for_drag_delta, resize_command_for_drag_delta, root_split_child_basis,
            visible_pane_titles,
        },
        workbench::shell::tabs::{
            FileTabSnapshot, WorkbenchTabCloseScope, WorkbenchTabKind, visible_tab_items,
            visible_tab_titles, visible_work_item_tabs,
        },
    },
    ui::{
        interaction::actions::{CreateProject, TabCloseAllTerminals},
        interaction::input_owner::{
            InputOwnerKind, InputOwnerRegistration, InputOwnerStack, InputOwnerToken, InputScopeId,
            TerminalInputGate, TerminalInputPolicy,
        },
        interaction::key_dispatch::workspace_command_for_keystroke,
    },
};
use yttt_terminal::{TerminalCursorShape, TerminalOsc52Policy};

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
fn git_status_parser_propagates_ignored_directory_status_without_marking_tree_dirty() {
    let parsed = parse_git_status_porcelain("## main\n!! target/\n!! .env\n");

    assert_eq!(
        parsed.file_status(Path::new("target/debug/app")),
        Some(GitFileStatus::Ignored)
    );
    assert_eq!(
        parsed.file_status(Path::new(".env")),
        Some(GitFileStatus::Ignored)
    );
    assert!(parsed.summary.is_clean());
}

#[test]
fn visible_work_item_tabs_merge_terminal_and_file_items() {
    let workspace = WorkbenchView::dev_fixture().workspace().clone();
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
    let root = WorkbenchView::new();

    let info = root.visible_titlebar_info();

    assert_eq!(info.project_name, "yttt");
    assert!(info.compact_path.is_none());
    assert!(info.git_branch.is_none());
}

#[test]
fn root_view_titlebar_info_describes_selected_project() {
    let root = WorkbenchView::dev_fixture();

    let info = root.visible_titlebar_info();

    assert_eq!(info.project_name, "yttt");
    assert_eq!(info.compact_path.as_deref(), Some("/tmp/yttt"));
}

#[test]
fn root_view_starts_with_empty_workspace() {
    let root = WorkbenchView::new();

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
    let root = WorkbenchView::dev_fixture();

    assert_eq!(root.workspace().opened_projects().len(), 1);
}

#[test]
fn root_view_toggles_sidebar_collapse_state() {
    let mut root = WorkbenchView::dev_fixture();

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
    let mut root =
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths.clone());

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
fn titlebar_action_buttons_open_command_picker_and_settings(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::dev_fixture());
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    cx.run_until_parked();

    let command_button = cx
        .debug_bounds("titlebar-command-palette")
        .expect("titlebar should expose a command picker button");
    assert!(cx.debug_bounds("titlebar-settings").is_some());
    cx.simulate_click(command_button.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(
            root.read(app).active_palette().map(|palette| palette.kind),
            Some(PaletteKind::Command)
        );
    });

    root.update(cx, |root, cx| {
        root.close_palette();
        cx.notify();
    });
    cx.run_until_parked();
    let settings_button = cx
        .debug_bounds("titlebar-settings")
        .expect("titlebar should expose a settings button");
    cx.simulate_click(settings_button.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        assert!(root.read(app).settings_is_open());
    });
}

#[gpui::test]
fn root_view_renders_sidebar_resize_handles_only_for_visible_expanded_panels(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::dev_fixture());
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    assert!(cx.debug_bounds("project-sidebar-resize-handle").is_some());
    assert!(
        cx.debug_bounds("project-file-panel-resize-handle")
            .is_some()
    );
    assert!(cx.debug_bounds("project-sidebar-initial-0").is_none());

    root.update(cx, |root, cx| {
        root.toggle_sidebar();
        cx.notify();
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert!(root.read(app).sidebar_is_collapsed());
        assert!(root.read(app).selected_project_panel_visible());
    });
    assert!(cx.debug_bounds("project-sidebar-initial-0").is_some());

    root.update(cx, |root, cx| {
        root.run_command(CommandId::ProjectPanelToggle).unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert!(!root.read(app).selected_project_panel_visible());
    });
}

#[gpui::test]
fn project_sidebar_context_menu_can_create_project(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::dev_fixture());
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.toggle_sidebar();
        cx.notify();
    });
    cx.run_until_parked();

    let project = cx
        .debug_bounds("project-sidebar-initial-0")
        .expect("collapsed project sidebar should expose its project item");
    cx.simulate_mouse_down(
        project.center(),
        gpui::MouseButton::Right,
        gpui::Modifiers::none(),
    );
    cx.run_until_parked();
    cx.simulate_keystrokes("down enter");
    cx.run_until_parked();

    assert!(cx.did_prompt_for_new_path());
    cx.simulate_new_path_selection(|_| None);
    cx.run_until_parked();
}

#[gpui::test]
fn root_view_error_notification_close_button_dismisses_error(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::dev_fixture());
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        let mut event = notification_event();
        event.pane_id = "missing-pane".to_string();
        assert!(root.focus_notification_target(&event).is_err());
        cx.notify();
    });
    cx.refresh().unwrap();

    let close = cx
        .debug_bounds("error-notification-close")
        .expect("error notification should expose a visible close button");
    cx.simulate_click(close.center(), gpui::Modifiers::none());
    cx.refresh().unwrap();

    cx.read(|app| {
        let message = root.read(app).visible_error_message();
        assert!(
            message.is_none(),
            "clicking the close button should dismiss the error notification, got {message:?}"
        );
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
    let mut root = WorkbenchView::dev_fixture();

    root.handle_project_tab_click("dev", 2).unwrap();
    root.confirm_tab_rename_dialog("Runtime").unwrap();

    assert_eq!(visible_tab_titles(root.workspace())[0], "Runtime");
    assert!(root.visible_tab_rename_dialog_title().is_none());
}

#[test]
fn root_view_canceling_tab_rename_keeps_title() {
    let mut root = WorkbenchView::dev_fixture();

    root.handle_project_tab_click("dev", 2).unwrap();
    root.cancel_tab_rename_dialog();

    assert_eq!(visible_tab_titles(root.workspace())[0], "Dev");
    assert!(root.visible_tab_rename_dialog_title().is_none());
}

#[test]
fn root_view_agent_exit_fixture_contains_sample_project() {
    let root = WorkbenchView::agent_exit_fixture();

    assert_eq!(root.workspace().opened_projects().len(), 1);
}

#[test]
fn first_run_onboarding_persists_separate_tabs_and_does_not_repeat() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    assert_eq!(
        root.onboarding_layout_kind(),
        Some(DefaultLayoutKind::SplitPane)
    );
    assert_eq!(root.onboarding_agent(), Some(BuiltinAgent::Codex));
    assert!(root.complete_onboarding(false).is_err());

    root.select_onboarding_agent(BuiltinAgent::OpenCode);
    assert_eq!(root.onboarding_agent(), Some(BuiltinAgent::Codex));
    root.advance_onboarding();
    root.select_onboarding_layout(DefaultLayoutKind::SeparateTabs);
    root.advance_onboarding();
    root.select_onboarding_agent(BuiltinAgent::OpenCode);
    root.advance_onboarding();
    root.complete_onboarding(false).unwrap();

    assert_eq!(root.onboarding_agent(), None);
    assert_eq!(root.onboarding_layout_kind(), None);
    assert!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .onboarding_completed
    );

    let template: DefaultLayoutTemplate =
        toml::from_str(&fs::read_to_string(paths.default_layout_file()).unwrap()).unwrap();
    assert_eq!(template.project.default_tab.as_deref(), Some("agent"));
    assert_eq!(template.tabs.len(), 2);
    let LayoutNode::Pane(agent) = &template.tabs[0].layout else {
        panic!("first saved tab should contain the selected agent");
    };
    assert_eq!(agent.command, "opencode");
    assert_eq!(agent.kind, PaneKind::Agent);
    assert!(matches!(&template.tabs[1].layout, LayoutNode::Pane(shell) if shell.id == "shell"));

    let restarted = WorkbenchView::with_config_paths(paths);
    assert_eq!(restarted.onboarding_agent(), None);
}

#[test]
fn first_run_onboarding_persists_split_view() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    root.advance_onboarding();
    root.advance_onboarding();
    root.select_onboarding_agent(BuiltinAgent::Claude);
    root.advance_onboarding();
    root.complete_onboarding(false).unwrap();

    let template: DefaultLayoutTemplate =
        toml::from_str(&fs::read_to_string(paths.default_layout_file()).unwrap()).unwrap();
    assert_eq!(template.project.default_tab.as_deref(), Some("workspace"));
    assert_eq!(template.tabs.len(), 1);
    let LayoutNode::Split(split) = &template.tabs[0].layout else {
        panic!("saved split view should contain one split tab");
    };
    assert!(matches!(split.left.as_ref(), LayoutNode::Pane(agent) if agent.command == "claude"));
    assert!(matches!(split.right.as_ref(), LayoutNode::Pane(shell) if shell.id == "shell"));
}

#[test]
fn force_onboarding_overrides_the_persisted_completion_marker() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.general.onboarding_completed = true;
    save_settings(&paths, &settings).unwrap();

    let normal = WorkbenchView::with_config_paths(paths.clone());
    assert_eq!(normal.onboarding_layout_kind(), None);

    let forced = WorkbenchView::with_config_paths_and_force_onboarding(paths.clone(), true);
    assert_eq!(
        forced.onboarding_layout_kind(),
        Some(DefaultLayoutKind::SplitPane)
    );
    assert_eq!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .language,
        LanguageSetting::System,
        "forced onboarding must not rerun first-launch language detection"
    );
    assert!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .onboarding_completed
    );
}

#[gpui::test]
fn first_run_onboarding_selects_layout_before_agent(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let view_paths = paths.clone();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths(view_paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    cx.run_until_parked();
    assert!(cx.debug_bounds("onboarding-language-step").is_some());
    assert!(cx.debug_bounds("onboarding-language-en").is_some());
    assert!(cx.debug_bounds("onboarding-language-zh-cn").is_some());
    assert!(cx.debug_bounds("onboarding-layout-split").is_none());
    assert!(cx.debug_bounds("onboarding-agent-codex").is_none());
    let detected_language = load_or_create_settings(&paths)
        .unwrap()
        .settings
        .general
        .language;
    assert_ne!(detected_language, LanguageSetting::System);
    cx.read(|app| {
        assert_eq!(
            root.read(app).onboarding_language(),
            Some(detected_language)
        );
    });

    let command_palette = cx
        .debug_bounds("onboarding-open-command-palette")
        .expect("command palette hint should be actionable");
    cx.simulate_click(command_palette.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(
            root.read(app).active_palette().map(|palette| palette.kind),
            Some(PaletteKind::Command)
        );
    });
    root.update(cx, |root, cx| {
        root.close_palette();
        cx.notify();
    });
    cx.run_until_parked();

    let chinese = cx
        .debug_bounds("onboarding-language-zh-cn")
        .expect("Chinese language choice should render");
    cx.simulate_click(chinese.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(
            root.read(app).onboarding_language(),
            Some(LanguageSetting::Chinese)
        );
    });
    assert_eq!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .language,
        LanguageSetting::Chinese
    );
    let language_next = cx
        .debug_bounds("onboarding-language-next")
        .expect("language step should expose next");
    cx.simulate_click(language_next.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("onboarding-language-step").is_none());
    assert!(cx.debug_bounds("onboarding-layout-split").is_some());
    assert!(cx.debug_bounds("onboarding-layout-tabs").is_some());
    assert!(cx.debug_bounds("onboarding-next").is_some());

    let tabs = cx
        .debug_bounds("onboarding-layout-tabs")
        .expect("separate tabs layout should render");
    cx.simulate_click(tabs.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(
            root.read(app).onboarding_layout_kind(),
            Some(DefaultLayoutKind::SeparateTabs)
        );
    });

    let next = cx
        .debug_bounds("onboarding-next")
        .expect("layout step should expose next");
    cx.simulate_click(next.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("onboarding-layout-split").is_none());
    for (agent, selector) in [
        ("codex", "onboarding-agent-codex"),
        ("claude", "onboarding-agent-claude"),
        ("opencode", "onboarding-agent-opencode"),
        ("pi", "onboarding-agent-pi"),
        ("omp", "onboarding-agent-omp"),
    ] {
        assert!(
            cx.debug_bounds(selector).is_some(),
            "{agent} choice should render on the second step"
        );
    }

    let back = cx
        .debug_bounds("onboarding-back")
        .expect("agent step should allow returning to layouts");
    cx.simulate_click(back.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("onboarding-layout-tabs").is_some());
    let next = cx.debug_bounds("onboarding-next").unwrap();
    cx.simulate_click(next.center(), gpui::Modifiers::none());
    cx.run_until_parked();

    let opencode = cx
        .debug_bounds("onboarding-agent-opencode")
        .expect("OpenCode choice should render");
    cx.simulate_click(opencode.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| {
        assert_eq!(
            root.read(app).onboarding_agent(),
            Some(BuiltinAgent::OpenCode)
        );
    });

    let agent_next = cx
        .debug_bounds("onboarding-agent-next")
        .expect("agent step should continue to Zed import");
    cx.simulate_click(agent_next.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    assert!(cx.debug_bounds("onboarding-zed-import-step").is_some());
    assert!(cx.debug_bounds("onboarding-zed-back").is_some());

    let finish = if let Some(skip) = cx.debug_bounds("onboarding-zed-skip") {
        assert!(
            cx.debug_bounds("onboarding-zed-ui-theme-0").is_some()
                || cx.debug_bounds("onboarding-zed-icon-theme-0").is_some(),
            "detected Zed themes should be listed before confirmation"
        );
        skip
    } else {
        cx.debug_bounds("onboarding-zed-continue")
            .expect("empty Zed import step should allow finishing")
    };
    cx.simulate_click(finish.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.read(|app| assert_eq!(root.read(app).onboarding_agent(), None));
    assert!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .onboarding_completed
    );
}

#[test]
fn root_view_open_project_path_records_visible_load_error() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("broken-project");
    let project_config_dir = project_dir.join(".yttt");
    fs::create_dir_all(&project_config_dir).unwrap();
    fs::write(project_config_dir.join("layout.toml"), "[project\n").unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths);

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
    let mut root = WorkbenchView::with_config_paths(paths);

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
        .open_project(project_dir.clone(), file_editor_layout())
        .unwrap();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root =
            cx.new(|_| WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths));
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
    let document = cx.read(|app| {
        assert_eq!(
            root.read(app).active_work_item(),
            Some(WorkItemId::File(expected_document_id.clone()))
        );
        let document = root
            .read(app)
            .project_editor_runtime()
            .document(&expected_document_id)
            .cloned()
            .expect("opened project file should create an editor document");
        assert_eq!(document.read(app).breadcrumb_header(), "src/main.rs");
        document
    });
    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
        window.dispatch_action(Box::new(gpui_component::input::Search), document_cx);
    });
    cx.run_until_parked();
    cx.refresh().unwrap();

    assert!(cx.debug_bounds("active-file-editor").is_some());
    assert!(cx.debug_bounds("project-editor-header").is_none());
    let breadcrumbs = cx
        .debug_bounds("editor-breadcrumbs")
        .expect("merged breadcrumb toolbar should be visible");
    let search = cx
        .debug_bounds("search-panel")
        .expect("search panel should be visible");
    assert_eq!(breadcrumbs.origin.x, search.origin.x);
    assert_eq!(breadcrumbs.size.width, search.size.width);
    assert_eq!(
        breadcrumbs.origin.y + breadcrumbs.size.height,
        search.origin.y
    );
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
    let mut root = WorkbenchView::dev_fixture();
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
    let mut root = WorkbenchView::dev_fixture();
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
            let mut root =
                WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths);
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
    let mut legacy_keybindings = default_keybindings();
    legacy_keybindings.schema_version = 0;
    legacy_keybindings.bindings.retain(|binding| {
        !matches!(
            binding.command.as_str(),
            "file.save" | "project_panel.toggle"
        )
    });
    fs::write(
        paths.keybindings_file(),
        toml::to_string_pretty(&legacy_keybindings).unwrap(),
    )
    .unwrap();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|root_cx| {
            let mut root =
                WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths);
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
        replace_editor_value(input, "saved text", window, input_cx);
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
        let command = root
            .runtime_command_for_dispatch(&Keystroke::parse("ctrl-s").unwrap())
            .unwrap();
        root.run_command(command).unwrap();
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
        replace_editor_value(input, "captured edit", window, input_cx);
    });
    cx.run_until_parked();
    root.update_in(cx, |root, window, root_cx| {
        root.save_active_document(window, root_cx);
    });
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "newer edit", window, input_cx);
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
        replace_editor_value(input, "unsaved after failure", window, input_cx);
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
            let mut root =
                WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths);
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
        replace_editor_value(input, "memory text", window, input_cx);
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
        replace_editor_value(input, "second memory text", window, input_cx);
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
        replace_editor_value(input, "recreated text", window, input_cx);
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
        replace_editor_value(input, "dirty refresh text", window, input_cx);
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
        replace_editor_value(input, "first delayed edit", window, input_cx);
        replace_editor_value(input, "latest delayed edit", window, input_cx);
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
        replace_editor_value(input, "captured overlapping save", window, input_cx);
    });
    cx.run_until_parked();
    root.update_in(cx, |root, window, root_cx| {
        root.save_active_document(window, root_cx);
    });
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "latest overlapping autosave", window, input_cx);
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
        replace_editor_value(input, "focus change edit", window, input_cx);
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert!(document.read(app).model().is_dirty());
        assert_eq!(
            root.read(app).editor_autosave(),
            yttt::config::settings::EditorAutosave::OnFocusChange
        );
    });
    let notes_document_id = cx.read(|app| document.read(app).model().document_id().clone());

    let project_id = cx.read(|app| document.read(app).model().document_id().project_id.clone());
    fs::write(project_dir.join("other.txt"), "other").unwrap();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::ProjectPanelRefresh).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
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
        assert!(tree.activate_path(Path::new("other.txt"), tree_cx));
    });
    cx.run_until_parked();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let other_document_id = DocumentId {
        project_id: project_id.clone(),
        canonical_path: fs::canonicalize(project_dir.join("other.txt")).unwrap(),
    };
    root.update(cx, |root, cx| {
        assert!(
            root.select_work_item(WorkItemId::File(notes_document_id.clone()))
                .unwrap()
        );
        assert!(
            root.select_work_item(WorkItemId::File(other_document_id.clone()))
                .unwrap()
        );
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
        replace_editor_value(input, "manual only edit", window, input_cx);
    });
    cx.run_until_parked();
    let notes_document_id = cx.read(|app| document.read(app).model().document_id().clone());

    let project_id = cx.read(|app| document.read(app).model().document_id().project_id.clone());
    fs::write(project_dir.join("other.txt"), "other").unwrap();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::ProjectPanelRefresh).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
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
        assert!(tree.activate_path(Path::new("other.txt"), tree_cx));
    });
    cx.run_until_parked();
    cx.refresh().unwrap();
    cx.run_until_parked();
    let other_document_id = DocumentId {
        project_id: project_id.clone(),
        canonical_path: fs::canonicalize(project_dir.join("other.txt")).unwrap(),
    };
    root.update(cx, |root, cx| {
        assert!(
            root.select_work_item(WorkItemId::File(notes_document_id.clone()))
                .unwrap()
        );
        assert!(
            root.select_work_item(WorkItemId::File(other_document_id.clone()))
                .unwrap()
        );
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
fn focusing_a_real_editor_document_does_not_reenter_root_render(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, _project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let other_input = cx.update(|window, app| {
        app.new(|input_cx| gpui_component::input::InputState::new(window, input_cx))
    });
    other_input.update_in(cx, |input, window, input_cx| {
        input.focus(window, input_cx);
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.select_work_item(WorkItemId::File(document_id.clone()))
            .unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    cx.read(|app| {
        let root = root.read(app);
        assert_eq!(
            root.active_work_item(),
            Some(WorkItemId::File(document_id.clone()))
        );
        assert!(root.pending_editor_focus_document_id().is_none());
    });
}

#[gpui::test]
fn editor_display_settings_update_open_documents_without_replacing_state(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let (temp, _project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let document_entity_id = document.entity_id();
    let input = cx.read(|app| document.read(app).input().clone());
    let input_entity_id = input.entity_id();
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "keep this live edit", window, input_cx);
    });
    cx.run_until_parked();

    root.update_in(cx, |root, window, cx| {
        root.set_editor_font_family("JetBrains Mono", window, cx)
            .unwrap();
        root.set_editor_font_size(17.0, window, cx).unwrap();
        root.set_editor_line_height(1.65, window, cx).unwrap();
        root.set_editor_soft_wrap(true, window, cx).unwrap();
        root.set_editor_line_numbers(false, window, cx).unwrap();
    });

    cx.read(|app| {
        let current = root
            .read(app)
            .project_editor_runtime()
            .document(&document_id)
            .unwrap();
        assert_eq!(current.entity_id(), document_entity_id);
        let document = current.read(app);
        assert_eq!(document.input().entity_id(), input_entity_id);
        assert_eq!(document.model().value(), "keep this live edit");
        assert_eq!(document.model().saved_value(), "old");
        assert!(document.model().is_dirty());
        assert_eq!(document.appearance().font_family, "JetBrains Mono");
        assert_eq!(document.appearance().font_size, 17.0);
        assert_eq!(document.appearance().line_height, 1.65);
        assert!(document.appearance().soft_wrap);
        assert!(!document.appearance().line_numbers);
    });

    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let loaded = yttt::config::settings::load_or_create_settings(&paths).unwrap();
    assert_eq!(loaded.settings.editor.font_family, "JetBrains Mono");
    assert_eq!(loaded.settings.editor.font_size, 17.0);
    assert_eq!(loaded.settings.editor.line_height, 1.65);
    assert!(loaded.settings.editor.soft_wrap);
    assert!(!loaded.settings.editor.line_numbers);
}

#[gpui::test]
fn editor_tab_size_applies_only_to_files_opened_after_the_setting_changes(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let project_id = cx.read(|app| document.read(app).model().document_id().project_id.clone());
    fs::write(project_dir.join("next.txt"), "next").unwrap();

    root.update(cx, |root, cx| {
        root.set_editor_tab_size(2).unwrap();
        root.run_command(CommandId::ProjectPanelRefresh).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    cx.read(|app| {
        assert_eq!(document.read(app).model().editor().config().tab_size(), 4);
    });
    let tree = cx
        .read(|app| {
            root.read(app)
                .project_editor_runtime()
                .tree(&project_id)
                .cloned()
        })
        .unwrap();
    tree.update(cx, |tree, tree_cx| {
        assert!(tree.activate_path(Path::new("next.txt"), tree_cx));
    });
    cx.run_until_parked();

    let next_id = DocumentId {
        project_id,
        canonical_path: fs::canonicalize(project_dir.join("next.txt")).unwrap(),
    };
    cx.read(|app| {
        let next = root
            .read(app)
            .project_editor_runtime()
            .document(&next_id)
            .unwrap()
            .read(app);
        assert_eq!(next.model().editor().config().tab_size(), 2);
    });
}

#[gpui::test]
fn showing_hidden_files_refreshes_existing_project_trees(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);
    let project_id = cx.read(|app| document.read(app).model().document_id().project_id.clone());
    let tree = cx
        .read(|app| {
            root.read(app)
                .project_editor_runtime()
                .tree(&project_id)
                .cloned()
        })
        .unwrap();
    fs::create_dir(project_dir.join("src")).unwrap();
    fs::write(project_dir.join("src/.secret"), "hidden").unwrap();
    root.update(cx, |root, cx| {
        root.run_command(CommandId::ProjectPanelRefresh).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();
    tree.update(cx, |tree, tree_cx| {
        assert!(tree.activate_path(Path::new("src"), tree_cx));
    });
    cx.run_until_parked();
    cx.refresh().unwrap();
    cx.read(|app| {
        assert!(
            tree.read(app)
                .snapshot()
                .row_for_path(Path::new("src/.secret"))
                .is_none()
        );
    });

    root.update(cx, |root, cx| {
        root.set_project_panel_show_hidden(true).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    cx.refresh().unwrap();

    cx.read(|app| {
        assert!(
            tree.read(app)
                .snapshot()
                .row_for_path(Path::new("src/.secret"))
                .is_some()
        );
    });
}

#[gpui::test]
fn disabling_autosave_cancels_pending_delayed_tasks(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (temp, _project_dir, root, document, cx) =
        project_file_autosave_fixture(cx, "after_delay", 10_000);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "pending delayed edit", window, input_cx);
    });
    cx.run_until_parked();
    cx.read(|app| {
        assert!(
            root.read(app)
                .project_editor_runtime()
                .autosave_task_is_scheduled(&document_id)
        );
    });

    root.update(cx, |root, _cx| {
        root.set_editor_autosave_delay_ms(250).unwrap();
        root.set_editor_autosave(EditorAutosave::Off).unwrap();
    });

    cx.read(|app| {
        assert_eq!(root.read(app).editor_autosave(), EditorAutosave::Off);
        assert!(
            !root
                .read(app)
                .project_editor_runtime()
                .autosave_task_is_scheduled(&document_id)
        );
    });
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let loaded = yttt::config::settings::load_or_create_settings(&paths).unwrap();
    assert_eq!(loaded.settings.editor.autosave, EditorAutosave::Off);
    assert_eq!(loaded.settings.editor.autosave_delay_ms, 250);
}

#[test]
fn project_panel_settings_update_selected_and_future_sessions_only() {
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/settings-panel-first"), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(PathBuf::from("/tmp/settings-panel-second"), sample_layout())
        .unwrap();
    workspace.select_project(&first).unwrap();
    let mut root =
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths.clone());

    root.set_project_panel_default_open(false).unwrap();
    root.set_project_panel_width(360.0).unwrap();
    root.set_project_sidebar_width(250.0).unwrap();

    let first_session = root
        .project_editor_runtime()
        .workspace()
        .session(&first)
        .unwrap();
    let second_session = root
        .project_editor_runtime()
        .workspace()
        .session(&second)
        .unwrap();
    assert!(first_session.project_panel_visible());
    assert_eq!(first_session.project_panel_width(), 360.0);
    assert!(second_session.project_panel_visible());
    assert_eq!(second_session.project_panel_width(), 280.0);
    assert_eq!(root.project_sidebar_width(), 250.0);

    let mut future_workspace = Workspace::new();
    let future = future_workspace
        .open_project(PathBuf::from("/tmp/settings-panel-future"), sample_layout())
        .unwrap();
    let future_root =
        WorkbenchView::with_workspace_for_test_and_config_paths(future_workspace, paths.clone());
    let future_session = future_root
        .project_editor_runtime()
        .workspace()
        .session(&future)
        .unwrap();
    assert!(!future_session.project_panel_visible());
    assert_eq!(future_session.project_panel_width(), 360.0);
    assert_eq!(future_root.project_sidebar_width(), 250.0);

    let loaded = yttt::config::settings::load_or_create_settings(&paths).unwrap();
    assert!(!loaded.settings.project_panel.default_open);
    assert_eq!(loaded.settings.project_panel.width, 360.0);
    assert_eq!(loaded.settings.project_panel.project_sidebar_width, 250.0);
}

#[gpui::test]
fn closing_dirty_file_requires_a_decision(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, _project_dir, root, document, cx) = project_file_terminal_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "unsaved close edit", window, input_cx);
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
fn cancelling_bulk_close_keeps_clean_and_dirty_targets_open(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, _project_dir, root, document, cx) = project_file_terminal_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let anchor = WorkItemId::File(document_id.clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "keep this dirty tab", window, input_cx);
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.close_work_item_tabs(&anchor, WorkbenchTabCloseScope::All, cx)
            .unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.read(|app| {
        let root = root.read(app);
        assert!(root.has_pending_dirty_close());
        assert_eq!(
            root.visible_dirty_close_actions(),
            vec!["Cancel", "Discard and Continue", "Save All and Continue"]
        );
        assert_eq!(
            root.workspace()
                .project(&document_id.project_id)
                .unwrap()
                .layout
                .tabs
                .len(),
            2
        );
        assert!(
            root.project_editor_runtime()
                .document(&document_id)
                .is_some()
        );
    });

    root.update(cx, |root, cx| {
        root.cancel_pending_dirty_close();
        cx.notify();
    });
    cx.read(|app| {
        let root = root.read(app);
        assert!(!root.has_pending_dirty_close());
        assert_eq!(
            root.workspace()
                .project(&document_id.project_id)
                .unwrap()
                .layout
                .tabs
                .len(),
            2
        );
        assert!(
            root.project_editor_runtime()
                .document(&document_id)
                .is_some()
        );
    });

    root.update(cx, |root, cx| {
        root.close_work_item_tabs(&anchor, WorkbenchTabCloseScope::All, cx)
            .unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    root.update_in(cx, |root, window, cx| {
        root.discard_pending_dirty_close(window, cx);
    });
    cx.read(|app| {
        let root = root.read(app);
        assert!(
            root.workspace()
                .project(&document_id.project_id)
                .unwrap()
                .layout
                .tabs
                .is_empty()
        );
        assert!(
            root.project_editor_runtime()
                .document(&document_id)
                .is_none()
        );
        assert_eq!(root.active_work_item(), None);
    });
}

#[gpui::test]
fn scoped_bulk_tab_close_removes_terminal_and_file_groups(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, _project_dir, root, document, cx) = project_file_terminal_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let anchor = WorkItemId::File(document_id.clone());

    root.update_in(cx, |_root, window, cx| {
        window.dispatch_action(Box::new(TabCloseAllTerminals), cx);
    });
    cx.refresh().unwrap();
    cx.read(|app| {
        let root = root.read(app);
        let project = root.workspace().project(&document_id.project_id).unwrap();
        assert!(project.layout.tabs.is_empty());
        assert!(
            root.project_editor_runtime()
                .document(&document_id)
                .is_some()
        );
        assert_eq!(root.active_work_item(), Some(anchor.clone()));
    });

    root.update(cx, |root, cx| {
        root.close_work_item_tabs(&anchor, WorkbenchTabCloseScope::Files, cx)
            .unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.read(|app| {
        let root = root.read(app);
        assert!(
            root.project_editor_runtime()
                .document(&document_id)
                .is_none()
        );
        assert_eq!(root.active_work_item(), None);
    });
}

#[gpui::test]
fn saving_a_dirty_file_continues_the_pending_close(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, project_dir, root, document, cx) = project_file_terminal_fixture(cx, "off", 50);
    let document_id = cx.read(|app| document.read(app).model().document_id().clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "save before close", window, input_cx);
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
        replace_editor_value(input, "cannot save this edit", window, input_cx);
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
    let (_temp, _project_dir, root, document, cx) = project_file_terminal_fixture(cx, "off", 50);
    let project_id = cx.read(|app| document.read(app).model().document_id().project_id.clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "unsaved project edit", window, input_cx);
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
    let (_temp, project_dir, root, document, cx) = project_file_terminal_fixture(cx, "off", 50);
    let project_id = cx.read(|app| document.read(app).model().document_id().project_id.clone());
    let input = cx.read(|app| document.read(app).input().clone());
    input.update_in(cx, |input, window, input_cx| {
        replace_editor_value(input, "save project before close", window, input_cx);
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
        replace_editor_value(input, "unsaved window edit", window, input_cx);
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
    let mut root = WorkbenchView::with_config_paths(paths.clone());
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
    let mut root = WorkbenchView::with_config_paths(paths);

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
    let mut root = WorkbenchView::with_config_paths(paths);

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
    let mut root = WorkbenchView::with_config_paths(paths);
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
    let mut root = WorkbenchView::with_config_paths(paths);

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
fn layout_editor_popup_uses_chinese_text_and_error_prefixes() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.general.language = LanguageSetting::Chinese;
    settings.general.onboarding_completed = true;
    save_settings(&paths, &settings).unwrap();
    let mut root = WorkbenchView::with_config_paths(paths);

    root.run_command(CommandId::LayoutDefaultEdit).unwrap();

    let config = root.visible_layout_toml_editor_config().unwrap();
    assert_eq!(config.title(), "编辑默认布局");
    assert_eq!(config.placeholder(), "编辑布局 TOML…");
    let ui_text = UiText::new(Locale::Chinese);
    assert_eq!(ui_text.get(yttt::ui::i18n::UiTextKey::Cancel), "取消");
    assert_eq!(ui_text.get(yttt::ui::i18n::UiTextKey::SettingsSave), "保存");

    root.set_layout_toml_editor_value("[project\n");
    root.save_layout_toml_editor().unwrap();

    assert!(
        root.visible_layout_toml_editor_error()
            .unwrap()
            .starts_with("解析布局 TOML 失败:")
    );
}

#[gpui::test]
fn layout_default_editor_uses_editor_settings_and_updates_appearance(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    fs::write(
        paths.settings_file(),
        r#"
[general]
language = "en"
onboarding_completed = true

[editor]
font_family = "Menlo"
font_size = 18.0
line_height = 1.6
tab_size = 2
soft_wrap = true
line_numbers = false
"#,
    )
    .unwrap();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths(paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    root.update(cx, |root, cx| {
        root.run_command(CommandId::LayoutDefaultEdit).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    let original_value = cx.read(|app| {
        let root = root.read(app);
        let config = root.visible_layout_toml_editor_config().unwrap();
        assert_eq!(config.tab_size(), 2);
        assert!(config.soft_wrap());
        assert!(!config.line_number());
        assert_eq!(
            root.visible_layout_toml_editor_appearance().unwrap(),
            &EditorAppearance {
                font_family: "Menlo".to_string(),
                font_size: 18.0,
                line_height: 1.6,
                soft_wrap: true,
                line_numbers: false,
            }
        );
        root.layout_toml_editor_value().unwrap().to_string()
    });

    root.update_in(cx, |root, window, cx| {
        root.set_editor_font_family("JetBrains Mono", window, cx)
            .unwrap();
        root.set_editor_font_size(17.0, window, cx).unwrap();
        root.set_editor_line_height(1.7, window, cx).unwrap();
        root.set_editor_soft_wrap(false, window, cx).unwrap();
        root.set_editor_line_numbers(true, window, cx).unwrap();
    });

    cx.read(|app| {
        let root = root.read(app);
        assert_eq!(
            root.visible_layout_toml_editor_appearance().unwrap(),
            &EditorAppearance {
                font_family: "JetBrains Mono".to_string(),
                font_size: 17.0,
                line_height: 1.7,
                soft_wrap: false,
                line_numbers: true,
            }
        );
        assert_eq!(
            root.layout_toml_editor_value(),
            Some(original_value.as_str())
        );
    });
}

#[test]
fn root_view_persists_editor_language_settings() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

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
    let mut root = WorkbenchView::with_config_paths(paths);
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
    let paths = english_test_config_paths(&temp);
    let mut root = WorkbenchView::with_config_paths(paths);
    root.run_command(CommandId::LayoutDefaultEdit).unwrap();

    root.set_layout_toml_editor_value("[project\n");
    root.save_layout_toml_editor().unwrap();

    assert!(root.layout_toml_editor_is_open());
    assert!(
        root.visible_layout_toml_editor_error()
            .unwrap()
            .contains("Failed to parse layout TOML")
    );
}

#[test]
fn root_view_layout_default_editor_records_parse_diagnostic() {
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let mut root = WorkbenchView::with_config_paths(paths);
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
            .contains("Failed to parse layout TOML")
    );
}

#[test]
fn root_view_layout_default_editor_clears_diagnostics_after_valid_save() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths);
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
    let mut root = WorkbenchView::with_config_paths(paths.clone());
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
    let mut root = WorkbenchView::with_config_paths(paths);
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
    let mut root = WorkbenchView::with_config_paths(paths);
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
    let mut root = WorkbenchView::with_config_paths(paths.clone());

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
    let mut root = WorkbenchView::with_config_paths(paths.clone());

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
    let mut root = WorkbenchView::with_config_paths(paths);

    root.run_command(CommandId::SettingsOpen).unwrap();

    assert!(root.settings_is_open());
    assert_eq!(
        root.visible_settings_group_titles(),
        vec![
            "General",
            "Appearance",
            "Languages",
            "Editor",
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
    let mut root = WorkbenchView::with_config_paths(paths);
    root.open_settings();

    root.set_settings_search_query("shell");

    assert_eq!(root.visible_settings_group_titles(), vec!["Terminal"]);
    assert_eq!(root.selected_settings_group_title(), Some("Terminal"));
}

#[test]
fn root_view_settings_can_select_and_close_group() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths);
    root.open_settings();

    root.select_settings_group("terminal").unwrap();
    root.close_settings();

    assert!(!root.settings_is_open());
    assert_eq!(root.selected_settings_group_title(), Some("Terminal"));
}

#[test]
fn new_tab_command_settings_edit_and_persist() {
    let (temp, mut root) = english_test_root();

    assert!(!root.new_tab_command_picker_enabled());
    assert!(!root.add_new_tab_command("nvim").unwrap());
    assert!(!root.add_new_tab_command("   ").unwrap());
    assert!(root.add_new_tab_command("nvim .").unwrap());
    assert!(root.remove_new_tab_command(0).unwrap());
    root.set_new_tab_command_picker_enabled(true).unwrap();

    assert_eq!(root.new_tab_commands(), &["nvim", "codex", "nvim ."]);
    let loaded =
        load_or_create_settings(&AppConfigPaths::from_config_dir(temp.path().join("config")))
            .unwrap();
    assert!(loaded.settings.general.new_tab_command_picker_enabled);
    assert_eq!(
        loaded.settings.general.new_tab_commands,
        vec!["nvim", "codex", "nvim ."]
    );
}

#[test]
fn new_tab_toolbar_defaults_to_creating_a_shell_tab() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let tab_count = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .tabs
        .len();

    root.new_tab_from_toolbar().unwrap();

    assert!(root.active_palette().is_none());
    let project = root.workspace().project(&project_id).unwrap();
    assert_eq!(project.layout.tabs.len(), tab_count + 1);
    let tab = project.layout.tab(&project.selected_tab_id).unwrap();
    let LayoutNode::Pane(pane) = &tab.layout else {
        panic!("new shell tab should contain one pane");
    };
    assert!(pane.command.is_empty());
}

#[gpui::test]
fn general_settings_render_and_toggle_new_tab_command_picker(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let mut settings = load_or_create_settings(&paths).unwrap().settings;
    settings.general.onboarding_completed = true;
    save_settings(&paths, &settings).unwrap();
    let view_paths = paths.clone();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths(view_paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.open_settings();
        cx.notify();
    });
    cx.refresh().unwrap();

    let picker_row = cx
        .debug_bounds("settings-new-tab-command-picker-row")
        .expect("general settings should render the new tab command picker row");
    assert!(cx.debug_bounds("settings-new-tab-commands-row").is_some());
    let toggle = cx
        .debug_bounds("settings-new-tab-command-picker")
        .expect("new tab command picker setting should expose a switch");
    cx.simulate_click(toggle.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: picker_row.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-160.0))),
        ..Default::default()
    });
    cx.refresh().unwrap();
    assert!(cx.debug_bounds("settings-add-new-tab-command").is_some());

    cx.read(|app| assert!(root.read(app).new_tab_command_picker_enabled()));
    assert!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .new_tab_command_picker_enabled
    );
}

#[gpui::test]
fn enabled_new_tab_toolbar_click_runs_the_selected_configured_command(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let mut settings = load_or_create_settings(&paths).unwrap().settings;
    settings.general.new_tab_command_picker_enabled = true;
    settings.general.new_tab_commands = vec![
        "lazygit".to_string(),
        "nvim .".to_string(),
        "codex".to_string(),
    ];
    save_settings(&paths, &settings).unwrap();
    let workspace = workspace_with_sample_project();
    let project_id = workspace.selected_project_id().unwrap().clone();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root =
            cx.new(|_| WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    cx.refresh().unwrap();

    let new_tab = cx
        .debug_bounds("tab-new")
        .expect("workbench should render the new tab button");
    cx.simulate_click(new_tab.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.refresh().unwrap();

    cx.read(|app| {
        let root = root.read(app);
        assert_eq!(
            root.active_palette().map(|palette| palette.kind),
            Some(PaletteKind::NewTabCommand)
        );
        assert_eq!(
            root.visible_palette_titles(),
            vec!["lazygit", "nvim .", "codex"]
        );
    });
    assert!(
        cx.debug_bounds("palette-list").is_some(),
        "clicking new tab should render the command picker"
    );

    root.update(cx, |root, _cx| {
        root.set_palette_query("nvim .");
        root.confirm_palette_selection().unwrap();
    });
    cx.read(|app| {
        let root = root.read(app);
        assert!(root.active_palette().is_none());
        let project = root.workspace().project(&project_id).unwrap();
        let tab = project.layout.tab(&project.selected_tab_id).unwrap();
        let LayoutNode::Pane(pane) = &tab.layout else {
            panic!("selected command should create a single-pane tab");
        };
        assert_eq!(pane.command, "nvim .");
        assert_eq!(pane.execution_mode, TerminalExecutionMode::Shell);
    });
}

#[gpui::test]
fn appearance_settings_group_renders_window_and_theme_controls(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::dev_fixture());
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.open_settings();
        root.select_settings_group("appearance").unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    assert!(
        cx.debug_bounds("settings-window-effect-row").is_some(),
        "Appearance settings should expose the window effect selector"
    );
    assert!(
        cx.debug_bounds("settings-window-opacity-row").is_some(),
        "Appearance settings should expose the shared window opacity control"
    );
    assert!(
        cx.debug_bounds("settings-ui-font-family-row").is_some(),
        "Appearance settings should expose the UI font selector"
    );
    assert!(
        cx.debug_bounds("settings-ui-font-size-row").is_some(),
        "Appearance settings should expose the UI font size control"
    );
    assert!(
        cx.debug_bounds("settings-ui-line-height-row").is_some(),
        "Appearance settings should expose the UI line-height control"
    );

    assert!(
        cx.debug_bounds("settings-import-zed-themes").is_some(),
        "Appearance settings should expose the Zed theme import action"
    );
}

#[gpui::test]
fn editor_settings_group_renders_all_effective_controls(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::dev_fixture());
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.open_settings();
        root.select_settings_group("editor").unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();

    for selector in [
        "settings-editor-font-family-row",
        "settings-editor-font-size-row",
        "settings-editor-line-height-row",
        "settings-editor-tab-size-row",
        "settings-editor-soft-wrap-row",
        "settings-editor-line-numbers-row",
        "settings-editor-autosave-row",
        "settings-editor-autosave-delay-row",
        "settings-project-panel-default-open-row",
        "settings-project-panel-show-hidden-row",
        "settings-project-panel-width-row",
        "settings-project-sidebar-width-row",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
    }
    let first_before = cx
        .debug_bounds("settings-editor-font-family-row")
        .expect("first editor settings row");
    cx.simulate_event(gpui::ScrollWheelEvent {
        position: first_before.origin + gpui::point(gpui::px(24.0), gpui::px(24.0)),
        delta: gpui::ScrollDelta::Pixels(gpui::point(gpui::px(0.0), gpui::px(-240.0))),
        ..Default::default()
    });
    cx.refresh().unwrap();
    let first_after = cx
        .debug_bounds("settings-editor-font-family-row")
        .expect("first editor settings row after scrolling");
    assert_eq!(
        first_after.origin.y,
        first_before.origin.y - gpui::px(240.0),
        "settings content should move by the wheel delta"
    );
}
#[gpui::test]
fn terminal_settings_group_renders_protocol_and_interaction_controls(
    cx: &mut gpui::TestAppContext,
) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::dev_fixture());
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.open_settings();
        root.select_settings_group("terminal").unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();

    for selector in [
        "settings-terminal-scrollbar-row",
        "settings-terminal-cursor-shape-row",
        "settings-terminal-cursor-blinking-row",
        "settings-terminal-hide-mouse-when-typing-row",
        "settings-terminal-copy-on-select-row",
        "settings-terminal-osc52-policy-row",
        "settings-terminal-kitty-keyboard-row",
    ] {
        assert!(cx.debug_bounds(selector).is_some(), "missing {selector}");
    }
}

#[test]
fn root_view_toggles_system_notifications() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

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

    let reloaded = WorkbenchView::with_config_paths(paths);
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
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    root.set_language(LanguageSetting::Chinese).unwrap();

    assert_eq!(
        root.visible_empty_workspace_actions(),
        vec!["打开目录", "打开最近项目", "命令面板"]
    );

    let reloaded = WorkbenchView::with_config_paths(paths);
    assert_eq!(
        reloaded.visible_empty_workspace_actions(),
        vec!["打开目录", "打开最近项目", "命令面板"]
    );
}

#[test]
fn root_view_status_notifications_use_selected_language() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths);

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
    let mut root = WorkbenchView::with_config_paths(paths);
    root.open_settings();

    root.set_language(LanguageSetting::Chinese).unwrap();

    assert_eq!(
        root.visible_settings_group_titles(),
        vec![
            "通用",
            "外观",
            "语言",
            "编辑器",
            "终端",
            "默认布局",
            "快捷键"
        ]
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
    let mut root = WorkbenchView::with_config_paths(paths);

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
fn root_view_command_palette_can_request_create_project() {
    let (_temp, mut root) = english_test_root();

    root.open_palette(PaletteKind::Command);
    root.set_palette_query("Create Project");
    root.confirm_palette_selection().unwrap();

    assert!(root.active_palette().is_none());
    assert!(root.take_pending_create_project_request());
    assert_eq!(root.visible_error_message(), None);
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
    let mut root = WorkbenchView::dev_fixture();

    root.close_project_tab("agent").unwrap();

    assert_eq!(visible_tab_titles(root.workspace()), vec!["Dev"]);
}

#[test]
fn root_view_custom_terminal_shell_setting_persists() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    assert!(root.add_custom_terminal_shell("/opt/tools/fish").unwrap());

    let loaded = load_or_create_settings(&paths).unwrap();
    assert_eq!(loaded.settings.terminal.shell, "/opt/tools/fish");
    assert_eq!(
        loaded.settings.terminal.custom_shells,
        vec!["/opt/tools/fish"]
    );
}

#[test]
fn root_view_ui_font_settings_persist_and_family_can_reset() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    root.set_ui_font_family("  Menlo  ").unwrap();
    root.set_ui_font_size(20.0).unwrap();
    root.set_ui_line_height(1.75).unwrap();
    let loaded = load_or_create_settings(&paths).unwrap();
    assert_eq!(loaded.settings.general.ui_font_family, "Menlo");
    assert_eq!(loaded.settings.general.ui_font_size, 20.0);
    assert_eq!(loaded.settings.general.ui_line_height, 1.75);

    root.set_ui_font_family("").unwrap();
    assert_eq!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .general
            .ui_font_family,
        ""
    );
}

#[gpui::test]
fn window_background_settings_persist_from_live_window(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let paths_for_window = paths.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths(paths_for_window));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    root.update_in(cx, |root, window, _cx| {
        root.set_window_opacity(0.42).unwrap();
        root.set_window_background_effect(WindowBackgroundEffect::Transparent, window)
            .unwrap();
    });

    let loaded = load_or_create_settings(&paths).unwrap();
    assert_eq!(
        loaded.settings.window.effect,
        WindowBackgroundEffect::Transparent
    );
    assert_eq!(loaded.settings.window.opacity, 0.42);
}

#[gpui::test]
fn ui_font_size_updates_window_scale(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths(paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    root.update_in(cx, |root, _window, cx| {
        root.set_ui_font_size(20.0).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();

    root.update_in(cx, |_root, window, _cx| {
        assert_eq!(window.rem_size(), gpui::px(20.0));
    });
}

#[test]
fn root_view_icon_theme_setting_persists_and_can_reset() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    root.set_icon_theme_name(Some("Fixture dark")).unwrap();
    assert_eq!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .theme
            .icon_theme
            .as_deref(),
        Some("Fixture dark")
    );

    root.set_icon_theme_name(None).unwrap();
    assert_eq!(
        load_or_create_settings(&paths)
            .unwrap()
            .settings
            .theme
            .icon_theme,
        None
    );
}

#[test]
fn root_view_terminal_shell_setting_changes_new_shell_tabs() {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("shell-settings-project");
    fs::create_dir(&project_dir).unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths);
    root.open_project_path(&project_dir).unwrap();

    root.set_terminal_shell("/bin/bash").unwrap();
    root.run_command(CommandId::TabNew).unwrap();

    let project_id = root.workspace().selected_project_id().unwrap();
    let project = root.workspace().project(project_id).unwrap();
    let tab = project.layout.tab(&project.selected_tab_id).unwrap();
    let pane = tab.layout.find_pane("shell").unwrap();
    assert_eq!(pane.command, "");
    assert_eq!(pane.execution_mode, TerminalExecutionMode::Shell);
    assert_eq!(
        root.visible_terminal_pane_contexts()
            .into_iter()
            .find(|context| context.pane.id == "shell")
            .unwrap()
            .shell,
        "/bin/bash"
    );
}

#[test]
fn root_view_terminal_display_settings_persist() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    root.set_terminal_font_family("JetBrains Mono").unwrap();
    root.set_terminal_font_size(14.5).unwrap();
    root.set_terminal_line_height(1.2).unwrap();
    root.set_terminal_padding(8.0).unwrap();
    root.set_terminal_scrollback(20000).unwrap();
    root.set_terminal_show_scrollbar(false).unwrap();
    root.set_terminal_cursor_shape(TerminalCursorShape::Underline)
        .unwrap();
    root.set_terminal_cursor_blinking(true).unwrap();
    root.set_terminal_hide_mouse_when_typing(true).unwrap();
    root.set_terminal_copy_on_select(true).unwrap();
    root.set_terminal_osc52_policy(TerminalOsc52Policy::ReadWrite)
        .unwrap();
    root.set_terminal_kitty_keyboard(true).unwrap();

    let runtime = &root.theme_runtime().terminal_settings;
    assert_eq!(runtime.font_family, "JetBrains Mono");
    assert_eq!(runtime.font_size, 14.5);
    assert_eq!(runtime.line_height, 1.2);
    assert_eq!(runtime.padding, 8.0);
    assert_eq!(runtime.scrollback, 20000);
    assert!(!runtime.show_scrollbar);
    assert_eq!(runtime.cursor_shape, TerminalCursorShape::Underline);
    assert!(runtime.cursor_blinking);
    assert!(runtime.hide_mouse_when_typing);
    assert!(runtime.copy_on_select);
    assert_eq!(runtime.osc52_policy, TerminalOsc52Policy::ReadWrite);
    assert!(runtime.kitty_keyboard);

    let reloaded = WorkbenchView::with_config_paths(paths);
    let terminal = &reloaded.theme_runtime().terminal_settings;
    assert_eq!(terminal.font_family, "JetBrains Mono");
    assert_eq!(terminal.font_size, 14.5);
    assert_eq!(terminal.line_height, 1.2);
    assert_eq!(terminal.padding, 8.0);
    assert_eq!(terminal.scrollback, 20000);
    assert!(!terminal.show_scrollbar);
    assert_eq!(terminal.cursor_shape, TerminalCursorShape::Underline);
    assert!(terminal.cursor_blinking);
    assert!(terminal.hide_mouse_when_typing);
    assert!(terminal.copy_on_select);
    assert_eq!(terminal.osc52_policy, TerminalOsc52Policy::ReadWrite);
    assert!(terminal.kitty_keyboard);
}

#[test]
fn root_view_does_not_auto_focus_workspace_while_settings_is_open() {
    let mut root = WorkbenchView::new();

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
    let mut root = WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths);

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
    let mut root = WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths);

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
    let mut root = WorkbenchView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let initial_tab_count = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .tabs
        .len();

    root.open_settings();

    assert!(
        root.runtime_command_for_dispatch(&Keystroke::parse("cmd-t").unwrap())
            .is_none()
    );
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
    let mut root = WorkbenchView::dev_fixture();
    root.open_layout_toml_editor().unwrap();

    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Dialog);
    assert!(
        root.runtime_command_for_dispatch(&Keystroke::parse("cmd-s").unwrap())
            .is_none()
    );
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
    let mut root = WorkbenchView::dev_fixture();

    root.handle_project_tab_click("dev", 2).unwrap();

    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Dialog);
    assert!(!root.terminal_input_allowed());
}

#[test]
fn root_view_does_not_consume_terminal_focus_while_overlay_is_open() {
    let mut root = WorkbenchView::dev_fixture();

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
    let mut root = WorkbenchView::new();

    root.open_palette(PaletteKind::Command);

    assert!(!root.should_use_palette_text_fallback(true));
    assert!(root.should_use_palette_text_fallback(false));
}

#[test]
fn root_view_notification_settings_can_be_disabled_again() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

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

    let reloaded = WorkbenchView::with_config_paths(paths);
    assert!(!reloaded.system_notifications_enabled());
}

#[test]
fn root_view_exposes_keybinding_warning_lines() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    fs::create_dir_all(paths.config_dir()).unwrap();
    let mut settings = AppSettings::default();
    settings.general.language = LanguageSetting::Chinese;
    settings.general.onboarding_completed = true;
    save_settings(&paths, &settings).unwrap();
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

    let root = WorkbenchView::with_config_paths(paths);

    assert_eq!(
        root.visible_keybinding_warning_lines(),
        vec!["快捷键冲突: cmd-p", "无效的命令 ID: missing.command"]
    );
}

#[test]
fn root_view_keybindings_editor_updates_and_persists_command_keys() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    root.set_keybinding_command_keys(CommandId::TabPalette, vec!["cmd-l".to_string()])
        .unwrap();

    let row = root
        .visible_keybinding_rows()
        .into_iter()
        .find(|row| row.command == CommandId::TabPalette)
        .unwrap();
    assert_eq!(row.keys, vec!["cmd-l".to_string()]);

    let reloaded = WorkbenchView::with_config_paths(paths);
    let row = reloaded
        .visible_keybinding_rows()
        .into_iter()
        .find(|row| row.command == CommandId::TabPalette)
        .unwrap();
    assert_eq!(row.keys, vec!["cmd-l".to_string()]);
}

#[test]
fn create_project_command_is_configurable_in_keybinding_settings() {
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    let initial = root
        .visible_keybinding_rows()
        .into_iter()
        .find(|row| row.command == CommandId::ProjectCreate)
        .expect("create project should be listed in keybinding settings");
    assert_eq!(initial.title, "Create Project");
    assert!(initial.keys.is_empty());

    root.set_keybinding_command_keys(CommandId::ProjectCreate, vec!["cmd-alt-n".to_string()])
        .unwrap();
    assert_eq!(
        root.runtime_command_for_keystroke(&Keystroke::parse("cmd-alt-n").unwrap()),
        Some(CommandId::ProjectCreate)
    );

    let reloaded = WorkbenchView::with_config_paths(paths);
    let persisted = reloaded
        .visible_keybinding_rows()
        .into_iter()
        .find(|row| row.command == CommandId::ProjectCreate)
        .unwrap();
    assert_eq!(persisted.keys, vec!["cmd-alt-n".to_string()]);
}

#[test]
fn root_view_runtime_keybindings_follow_edited_settings() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths);

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
    let mut root = WorkbenchView::with_config_paths(paths);

    let error = root
        .set_keybinding_command_keys(CommandId::TabPalette, vec!["cmd-p".to_string()])
        .unwrap_err();

    assert!(error.to_string().contains("conflicting keybindings"));
}

#[test]
fn root_view_keybinding_edit_dialog_updates_command_keys() {
    let temp = tempdir().unwrap();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut root = WorkbenchView::with_config_paths(paths.clone());

    root.open_keybinding_edit_dialog(CommandId::TabPalette)
        .unwrap();

    assert_eq!(
        root.pending_keybinding_edit_keys(),
        Some(vec!["cmd-j".to_string(), "ctrl-j".to_string()])
    );
    assert_eq!(
        root.foreground_input_owner_kind(),
        InputOwnerKind::KeybindingRecorder
    );

    assert!(root.record_keybinding_edit_keystroke(&Keystroke::parse("cmd-l").unwrap()));
    assert!(root.record_keybinding_edit_keystroke(&Keystroke::parse("ctrl-l").unwrap()));
    root.confirm_keybinding_edit_dialog().unwrap();

    assert!(root.pending_keybinding_edit_keys().is_none());
    assert_eq!(
        root.visible_keybinding_rows()
            .into_iter()
            .find(|row| row.command == CommandId::TabPalette)
            .unwrap()
            .keys,
        vec!["cmd-l".to_string(), "ctrl-l".to_string()]
    );

    let reloaded = WorkbenchView::with_config_paths(paths);
    assert_eq!(
        reloaded
            .visible_keybinding_rows()
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
fn visible_project_items_mark_selection_and_distinct_initials() {
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/one"), sample_layout())
        .unwrap();
    let mut second_layout = sample_layout();
    second_layout.project.name = "backend".to_string();
    let second = workspace
        .open_project(PathBuf::from("/tmp/two"), second_layout)
        .unwrap();

    workspace.select_project(&first).unwrap();

    let items = visible_project_items(&workspace);
    assert_eq!(items.len(), 2);
    assert_eq!(items[0].id, first.as_str());
    assert_eq!(items[0].initial, "Y");
    assert_eq!(items[0].state, SelectableState::Active);
    assert_eq!(items[1].id, second.as_str());
    assert_eq!(items[1].initial, "B");
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
    let mut root = WorkbenchView::dev_fixture();
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
    let root = WorkbenchView::dev_fixture();

    let contexts = root.visible_terminal_pane_contexts();

    assert_eq!(contexts.len(), 2);
    assert!(
        contexts
            .iter()
            .all(|context| context.project_path == PathBuf::from("/tmp/yttt"))
    );
}

#[test]
fn root_view_terminal_pane_contexts_use_tab_cwd() {
    let mut root = WorkbenchView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let mut layout = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .clone();
    layout
        .tabs
        .iter_mut()
        .find(|tab| tab.id == "dev")
        .unwrap()
        .cwd = Some(PathBuf::from("/tmp/yttt/services/api"));
    root.workspace_mut()
        .replace_selected_project_layout(layout)
        .unwrap();

    let contexts = root.visible_terminal_pane_contexts();

    assert_eq!(contexts.len(), 2);
    assert!(
        contexts
            .iter()
            .all(|context| context.project_path == PathBuf::from("/tmp/yttt/services/api"))
    );
}

#[test]
fn root_view_tab_palette_scopes_to_current_project_tabs() {
    let mut root = WorkbenchView::dev_fixture();

    root.open_palette(PaletteKind::Tab);

    assert_eq!(root.visible_palette_titles(), vec!["Dev", "Agent"]);
}

#[test]
fn root_view_pane_palette_scopes_to_current_tab_panes() {
    let mut root = WorkbenchView::dev_fixture();

    root.open_palette(PaletteKind::Pane);

    assert_eq!(root.visible_palette_titles(), vec!["server", "shell"]);
}

#[test]
fn root_view_syncs_palette_query_from_input_value() {
    let mut root = WorkbenchView::dev_fixture();
    root.open_palette(PaletteKind::Tab);
    root.set_palette_query("agent");

    assert!(root.sync_palette_query_from_input_value("dev"));

    assert_eq!(root.visible_palette_titles(), vec!["Dev"]);
}

#[test]
fn root_view_ignores_palette_input_value_without_active_palette() {
    let mut root = WorkbenchView::dev_fixture();

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
    let mut root = WorkbenchView::dev_fixture();

    root.open_palette(PaletteKind::Tab);
    root.set_palette_query("agent");
    root.confirm_palette_selection().unwrap();

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "agent");
}

#[test]
fn root_view_confirming_tab_palette_selection_queues_terminal_focus() {
    let mut root = WorkbenchView::dev_fixture();

    root.open_palette(PaletteKind::Tab);
    root.set_palette_query("agent");
    root.confirm_palette_selection().unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("codex"));
}

#[test]
fn root_view_confirming_pane_palette_selection_focuses_pane() {
    let mut root = WorkbenchView::dev_fixture();

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
    let mut root = WorkbenchView::dev_fixture();

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
fn root_view_project_commands_open_separate_project_palettes() {
    let mut root = WorkbenchView::dev_fixture();
    root.focus_visible_terminal_pane("shell").unwrap();

    let command = root
        .runtime_command_for_dispatch(&Keystroke::parse("cmd-shift-p").unwrap())
        .unwrap();
    assert_eq!(command, CommandId::ProjectOpenedPalette);
    root.run_command(command).unwrap();
    assert_eq!(
        root.active_palette().map(|palette| palette.kind),
        Some(PaletteKind::OpenedProject)
    );
    let items = root.active_palette_items();
    assert!(!items.is_empty());
    assert!(
        items
            .iter()
            .all(|item| item.command == CommandId::ProjectOpenedPalette)
    );

    root.close_palette();
    root.run_command(CommandId::ProjectOpenRecent).unwrap();
    assert_eq!(
        root.active_palette().map(|palette| palette.kind),
        Some(PaletteKind::RecentProject)
    );
}

#[test]
fn root_view_focus_visible_terminal_pane_updates_focused_pane() {
    let mut root = WorkbenchView::dev_fixture();

    root.focus_visible_terminal_pane("shell").unwrap();

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    let tab = project.tab_state("dev").unwrap();
    assert_eq!(tab.focused_pane_id.as_deref(), Some("shell"));
}

#[test]
fn root_view_marks_focused_terminal_pane_context() {
    let mut root = WorkbenchView::dev_fixture();

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
    let mut root = WorkbenchView::dev_fixture();

    root.focus_visible_terminal_pane("shell").unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("shell"));
}

#[test]
fn root_view_leaves_terminal_control_keybindings_for_focused_terminal() {
    let mut root = WorkbenchView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let initial_tab_count = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .tabs
        .len();

    root.focus_visible_terminal_pane("shell").unwrap();

    assert!(
        root.runtime_command_for_dispatch(&Keystroke::parse("ctrl-t").unwrap())
            .is_none()
    );
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
    let mut root = WorkbenchView::dev_fixture();

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
    let mut root = WorkbenchView::dev_fixture();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let initial_tab_count = root
        .workspace()
        .project(&project_id)
        .unwrap()
        .layout
        .tabs
        .len();

    root.focus_visible_terminal_pane("shell").unwrap();

    let command = root
        .runtime_command_for_dispatch(&Keystroke::parse("cmd-t").unwrap())
        .unwrap();
    root.run_command(command).unwrap();
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

#[gpui::test]
fn create_project_action_creates_and_opens_new_directory(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join("new-project");
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.general.onboarding_completed = true;
    save_settings(&paths, &settings).unwrap();

    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths(paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    cx.refresh().unwrap();

    root.update_in(cx, |_root, window, cx| {
        window.dispatch_action(Box::new(CreateProject), cx);
    });
    cx.run_until_parked();
    assert!(cx.did_prompt_for_new_path());

    let selected_path = project_dir.clone();
    cx.simulate_new_path_selection(move |parent| {
        assert!(parent.is_dir());
        Some(selected_path)
    });
    cx.run_until_parked();

    assert!(project_dir.is_dir());
    let canonical_project_dir = project_dir.canonicalize().unwrap();
    root.update(cx, |root, _| {
        let project_id = root.workspace().selected_project_id().unwrap();
        let project = root.workspace().project(project_id).unwrap();
        assert_eq!(project.path, canonical_project_dir);
        assert_eq!(root.visible_error_message(), None);
    });
}

#[gpui::test]
fn focused_terminal_platform_open_shortcut_prompts_immediately(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::dev_fixture());
        register_workbench_keybinding_interceptor(cx, &root);
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        gpui_component::Root::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();

    root.update(cx, |root, cx| {
        root.focus_visible_terminal_pane("shell").unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();

    cx.simulate_keystrokes("cmd-o");
    cx.run_until_parked();

    assert!(
        cx.did_prompt_for_paths(),
        "the open-project action must run during the shortcut dispatch"
    );
    root.update(cx, |root, _| {
        assert!(!root.take_pending_open_project_request());
    });
    cx.simulate_path_prompt_response(|options| {
        assert!(!options.files);
        assert!(options.directories);
        assert!(!options.multiple);
        None
    });
    cx.run_until_parked();
}

#[gpui::test]
fn focused_editor_global_settings_shortcut_opens_settings(cx: &mut gpui::TestAppContext) {
    cx.update(gpui_component::init);
    let (_temp, _project_dir, root, document, cx) = project_file_autosave_fixture(cx, "off", 50);

    document.update_in(cx, |document, window, document_cx| {
        document.focus(window, document_cx);
    });
    cx.refresh().unwrap();
    cx.read(|app| {
        assert_eq!(
            root.read(app).foreground_input_owner_kind(),
            InputOwnerKind::Editor
        );
        assert!(!root.read(app).settings_is_open());
    });

    cx.simulate_keystrokes("cmd-,");
    cx.run_until_parked();

    cx.read(|app| {
        assert!(
            root.read(app).settings_is_open(),
            "the editor must not swallow global shortcuts"
        );
    });
}

#[test]
fn root_view_pane_focus_command_queues_target_terminal_focus() {
    let mut root = WorkbenchView::dev_fixture();

    root.run_command(CommandId::PaneFocusRight).unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("shell"));
}

#[test]
fn root_view_split_command_queues_new_terminal_focus() {
    let mut root = WorkbenchView::dev_fixture();

    root.run_command(CommandId::PaneSplitVertical).unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("pane-1"));
}

#[test]
fn root_view_terminal_exit_closes_exact_split_pane() {
    let mut root = WorkbenchView::dev_fixture();

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
    let mut root = WorkbenchView::dev_fixture();
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
fn root_view_terminal_exit_keeps_project_open_and_allows_new_tab() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/single"), single_tab_layout())
        .unwrap();
    let mut root = WorkbenchView::with_workspace_for_test(workspace);

    root.handle_terminal_pane_exit(TerminalPaneExitedEvent {
        project_id: "/tmp/single".to_string(),
        tab_id: "dev".to_string(),
        pane_id: "shell".to_string(),
        status: ProcessStatus::Exited { code: Some(0) },
        exit_reason: ExitReason::Completed,
        exit_behavior: ProcessExitBehavior::Close,
    })
    .unwrap();

    assert_eq!(root.workspace().opened_projects().len(), 1);
    assert!(visible_tab_titles(root.workspace()).is_empty());
    assert!(root.visible_terminal_pane_contexts().is_empty());
    assert!(root.selected_project_is_empty());

    root.run_command(CommandId::TabNew).unwrap();

    let project_id = root.workspace().selected_project_id().unwrap();
    let project = root.workspace().project(project_id).unwrap();
    assert_eq!(project.layout.tabs.len(), 1);
    assert_eq!(project.selected_tab_id, "tab-1");
    assert_eq!(
        root.active_work_item(),
        Some(WorkItemId::Terminal("tab-1".to_string()))
    );
    assert!(!root.selected_project_is_empty());
    assert_eq!(root.visible_error_message(), None);
}

#[test]
fn root_view_terminal_exit_keeps_split_pane_for_manual_restart() {
    let mut root = WorkbenchView::dev_fixture();

    let outcome = root
        .handle_terminal_pane_exit(terminal_pane_exited_event_with_behavior(
            "dev",
            "server",
            ProcessExitBehavior::ManualRestart,
        ))
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
fn root_view_auto_restart_exit_transitions_back_to_running() {
    let mut root = WorkbenchView::dev_fixture();
    root.workspace_mut().select_tab("agent").unwrap();

    root.handle_terminal_pane_exit(terminal_pane_exited_event_with_behavior(
        "agent",
        "codex",
        ProcessExitBehavior::AutoRestart,
    ))
    .unwrap();

    assert_eq!(visible_tab_titles(root.workspace()), vec!["Dev", "Agent"]);
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    assert_eq!(
        project.tab_state("agent").unwrap().pane_states[0].process_state,
        PaneProcessState::Exited
    );

    root.handle_terminal_pane_started(TerminalPaneStartedEvent {
        project_id: project_id.as_str().to_string(),
        tab_id: "agent".to_string(),
        pane_id: "codex".to_string(),
    })
    .unwrap();

    let project = root.workspace().project(&project_id).unwrap();
    assert_eq!(
        project.tab_state("agent").unwrap().pane_states[0].process_state,
        PaneProcessState::Running
    );
}

#[test]
fn root_view_terminal_exit_keeps_last_tab_for_manual_restart() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/single"), single_tab_layout())
        .unwrap();
    let mut root = WorkbenchView::with_workspace_for_test(workspace);

    root.handle_terminal_pane_exit(TerminalPaneExitedEvent {
        project_id: "/tmp/single".to_string(),
        tab_id: "dev".to_string(),
        pane_id: "shell".to_string(),
        status: ProcessStatus::Exited { code: Some(0) },
        exit_reason: ExitReason::Completed,
        exit_behavior: ProcessExitBehavior::ManualRestart,
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
    let mut root = WorkbenchView::dev_fixture();
    let event = notification_event();

    root.focus_notification_target(&event).unwrap();

    assert_eq!(root.pending_terminal_focus_pane_id(), Some("codex"));
}

#[test]
fn root_view_focus_notification_target_leaves_active_file_for_terminal() {
    let mut root = WorkbenchView::dev_fixture();
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
        WorkbenchView::workspace_arrow_keydown_command("right", true, false, true, false),
        Some(CommandId::PaneFocusRight)
    );
    assert_eq!(
        WorkbenchView::workspace_arrow_keydown_command("left", false, true, true, false),
        Some(CommandId::PaneFocusLeft)
    );
    assert_eq!(
        WorkbenchView::workspace_arrow_keydown_command("down", true, false, true, true),
        Some(CommandId::PaneResizeDown)
    );
    assert_eq!(
        WorkbenchView::workspace_arrow_keydown_command("right", true, false, false, false),
        None
    );
    assert_eq!(
        WorkbenchView::workspace_arrow_keydown_command_for_owner(
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
    let mut root = WorkbenchView::new();

    root.handle_terminal_notification(notification_event());

    assert_eq!(root.visible_toast_titles(), vec!["Codex completed"]);
}

#[test]
fn root_view_records_agent_status_from_notification() {
    let mut root = WorkbenchView::dev_fixture();

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
    let mut root = WorkbenchView::new();
    let mut event = notification_event();
    event.kind = NotificationKind::AgentFailed;

    root.handle_terminal_notification(event);

    assert_eq!(root.visible_toast_titles(), vec!["Codex failed"]);
}

#[test]
fn root_view_focuses_notification_target() {
    let mut root = WorkbenchView::dev_fixture();
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
    let mut root = WorkbenchView::new();
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
        exit_behavior: ProcessExitBehavior::ManualRestart,
    };

    assert_eq!(event.project_id, "/tmp/yttt");
    assert_eq!(event.tab_id, "dev");
    assert_eq!(event.pane_id, "server");
    assert_eq!(event.exit_behavior, ProcessExitBehavior::ManualRestart);
}

#[test]
fn terminal_pane_lifecycle_labels_are_visible() {
    assert_eq!(pane_lifecycle_label(&PaneLifecycle::Running), "running");
    assert_eq!(
        pane_lifecycle_label(&PaneLifecycle::Stopping {
            reason: ExitReason::Completed,
        }),
        "stopping"
    );
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
    terminal_pane_exited_event_with_behavior(tab_id, pane_id, ProcessExitBehavior::Close)
}

fn terminal_pane_exited_event_with_behavior(
    tab_id: &str,
    pane_id: &str,
    exit_behavior: ProcessExitBehavior,
) -> TerminalPaneExitedEvent {
    TerminalPaneExitedEvent {
        project_id: "/tmp/yttt".to_string(),
        tab_id: tab_id.to_string(),
        pane_id: pane_id.to_string(),
        status: ProcessStatus::Exited { code: Some(0) },
        exit_reason: ExitReason::Completed,
        exit_behavior,
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

fn english_test_root() -> (tempfile::TempDir, WorkbenchView) {
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    (temp, WorkbenchView::with_config_paths(paths))
}

fn english_test_root_with_workspace(workspace: Workspace) -> (tempfile::TempDir, WorkbenchView) {
    let temp = tempdir().unwrap();
    let paths = english_test_config_paths(&temp);
    (
        temp,
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths),
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

fn replace_editor_value(
    input: &mut gpui_component::input::InputState,
    value: &str,
    window: &mut gpui::Window,
    cx: &mut gpui::Context<gpui_component::input::InputState>,
) {
    input.set_value("", window, cx);
    input.replace(value, window, cx);
}

fn project_file_autosave_fixture<'a>(
    cx: &'a mut gpui::TestAppContext,
    autosave: &str,
    delay_ms: u64,
) -> (
    tempfile::TempDir,
    PathBuf,
    gpui::Entity<WorkbenchView>,
    gpui::Entity<ProjectEditorDocument>,
    &'a mut gpui::VisualTestContext,
) {
    project_file_fixture(cx, autosave, delay_ms, file_editor_layout())
}

fn project_file_terminal_fixture<'a>(
    cx: &'a mut gpui::TestAppContext,
    autosave: &str,
    delay_ms: u64,
) -> (
    tempfile::TempDir,
    PathBuf,
    gpui::Entity<WorkbenchView>,
    gpui::Entity<ProjectEditorDocument>,
    &'a mut gpui::VisualTestContext,
) {
    project_file_fixture(cx, autosave, delay_ms, sample_layout())
}

fn project_file_fixture<'a>(
    cx: &'a mut gpui::TestAppContext,
    autosave: &str,
    delay_ms: u64,
    layout: yttt::model::layout::ProjectLayout,
) -> (
    tempfile::TempDir,
    PathBuf,
    gpui::Entity<WorkbenchView>,
    gpui::Entity<ProjectEditorDocument>,
    &'a mut gpui::VisualTestContext,
) {
    let temp = tempdir().unwrap();
    let project_dir = temp.path().join(format!("autosave-{autosave}"));
    fs::create_dir(&project_dir).unwrap();
    fs::write(project_dir.join("notes.txt"), "old").unwrap();
    let canonical_file = fs::canonicalize(project_dir.join("notes.txt")).unwrap();
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
    let project_id = workspace.open_project(project_dir.clone(), layout).unwrap();
    let preselected_project_id = project_id.clone();
    let preselected_file = canonical_file.clone();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| {
            let mut root =
                WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths);
            root.project_editor_runtime_mut()
                .workspace_mut()
                .session_mut(&preselected_project_id)
                .unwrap()
                .open_file(preselected_file);
            root
        });
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        register_workbench_keybinding_interceptor(cx, &root);
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
        canonical_path: canonical_file,
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

fn file_editor_layout() -> yttt::model::layout::ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "file-editor"
    "#,
    )
    .unwrap()
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

#[test]
fn git_commands_are_registered_in_action_and_keybinding_panels() {
    let mut root = WorkbenchView::dev_fixture();
    root.open_palette(PaletteKind::Command);
    let command_ids = root
        .active_palette_items()
        .into_iter()
        .map(|item| item.command)
        .collect::<Vec<_>>();
    assert!(command_ids.contains(&CommandId::GitBranchSwitch));
    assert!(command_ids.contains(&CommandId::GitDiffOpen));

    let keybinding_commands = root
        .visible_keybinding_rows()
        .into_iter()
        .map(|row| row.command)
        .collect::<Vec<_>>();
    assert!(keybinding_commands.contains(&CommandId::GitBranchSwitch));
    assert!(keybinding_commands.contains(&CommandId::GitDiffOpen));
}

#[test]
fn git_commands_open_branch_selector_and_diff_panel() {
    let mut root = WorkbenchView::dev_fixture();

    root.run_command(CommandId::GitBranchSwitch).unwrap();
    assert_eq!(
        root.active_palette().map(|palette| palette.kind),
        Some(PaletteKind::GitBranch)
    );

    root.run_command(CommandId::GitDiffOpen).unwrap();
    assert!(root.active_palette().is_none());
    assert!(root.git_diff_panel_is_open());
    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Dialog);
    assert!(!root.terminal_input_allowed());

    root.close_git_diff_panel();
    assert!(!root.git_diff_panel_is_open());
    assert_eq!(
        root.foreground_input_owner_kind(),
        InputOwnerKind::Workspace
    );
}

#[test]
fn configured_git_keybinding_dispatches_from_the_workbench() {
    let (_temp, mut root) = english_test_root_with_workspace(workspace_with_sample_project());
    root.set_keybinding_command_keys(CommandId::GitDiffOpen, vec!["cmd-shift-g".to_string()])
        .unwrap();
    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let document_id = root
        .project_editor_runtime_mut()
        .workspace_mut()
        .session_mut(&project_id)
        .unwrap()
        .open_file(PathBuf::from("/tmp/yttt/src/main.rs"));
    root.select_work_item(WorkItemId::File(document_id))
        .unwrap();
    assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Editor);

    assert_eq!(
        root.runtime_command_for_keystroke(&Keystroke::parse("cmd-shift-g").unwrap()),
        Some(CommandId::GitDiffOpen)
    );
    let command = root
        .runtime_command_for_dispatch(&Keystroke::parse("cmd-shift-g").unwrap())
        .unwrap();
    root.run_command(command).unwrap();
    assert!(root.git_diff_panel_is_open());
}
