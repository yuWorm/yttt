use std::{collections::HashMap, fs, path::Path, process::Command};

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

    let (root, mut cx) = cx.add_window_view(|_, _| {
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

    let (_root, mut cx) = cx.add_window_view(|_, _| {
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
    let (root, mut cx) = cx.add_window_view(|_, _| {
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
    let (root, mut cx) = cx.add_window_view(|_, _| {
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
