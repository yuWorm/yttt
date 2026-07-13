use std::{
    cell::RefCell,
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    rc::Rc,
    time::{Duration, Instant},
};

use gpui::{EntityId, TestAppContext};
use tempfile::tempdir;

use super::*;

#[derive(Clone)]
struct RuntimeSnapshot {
    layout: ProjectLayout,
    terminal_entities: HashMap<String, EntityId>,
}

fn git(project_path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(project_path)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {} failed: {}",
        args.join(" "),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn runtime_snapshot(root: &WorkbenchView) -> RuntimeSnapshot {
    let project_id = root.workspace.selected_project_id().unwrap();
    RuntimeSnapshot {
        layout: root.workspace.project(project_id).unwrap().layout.clone(),
        terminal_entities: root
            .terminal
            .terminal_panes
            .iter()
            .map(|(key, pane)| (key.clone(), pane.entity_id()))
            .collect(),
    }
}

fn assert_runtime_unchanged(root: &WorkbenchView, expected: &RuntimeSnapshot) {
    let actual = runtime_snapshot(root);
    assert_eq!(actual.layout, expected.layout);
    assert_eq!(actual.terminal_entities, expected.terminal_entities);
}

#[gpui::test]
fn active_terminal_content_receives_default_focus(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(project_path, dev_fixture_layout())
        .unwrap();

    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    cx.run_until_parked();

    cx.update(|window, cx| {
        let root = root.read(cx);
        let workbench_focus = root
            .focus_handle
            .as_ref()
            .expect("render must initialize the workbench focus handle");
        assert!(
            workbench_focus.contains_focused(window, cx),
            "the active tab content must be inside the focused workbench subtree"
        );
        assert!(
            !workbench_focus.is_focused(window),
            "focus must land on the active tab content, not the workbench fallback"
        );
        assert!(
            root.terminal.pending_terminal_focus_pane_id.is_none(),
            "render must consume the active terminal focus request"
        );
    });
}

#[gpui::test]
fn titlebar_renders_branch_and_changes_actions(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(project_path, dev_fixture_layout())
        .unwrap();

    let (_root, cx) = cx.add_window_view(|_, _| {
        let mut root =
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths);
        root.project.project_git_statuses.insert(
            project_id,
            crate::runtime::git_status::parse_git_status_porcelain("## main\n M src/main.rs\n"),
        );
        root
    });
    cx.run_until_parked();

    assert!(cx.debug_bounds("titlebar-git-branch").is_some());
    assert!(cx.debug_bounds("titlebar-git-changes").is_some());
}

#[gpui::test]
fn active_project_file_watcher_refreshes_tree_and_git_status(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    git(&project_path, &["init"]);
    let other_project_path = temp.path().join("other-project");
    fs::create_dir(&other_project_path).unwrap();
    git(&other_project_path, &["init"]);
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(project_path.clone(), dev_fixture_layout())
        .unwrap();
    let other_project_id = workspace
        .open_project(other_project_path.clone(), dev_fixture_layout())
        .unwrap();
    workspace.select_project(&project_id).unwrap();

    let (root, mut cx) = cx.add_window_view(|_, _| {
        let mut root =
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths);
        root.project_file_watching_enabled = true;
        root
    });
    cx.run_until_parked();
    assert!(cx.read(|app| {
        root.read(app)
            .active_project_file_watcher
            .as_ref()
            .is_some_and(|watcher| {
                watcher.project_id == project_id && watcher.project_path == project_path
            })
    }));

    cx.background_executor
        .advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    cx.refresh().unwrap();
    cx.run_until_parked();

    fs::write(project_path.join("external.txt"), "created outside yttt\n").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        std::thread::sleep(Duration::from_millis(25));
        cx.background_executor
            .advance_clock(Duration::from_millis(200));
        cx.run_until_parked();
        cx.refresh().unwrap();
        cx.run_until_parked();

        let refreshed = cx.read(|app| {
            let root = root.read(app);
            let tree_refreshed = root
                .project
                .project_editor_runtime
                .workspace()
                .session(&project_id)
                .is_some_and(|session| {
                    session
                        .file_tree()
                        .visible_rows()
                        .iter()
                        .any(|row| row.relative_path == Path::new("external.txt"))
                });
            let git_refreshed = root
                .project
                .project_git_statuses
                .get(&project_id)
                .and_then(|status| status.file_status(Path::new("external.txt")))
                == Some(crate::runtime::git_status::GitFileStatus::Untracked);
            tree_refreshed && git_refreshed
        });
        if refreshed {
            break;
        }
        if Instant::now() >= deadline {
            let (rows, git_status) = cx.read(|app| {
                let root = root.read(app);
                let rows = root
                    .project
                    .project_editor_runtime
                    .workspace()
                    .session(&project_id)
                    .map(|session| session.file_tree().visible_rows())
                    .unwrap_or_default();
                let git_status = root.project.project_git_statuses.get(&project_id).cloned();
                (rows, git_status)
            });
            panic!(
                "active project watcher did not refresh in time; rows={rows:?}, git_status={git_status:?}"
            );
        }
    }

    let inactive_tree_generation = cx.read(|app| {
        root.read(app)
            .project
            .project_editor_runtime
            .workspace()
            .session(&project_id)
            .unwrap()
            .file_tree()
            .generation()
    });
    root.update(cx, |root, cx| {
        root.select_project(&other_project_id).unwrap();
        cx.notify();
    });
    cx.refresh().unwrap();
    cx.run_until_parked();
    assert!(cx.read(|app| {
        root.read(app)
            .active_project_file_watcher
            .as_ref()
            .is_some_and(|watcher| {
                watcher.project_id == other_project_id && watcher.project_path == other_project_path
            })
    }));

    fs::write(
        project_path.join("inactive.txt"),
        "changed while project is inactive\n",
    )
    .unwrap();
    std::thread::sleep(Duration::from_millis(50));
    cx.background_executor
        .advance_clock(Duration::from_millis(400));
    cx.run_until_parked();
    cx.refresh().unwrap();
    cx.run_until_parked();
    assert_eq!(
        cx.read(|app| {
            root.read(app)
                .project
                .project_editor_runtime
                .workspace()
                .session(&project_id)
                .unwrap()
                .file_tree()
                .generation()
        }),
        inactive_tree_generation
    );
}

#[gpui::test]
fn git_diff_panel_renders_controls_and_handles_shortcuts(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    git(&project_path, &["init"]);
    git(&project_path, &["config", "user.email", "test@example.com"]);
    git(&project_path, &["config", "user.name", "YTTT Test"]);
    fs::create_dir(project_path.join("src")).unwrap();
    fs::create_dir(project_path.join("tests")).unwrap();
    fs::write(project_path.join("src/one.rs"), "fn base() {}\n").unwrap();
    fs::write(project_path.join("tests/two.rs"), "fn base() {}\n").unwrap();
    git(&project_path, &["add", "."]);
    git(&project_path, &["commit", "-m", "initial"]);
    fs::write(project_path.join("src/one.rs"), "fn changed() {}\n").unwrap();
    fs::write(project_path.join("tests/two.rs"), "fn changed() {}\n").unwrap();

    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(project_path, dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        let mut root =
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths);
        root.app_settings.editor.font_family = "YTTT Test Editor Font".to_string();
        root.app_settings.editor.font_size = 18.0;
        root.app_settings.editor.line_height = 1.5;
        root
    });
    root.update_in(cx, |root, _window, cx| {
        root.open_git_diff_panel().unwrap();
        cx.notify();
    });
    cx.run_until_parked();

    assert!(cx.debug_bounds("git-diff-panel").is_some());
    assert!(cx.debug_bounds("git-diff-unified").is_some());
    assert!(cx.debug_bounds("git-diff-split").is_some());
    assert!(cx.debug_bounds("git-diff-file-0").is_some());
    assert!(cx.debug_bounds("git-diff-file-1").is_some());
    assert!(
        cx.debug_bounds("git-diff-unified-horizontal-scroll")
            .is_some()
    );
    assert_eq!(
        cx.debug_bounds("git-diff-line-1").unwrap().size.height,
        px(27.0),
        "diff rows must use the editor font size and line height"
    );
    assert!(cx.debug_bounds("git-diff-selected-file-0").is_some());
    cx.update(|_window, app| {
        let highlights = &root
            .read(app)
            .overlays
            .git_diff_panel
            .as_ref()
            .unwrap()
            .syntax_highlights;
        assert!(
            highlights
                .iter()
                .flatten()
                .any(|(_, style)| style.color.is_some()),
            "Rust diff lines must receive syntax colors"
        );
    });

    let first_folder = cx.debug_bounds("git-diff-folder-0").unwrap();
    cx.simulate_click(first_folder.center(), gpui::Modifiers::none());
    assert!(cx.debug_bounds("git-diff-file-0").is_none());
    assert!(cx.debug_bounds("git-diff-file-1").is_some());
    let first_folder = cx.debug_bounds("git-diff-folder-0").unwrap();
    cx.simulate_click(first_folder.center(), gpui::Modifiers::none());
    assert!(cx.debug_bounds("git-diff-file-0").is_some());

    let split = cx.debug_bounds("git-diff-split").unwrap();
    cx.simulate_click(split.center(), gpui::Modifiers::none());
    cx.update(|_window, app| {
        assert_eq!(
            root.read(app).git_diff_view_mode(),
            Some(GitDiffViewMode::Split)
        );
    });
    assert!(
        cx.debug_bounds("git-diff-split-left-horizontal-scroll")
            .is_some()
    );
    assert!(
        cx.debug_bounds("git-diff-split-right-horizontal-scroll")
            .is_some()
    );
    let left_pane = cx.debug_bounds("git-diff-split-left-pane").unwrap();
    let right_pane = cx.debug_bounds("git-diff-split-right-pane").unwrap();
    assert_eq!(
        left_pane.size.width, right_pane.size.width,
        "split panes must retain equal viewport widths regardless of line content"
    );
    assert_eq!(
        left_pane.origin.x + left_pane.size.width + px(1.0),
        right_pane.origin.x,
        "split panes must be separated by one fixed divider"
    );
    assert!(
        cx.debug_bounds("git-diff-split-left-header").is_some()
            && cx.debug_bounds("git-diff-split-right-header").is_some()
    );

    cx.simulate_keystrokes("s");
    cx.update(|_window, app| {
        assert_eq!(
            root.read(app).git_diff_view_mode(),
            Some(GitDiffViewMode::Unified)
        );
    });

    cx.simulate_keystrokes("down");
    cx.update(|_window, app| {
        assert_eq!(
            root.read(app)
                .overlays
                .git_diff_panel
                .as_ref()
                .unwrap()
                .selected_file,
            1
        );
    });
    assert!(
        cx.debug_bounds("git-diff-selected-file-0").is_none(),
        "the previous selection must repaint during the key event"
    );
    assert!(
        cx.debug_bounds("git-diff-selected-file-1").is_some(),
        "the next selection must repaint without waiting for focus loss"
    );

    cx.simulate_keystrokes("up");
    assert!(cx.debug_bounds("git-diff-selected-file-0").is_some());
    assert!(cx.debug_bounds("git-diff-selected-file-1").is_none());

    cx.simulate_keystrokes("down");
    assert!(cx.debug_bounds("git-diff-selected-file-0").is_none());
    assert!(cx.debug_bounds("git-diff-selected-file-1").is_some());

    cx.simulate_keystrokes(if cfg!(target_os = "macos") {
        "cmd-c"
    } else {
        "ctrl-c"
    });
    let copied = cx.read_from_clipboard().unwrap().text().unwrap();
    assert!(copied.contains("-fn base() {}"));
    assert!(copied.contains("+fn changed() {}"));

    cx.simulate_keystrokes("tab");
    cx.update(|_window, app| {
        assert_eq!(root.read(app).git_diff_mode(), Some(GitDiffMode::Staged));
    });

    cx.simulate_keystrokes("escape");
    cx.update(|_window, app| {
        assert!(!root.read(app).git_diff_panel_is_open());
    });
}

#[gpui::test]
fn git_diff_split_panes_share_vertical_scroll_but_keep_horizontal_scroll_independent(
    cx: &mut TestAppContext,
) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    git(&project_path, &["init"]);
    git(&project_path, &["config", "user.email", "test@example.com"]);
    git(&project_path, &["config", "user.name", "YTTT Test"]);
    let original = (0..120)
        .map(|line| format!("fn original_{line}() {{}}\n"))
        .collect::<String>();
    let changed = (0..120)
        .map(|line| {
            format!(
                "fn changed_{line}_with_a_long_name_that_requires_horizontal_scrolling_{}() {{}}\n",
                "segment_".repeat(16)
            )
        })
        .collect::<String>();
    fs::write(project_path.join("main.rs"), original).unwrap();
    git(&project_path, &["add", "main.rs"]);
    git(&project_path, &["commit", "-m", "initial"]);
    fs::write(project_path.join("main.rs"), changed).unwrap();

    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(project_path, dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    root.update_in(cx, |root, _window, cx| {
        root.open_git_diff_panel().unwrap();
        cx.notify();
    });
    cx.run_until_parked();
    let split = cx.debug_bounds("git-diff-split").unwrap();
    cx.simulate_click(split.center(), gpui::Modifiers::none());

    let left_pane = cx.debug_bounds("git-diff-split-left-pane").unwrap();
    for _ in 0..12 {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: left_pane.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-20.0))),
            ..Default::default()
        });
    }
    cx.run_until_parked();

    cx.update(|_window, app| {
        let offset = root
            .read(app)
            .overlays
            .git_diff_panel
            .as_ref()
            .unwrap()
            .diff_scroll_handle
            .0
            .borrow()
            .base_handle
            .offset();
        assert_ne!(
            offset.y,
            px(0.0),
            "the shared vertical viewport must scroll"
        );
    });
    let left_row = cx
        .debug_bounds("git-diff-split-left-row-12")
        .expect("left row 12 must be visible after scrolling");
    let right_row = cx
        .debug_bounds("git-diff-split-right-row-12")
        .expect("right row 12 must be visible after scrolling");
    assert_eq!(
        left_row.origin.y, right_row.origin.y,
        "paired rows must remain vertically aligned after scrolling"
    );
    assert!(
        left_row.size.width > left_pane.size.width,
        "long code rows must overflow their pane horizontally: row={:?}, pane={:?}",
        left_row.size.width,
        left_pane.size.width
    );

    cx.simulate_event(gpui::ScrollWheelEvent {
        position: left_pane.center(),
        delta: gpui::ScrollDelta::Pixels(gpui::point(px(-160.0), px(0.0))),
        ..Default::default()
    });
    cx.run_until_parked();
    cx.update(|_window, app| {
        let panel = root.read(app).overlays.git_diff_panel.as_ref().unwrap();
        assert_ne!(
            panel.split_left_horizontal_scroll_handle.offset().x,
            panel.split_right_horizontal_scroll_handle.offset().x,
            "horizontal scrolling must remain independent per pane"
        );
    });
}

#[gpui::test]
fn layout_default_does_not_drop_terminal_entities(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(project_path, dev_fixture_layout())
        .unwrap();

    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });

    root.update(cx, |root, _| {
        let before = runtime_snapshot(root);
        assert!(
            !before.terminal_entities.is_empty(),
            "render must create real terminal pane entities"
        );

        root.run_command(CommandId::LayoutDefaultEdit).unwrap();
        let updated = root
            .layout_toml_editor_value()
            .unwrap()
            .replace("title = \"Shell\"", "title = \"Saved Default\"");
        root.set_layout_toml_editor_value(updated);
        root.save_layout_toml_editor().unwrap();
        assert_runtime_unchanged(root, &before);

        root.run_command(CommandId::LayoutDefaultReload).unwrap();
        assert_runtime_unchanged(root, &before);

        root.run_command(CommandId::LayoutDefaultReset).unwrap();
        assert_runtime_unchanged(root, &before);

        root.run_command(CommandId::LayoutSaveCurrent).unwrap();
        root.run_command(CommandId::LayoutResetLocalOverride)
            .unwrap();
        assert_runtime_unchanged(root, &before);
    });
}

#[gpui::test]
fn project_entry_delete_alert_renders_and_executes_confirmation(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let victim_path = project_path.join("victim.txt");
    fs::write(&victim_path, "delete me").unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(project_path, dev_fixture_layout())
        .unwrap();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| {
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
        });
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        ComponentRoot::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    cx.run_until_parked();
    let project_id = cx.read(|cx| {
        root.read(cx)
            .workspace
            .selected_project_id()
            .unwrap()
            .clone()
    });

    root.update_in(cx, |root, window, root_cx| {
        root.confirm_project_entry_delete(
            project_id.clone(),
            PathBuf::from("victim.txt"),
            window,
            root_cx,
        );
    });
    cx.run_until_parked();

    cx.debug_bounds("project-entry-delete-confirm")
        .expect("delete confirmation must render an actionable button");
    root.update_in(cx, |root, window, root_cx| {
        root.spawn_project_entry_delete(project_id, PathBuf::from("victim.txt"), window, root_cx);
    });
    cx.run_until_parked();
    let deadline = Instant::now() + Duration::from_secs(1);
    while victim_path.exists() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
        cx.run_until_parked();
    }

    assert!(!victim_path.exists());
}
