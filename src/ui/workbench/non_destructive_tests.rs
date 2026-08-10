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
use crate::model::{
    layout::TabStartup,
    workspace::{PaneProcessState, TabStartState},
};

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

fn local_project(path: PathBuf) -> ProjectDescriptor {
    let location = ProjectLocation::local(path);
    let id = ProjectId::from_legacy_location(&location.display_path());
    ProjectDescriptor::new(id, location)
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

#[gpui::test]
fn remote_project_open_preserves_ssh_location_and_terminal_root(cx: &mut TestAppContext) {
    use crate::{
        config::ssh::{SshConnectionConfig, SshConnectionsConfig, save_ssh_connections},
        model::{ids::ConnectionId, project::RemotePathBuf},
    };

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let connection_id = ConnectionId::new("remote-dev");
    let mut connection = SshConnectionConfig::new("Remote Dev", "dev.example.com", 22, "alice");
    connection.id = connection_id.clone();
    save_ssh_connections(
        &config_paths,
        &SshConnectionsConfig {
            connections: vec![connection],
            ..SshConnectionsConfig::default()
        },
    )
    .unwrap();
    let remote_root = RemotePathBuf::new("/srv/remote-app").unwrap();
    let expected_root = PathBuf::from(remote_root.as_str());

    let (root, cx) = cx.add_window_view(|_, _| WorkbenchView::with_config_paths(config_paths));
    cx.update(|_, app| {
        root.update(app, |root, _cx| {
            root.open_ssh_project_location(connection_id.clone(), remote_root.clone(), false)
                .unwrap();
        });
    });

    cx.update(|_, app| {
        let root = root.read(app);
        let project_id = root.workspace.selected_project_id().unwrap();
        let project = root.workspace.project(project_id).unwrap();
        assert_eq!(
            project.location,
            ProjectLocation::Ssh {
                connection_id,
                root: remote_root,
            }
        );
        assert!(root.project.services.contains_key(project_id));
        let (_, terminal_root, _, _, _, _) = root.selected_tab_layout_clone().unwrap();
        assert_eq!(terminal_root, expected_root);
    });
}

#[gpui::test]
fn open_ssh_project_starts_in_connection_picker_without_configuration(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let (root, cx) = cx.add_window_view(|_, _| WorkbenchView::with_config_paths(config_paths));

    cx.update(|_, app| {
        root.update(app, |root, _cx| {
            root.open_ssh_project_picker();
        });
    });
    cx.run_until_parked();

    cx.update(|_, app| {
        let root = root.read(app);
        assert!(root.ssh.project_picker.open);
        assert_eq!(
            root.ssh.project_picker.view,
            SshProjectPickerView::Connections
        );
        assert!(root.ssh.connections.connections.is_empty());
        assert!(!root.ssh.manager_open);
        assert!(root.ssh.form.is_none());
    });
}

#[gpui::test]
fn ssh_project_password_connection_without_a_saved_secret_opens_a_focused_prompt(
    cx: &mut TestAppContext,
) {
    use crate::config::ssh::{
        SshAuthPreference, SshConnectionConfig, SshConnectionsConfig, save_ssh_connections,
    };
    use crate::model::ids::ConnectionId;

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let connection_id = ConnectionId::new("password-prompt");
    let mut connection = SshConnectionConfig::new("Password Host", "host.example.com", 22, "alice");
    connection.id = connection_id.clone();
    connection.auth = SshAuthPreference::Password;
    save_ssh_connections(
        &config_paths,
        &SshConnectionsConfig {
            connections: vec![connection],
            ..SshConnectionsConfig::default()
        },
    )
    .unwrap();

    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths(config_paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        ComponentRoot::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update_in(cx, |root, _window, cx| {
        root.open_ssh_project_picker();
        cx.notify();
    });
    cx.run_until_parked();
    let icon = cx
        .debug_bounds("ssh-connection-list-icon")
        .expect("remote connection rows must render a semantic icon");
    assert!(icon.size.width > px(0.0) && icon.size.height > px(0.0));

    root.update_in(cx, |root, _window, cx| {
        root.select_ssh_project_connection(connection_id.clone(), cx);
        cx.notify();
    });
    cx.run_until_parked();

    cx.update(|window, app| {
        let root = root.read(app);
        assert_eq!(root.ssh.project_picker.view, SshProjectPickerView::Password);
        assert!(root.ssh.project_picker.remember_password);
        let input = root
            .ssh
            .project_picker
            .password_input
            .as_ref()
            .expect("password prompt must create its input");
        assert!(input.read(app).focus_handle(app).is_focused(window));
    });
}

#[gpui::test]
fn ssh_project_directory_rows_show_icons_align_left_and_scroll(cx: &mut TestAppContext) {
    use crate::model::project::RemotePathBuf;

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let (root, cx) = cx.add_window_view(|_, _| WorkbenchView::with_config_paths(config_paths));

    root.update_in(cx, |root, _window, cx| {
        root.open_ssh_project_picker();
        root.ssh.project_picker.view = SshProjectPickerView::Browsing;
        root.ssh.project_picker.current_path = Some(RemotePathBuf::new("/").unwrap());
        root.ssh.project_picker.directories = (0..24)
            .map(|index| SshProjectDirectory {
                name: format!("directory-{index:02}"),
                path: RemotePathBuf::new(format!("/directory-{index:02}")).unwrap(),
            })
            .collect();
        cx.notify();
    });
    cx.run_until_parked();

    let list = cx
        .debug_bounds("ssh-project-directory-list")
        .expect("directory list must render");
    let row = cx
        .debug_bounds("ssh-project-directory-content-/directory-00")
        .expect("directory row content must render");
    let icon = cx
        .debug_bounds("ssh-project-directory-icon-/directory-00")
        .expect("directory row must render an icon");
    assert!(
        icon.size.width > px(0.0) && icon.size.height > px(0.0),
        "directory icon must occupy visible space: icon={icon:?}"
    );
    assert!(
        icon.origin.x >= row.origin.x
            && icon.origin.x + icon.size.width <= row.origin.x + row.size.width,
        "directory icon must remain inside the row: icon={icon:?}, row={row:?}"
    );
    assert!(
        row.origin.x <= list.origin.x + px(24.0),
        "directory content must start at the list's left edge: row={row:?}, list={list:?}"
    );
    assert!(
        row.size.width >= list.size.width - px(48.0),
        "directory content must fill the row instead of centering: row={row:?}, list={list:?}"
    );
    for _ in 0..4 {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: row.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-120.0))),
            ..Default::default()
        });
    }
    cx.run_until_parked();
    let row_after_scroll = cx
        .debug_bounds("ssh-project-directory-content-/directory-00")
        .expect("directory row must remain rendered after scrolling");
    assert!(
        row_after_scroll.origin.y < row.origin.y - px(1.0),
        "directory list must move rows in response to wheel input: before={row:?}, after={row_after_scroll:?}"
    );
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
        .open_project(local_project(project_path), dev_fixture_layout())
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
            root.terminal.pending_terminal_focus.is_none(),
            "render must consume the active terminal focus request"
        );
    });
}

#[gpui::test]
fn eager_tab_starts_terminal_before_selection(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("eager-project");
    fs::create_dir(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut layout = dev_fixture_layout();
    layout
        .tabs
        .iter_mut()
        .find(|tab| tab.id == "agent")
        .unwrap()
        .startup = TabStartup::Eager;
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), layout)
        .unwrap();
    let eager_pane_key = terminal_pane_key(project_id.as_str(), "agent", "codex");

    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    cx.run_until_parked();

    cx.update(|_, cx| {
        let root = root.read(cx);
        let project = root.workspace.project(&project_id).unwrap();
        let agent_state = project.tab_state("agent").unwrap();

        assert_eq!(project.selected_tab_id, "dev");
        assert_eq!(agent_state.start_state, TabStartState::Started);
        assert_eq!(
            agent_state.pane_states[0].process_state,
            PaneProcessState::Running
        );
        assert!(root.terminal.terminal_panes.contains_key(&eager_pane_key));
    });
}

#[gpui::test]
fn keybinding_recorder_renders_focuses_and_records(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

    let (root, cx) = cx.add_window_view(|_, _| WorkbenchView::with_config_paths(config_paths));
    cx.update(|_, app| {
        root.update(app, |root, cx| {
            root.open_keybinding_edit_dialog(CommandId::TabPalette)
                .unwrap();
            cx.notify();
        });
    });
    cx.refresh().unwrap();
    cx.run_until_parked();

    assert!(cx.debug_bounds("keybinding-recorder").is_some());
    cx.simulate_keystrokes("cmd-l");
    cx.run_until_parked();

    cx.update(|window, app| {
        let root = root.read(app);
        assert_eq!(
            root.pending_keybinding_edit_keys(),
            Some(vec![if cfg!(target_os = "windows") {
                "win-l".to_string()
            } else {
                "cmd-l".to_string()
            }])
        );
        assert!(
            root.focus_handle
                .as_ref()
                .expect("render must initialize the workbench focus handle")
                .is_focused(window)
        );
    });
}

#[gpui::test]
fn window_reactivation_restores_previously_focused_terminal_pane(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    workspace.focus_pane("shell").unwrap();

    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    cx.update(|window, cx| {
        crate::ui::app::register_workbench_focus_restore(window, cx, &root);
    });
    cx.run_until_parked();

    let terminal_focus = cx.update(|window, cx| {
        let root = root.read(cx);
        assert_eq!(root.selected_focused_pane_id(), Some("shell"));
        assert!(
            !root
                .focus_handle
                .as_ref()
                .expect("render must initialize the workbench focus handle")
                .is_focused(window),
            "initial focus must land on the selected terminal pane"
        );
        window
            .focused(cx)
            .expect("the selected terminal pane must own focus")
    });
    cx.update(|window, cx| {
        let workbench_focus = root
            .read(cx)
            .focus_handle
            .as_ref()
            .expect("render must initialize the workbench focus handle")
            .clone();
        workbench_focus.focus(window, cx);
    });

    cx.deactivate_window();
    cx.update(|window, _cx| window.activate_window());
    cx.run_until_parked();

    cx.update(|window, cx| {
        assert!(window.is_window_active());
        assert!(
            terminal_focus.is_focused(window),
            "reactivation must restore the exact terminal pane that was selected before deactivation"
        );
        assert_eq!(root.read(cx).selected_focused_pane_id(), Some("shell"));
    });
}

#[gpui::test]
fn project_tree_edit_blocks_active_file_focus_restore(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path.clone()), dev_fixture_layout())
        .unwrap();
    let view_project_id = project_id.clone();
    let active_file = project_path.join("README.md");

    let (root, cx) = cx.add_window_view(move |_, _| {
        let mut root =
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths);
        let session = root
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&view_project_id)
            .expect("the project editor session must exist");
        session.set_project_panel_visible(false);
        session.open_file(active_file);
        root.sync_input_owner_state();
        root
    });
    let tree = cx.update(|window, app| {
        root.update(app, |root, cx| {
            root.ensure_project_tree_view(&project_id, window, cx)
                .expect("the project tree must exist")
        })
    });
    cx.update(|window, app| {
        tree.update(app, |tree, cx| {
            tree.begin_create_selected(false, window, cx);
        });
    });

    cx.update(|_window, app| {
        root.update(app, |root, cx| {
            root.project.pending_editor_focus_document_id = None;
            assert_eq!(root.foreground_input_owner_kind(), InputOwnerKind::Editor);
            assert!(
                !root.queue_default_active_work_item_focus(cx),
                "an inline project-tree input must suppress active editor focus restoration"
            );
            assert!(root.project.pending_editor_focus_document_id.is_none());
        });
    });
}

#[gpui::test]
fn agent_exit_notification_does_not_reenter_workbench_entity(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), agent_exit_fixture_layout())
        .unwrap();
    let notification = NotificationEvent {
        kind: NotificationKind::AgentCompleted,
        project_id: project_id.as_str().to_string(),
        tab_id: "agent".to_string(),
        pane_id: "codex".to_string(),
        project_title: "yttt-agent-exit".to_string(),
        tab_title: "Agent".to_string(),
        pane_title: "Codex".to_string(),
    };

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
    let pane = cx.update(|_window, cx| {
        root.read(cx)
            .terminal
            .terminal_panes
            .values()
            .next()
            .cloned()
            .expect("render must create the agent terminal pane")
    });
    let event = TerminalPaneEvent::Notification(notification.clone());

    cx.update(|window, cx| {
        root.update(cx, |root, cx| {
            root.on_terminal_pane_event(&pane, &event, window, cx);
        });
    });

    cx.update(|_window, cx| {
        assert_eq!(root.read(cx).toast_queue.events(), &[notification]);
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
        .open_project(local_project(project_path), dev_fixture_layout())
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
        .open_project(local_project(project_path.clone()), dev_fixture_layout())
        .unwrap();
    let other_project_id = workspace
        .open_project(
            local_project(other_project_path.clone()),
            dev_fixture_layout(),
        )
        .unwrap();
    workspace.select_project(&project_id).unwrap();

    let (root, cx) = cx.add_window_view(|_, _| {
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
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.bind_keys(crate::ui::interaction::actions::app_startup_keybindings());
    });
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
        .open_project(local_project(project_path), dev_fixture_layout())
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
    cx.update(|_window, app| {
        let panel = root.read(app).overlays.git_diff_panel.as_ref().unwrap();
        assert!(panel.collapsed_folders.contains("src"));
        assert!(
            !panel
                .sidebar_rows
                .iter()
                .any(|row| { matches!(row, GitDiffSidebarRow::File { file_index: 0, .. }) })
        );
    });
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

    cx.update(|_window, app| {
        app.write_to_clipboard(ClipboardItem::new_string("sentinel".to_string()));
    });
    let copy = cx.debug_bounds("git-diff-copy").unwrap();
    cx.simulate_click(copy.center(), gpui::Modifiers::none());
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
fn git_diff_panel_virtualizes_many_files_and_large_file_rows(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        cx.bind_keys(crate::ui::interaction::actions::app_startup_keybindings());
    });
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    let bulk_path = project_path.join("bulk");
    fs::create_dir_all(&bulk_path).unwrap();
    git(&project_path, &["init"]);
    git(&project_path, &["config", "user.email", "test@example.com"]);
    git(&project_path, &["config", "user.name", "YTTT Test"]);
    for index in 0..400 {
        fs::write(
            bulk_path.join(format!("file_{index:04}.rs")),
            "fn original() {}\n",
        )
        .unwrap();
    }
    git(&project_path, &["add", "."]);
    git(&project_path, &["commit", "-m", "initial"]);
    for index in 0..400 {
        fs::write(
            bulk_path.join(format!("file_{index:04}.rs")),
            "fn changed() {}\n",
        )
        .unwrap();
    }
    let large_file = (0..30_000)
        .map(|line| format!("fn added_{line}() {{}}\n"))
        .collect::<String>();
    fs::write(project_path.join("large.rs"), large_file).unwrap();

    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    root.update_in(cx, |root, _window, cx| {
        root.open_git_diff_panel().unwrap();
        cx.notify();
    });
    cx.run_until_parked();

    assert!(cx.debug_bounds("git-diff-file-399").is_none());
    root.update_in(cx, |root, _window, cx| {
        let panel = root.overlays.git_diff_panel.as_ref().unwrap();
        let GitDiffPanelContent::Ready(result) = &panel.content else {
            panic!("Git diff must finish loading");
        };
        let large_index = result
            .files
            .iter()
            .position(|file| file.path() == "large.rs")
            .unwrap();
        assert!(root.select_git_diff_file(large_index));
        cx.notify();
    });
    cx.run_until_parked();

    cx.update(|_window, app| {
        let panel = root.read(app).overlays.git_diff_panel.as_ref().unwrap();
        let GitDiffPanelContent::Ready(result) = &panel.content else {
            panic!("Git diff must remain loaded");
        };
        let file = &result.files[panel.selected_file];
        assert!(file.line_count() > 30_000);
        assert!(panel.syntax_highlights.is_empty());
        assert!(panel.unified_view_rows.is_empty());
        assert!(panel.split_left_view_rows.is_empty());
        assert!(panel.split_right_view_rows.is_empty());
    });
    assert!(cx.debug_bounds("git-diff-line-1").is_some());
    assert!(cx.debug_bounds("git-diff-line-20000").is_none());
    cx.simulate_keystrokes("s");
    assert!(
        cx.debug_bounds("git-diff-split-left-row-1").is_some()
            && cx.debug_bounds("git-diff-split-right-row-1").is_some()
    );
    assert!(cx.debug_bounds("git-diff-split-left-row-20000").is_none());
    assert!(cx.debug_bounds("git-diff-split-right-row-20000").is_none());
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
        .open_project(local_project(project_path), dev_fixture_layout())
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
        .open_project(local_project(project_path), dev_fixture_layout())
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
        .open_project(local_project(project_path), dev_fixture_layout())
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

    let confirm = cx
        .debug_bounds("project-entry-delete-confirm")
        .expect("delete confirmation must render an actionable button");
    let cancel = cx
        .debug_bounds("project-entry-delete-cancel")
        .expect("delete confirmation must render a compact cancel button");
    assert_eq!(confirm.size.height, gpui::px(DEFAULT_UI_FONT_SIZE * 1.25));
    assert_eq!(cancel.size.height, confirm.size.height);
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

#[derive(Clone)]
struct CountingSystemNotifier(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl SystemNotifier for CountingSystemNotifier {
    fn notify(&self, _event: &NotificationEvent) -> anyhow::Result<()> {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    }
}

#[gpui::test]
fn agent_state_transitions_enqueue_attention_and_completion_notifications(cx: &mut TestAppContext) {
    use yttt_agent_core::{AgentInstanceId, AgentProcessState, AgentTurnState, ProviderId};

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let address = AgentPaneAddress::new(project_id.as_str(), "agent", "codex");
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let system_notification_count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let system_notification_count_for_window = system_notification_count.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| {
            let mut root =
                WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths);
            root.system_notifications_enabled = true;
            root.system_notifier =
                Arc::new(CountingSystemNotifier(system_notification_count_for_window));
            root
        });
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        ComponentRoot::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    let snapshot = |turn_state| AgentSnapshot {
        instance_id: AgentInstanceId::new("notification-test-agent").unwrap(),
        provider_id: ProviderId::from_static("codex"),
        generation: 1,
        process_state: AgentProcessState::Running,
        turn_state,
        waiting_reason: None,
        waiting_message: None,
        task: None,
        current_action: None,
        last_action_failed: false,
        children: Vec::new(),
        session: None,
        process_exit: None,
        state_started_at: 1,
        updated_at: 1,
    };

    root.update_in(cx, |root, window, cx| {
        root.record_agent_event_snapshot(
            address.clone(),
            snapshot(AgentTurnState::Waiting),
            window,
            cx,
        )
        .unwrap();
        root.record_agent_event_snapshot(
            address.clone(),
            snapshot(AgentTurnState::Waiting),
            window,
            cx,
        )
        .unwrap();
        root.record_agent_event_snapshot(
            address.clone(),
            snapshot(AgentTurnState::Working),
            window,
            cx,
        )
        .unwrap();
        root.record_agent_event_snapshot(address, snapshot(AgentTurnState::Completed), window, cx)
            .unwrap();
        assert_eq!(
            root.visible_toast_titles(),
            vec![
                "Codex needs attention".to_string(),
                "Codex completed".to_string()
            ]
        );
    });
    assert_eq!(
        system_notification_count.load(std::sync::atomic::Ordering::SeqCst),
        2
    );
}

#[gpui::test]
fn killed_detected_agent_clears_sidebar_snapshot_without_notification(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let address = AgentPaneAddress::new(project_id.as_str(), "dev", "shell");
    let view_project_id = project_id.clone();
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

    root.update_in(cx, |root, window, cx| {
        let running = root
            .agent_manager
            .detected_process_started(address.clone(), BuiltinAgent::Codex, 7)
            .unwrap();
        root.record_agent_runtime_snapshot(address.clone(), running)
            .unwrap();
        assert!(
            root.workspace
                .project(&view_project_id)
                .unwrap()
                .tab_state("dev")
                .unwrap()
                .pane_states
                .iter()
                .find(|pane| pane.pane_id == "shell")
                .unwrap()
                .agent_snapshot
                .is_some()
        );

        assert!(
            root.finish_detected_agent(&address, 7, AgentExitReason::KilledByUser, window, cx,)
        );
        assert!(
            root.workspace
                .project(&view_project_id)
                .unwrap()
                .tab_state("dev")
                .unwrap()
                .pane_states
                .iter()
                .find(|pane| pane.pane_id == "shell")
                .unwrap()
                .agent_snapshot
                .is_none()
        );
        assert!(root.agent_manager.retained_snapshots().is_empty());
        assert!(root.visible_toast_titles().is_empty());
    });
}
fn persist_codex_shell_session(
    config_paths: &AppConfigPaths,
    project_path: &Path,
) -> ProjectReferenceConfig {
    use crate::runtime::agent_hooks::AgentHookRequest;

    let mut layout = dev_fixture_layout();
    layout
        .tabs
        .iter_mut()
        .find(|tab| tab.id == "dev")
        .unwrap()
        .startup = TabStartup::Eager;
    save_local_layout(config_paths, project_path, &layout).unwrap();
    let opened = open_project_config(
        config_paths,
        project_path,
        &mut DefaultLayoutState::load_or_create(config_paths),
    )
    .unwrap();
    let project = ProjectReferenceConfig::new(
        opened.descriptor.id.clone(),
        opened.descriptor.location.clone(),
    );
    let address = AgentPaneAddress::new(project.id.as_str(), "dev", "shell");
    let mut manager = AgentManager::new(config_paths);
    manager
        .detected_process_started(address.clone(), BuiltinAgent::Codex, 7)
        .unwrap();
    manager
        .ingest_hook_request(AgentHookRequest {
            address: address.clone(),
            generation: 7,
            source: BuiltinAgent::Codex,
            event: "SessionStart".to_string(),
            payload: serde_json::json!({ "session_id": "codex-session-1" }),
        })
        .unwrap()
        .unwrap();
    manager
        .ingest_hook_request(AgentHookRequest {
            address,
            generation: 7,
            source: BuiltinAgent::Codex,
            event: "UserPromptSubmit".to_string(),
            payload: serde_json::json!({ "prompt": "Fix the flaky terminal test" }),
        })
        .unwrap()
        .unwrap();
    project
}

#[gpui::test]
fn opening_recent_project_does_not_restore_persisted_agent_session(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let project = persist_codex_shell_session(&config_paths, &project_path);
    let view_project_id = project.id.clone();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths_for_test(config_paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        ComponentRoot::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.open_project_path(&project_path).unwrap();
        cx.notify();
    });
    cx.run_until_parked();

    cx.update(|_, app| {
        let root = root.read(app);
        let project = root.workspace.project(&view_project_id).unwrap();
        let pane_state = project
            .tab_state("dev")
            .unwrap()
            .pane_states
            .iter()
            .find(|pane| pane.pane_id == "shell")
            .unwrap();
        assert!(
            pane_state.agent_snapshot.is_none(),
            "opening a recent project must start a fresh workspace"
        );
        let pane = root
            .terminal
            .terminal_panes
            .get(&terminal_pane_key(view_project_id.as_str(), "dev", "shell"))
            .unwrap()
            .read(app);
        assert_eq!(pane.title(), "shell");
        assert!(pane.agent_instance_id().is_none());
        assert!(root.agent_manager.retained_snapshots().is_empty());
    });
}

#[gpui::test]
fn restoring_last_session_restores_persisted_agent_session(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let project = persist_codex_shell_session(&config_paths, &project_path);
    let view_project_id = project.id.clone();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths_for_test(config_paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        ComponentRoot::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.recent_projects_config.last_opened_projects = vec![project];
        assert_eq!(root.restore_last_opened_projects(), 1);
        cx.notify();
    });
    cx.run_until_parked();

    cx.update(|_, app| {
        let root = root.read(app);
        let snapshot = root
            .workspace
            .project(&view_project_id)
            .unwrap()
            .tab_state("dev")
            .unwrap()
            .pane_states
            .iter()
            .find(|pane| pane.pane_id == "shell")
            .unwrap()
            .agent_snapshot
            .as_ref()
            .unwrap();
        assert_eq!(snapshot.primary_text(), "Fix the flaky terminal test");
        let pane = root
            .terminal
            .terminal_panes
            .get(&terminal_pane_key(view_project_id.as_str(), "dev", "shell"))
            .unwrap()
            .read(app);
        assert_eq!(pane.title(), "Fix the flaky terminal test");
        assert!(pane.agent_instance_id().is_some());
    });
}

#[gpui::test]
fn failed_restored_shell_session_is_replaced_with_a_fresh_agent(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let project = persist_codex_shell_session(&config_paths, &project_path);
    let project_id = project.id.clone();
    let root_slot = Rc::new(RefCell::new(None));
    let root_slot_for_window = root_slot.clone();
    let (_component_root, cx) = cx.add_window_view(move |window, cx| {
        let root = cx.new(|_| WorkbenchView::with_config_paths_for_test(config_paths));
        *root_slot_for_window.borrow_mut() = Some(root.clone());
        ComponentRoot::new(root, window, cx)
    });
    let root = root_slot.borrow_mut().take().unwrap();
    root.update(cx, |root, cx| {
        root.recent_projects_config.last_opened_projects = vec![project];
        assert_eq!(root.restore_last_opened_projects(), 1);
        cx.notify();
    });
    cx.run_until_parked();

    let key = terminal_pane_key(project_id.as_str(), "dev", "shell");
    let (failed_pane, failed_instance, generation) = cx.update(|_, app| {
        let pane = root
            .read(app)
            .terminal
            .terminal_panes
            .get(&key)
            .unwrap()
            .clone();
        let pane_state = pane.read(app);
        (
            pane.clone(),
            pane_state.agent_instance_id().unwrap().clone(),
            pane_state.generation(),
        )
    });
    root.update_in(cx, |root, window, cx| {
        root.on_terminal_pane_event(
            &failed_pane,
            &TerminalPaneEvent::Exited(TerminalPaneExitedEvent {
                project_id: project_id.as_str().to_string(),
                tab_id: "dev".to_string(),
                pane_id: "shell".to_string(),
                status: yttt_terminal::ProcessStatus::Exited { code: Some(1) },
                exit_reason: yttt_terminal::ExitReason::Failed,
                exit_behavior: ProcessExitBehavior::ManualRestart,
                generation,
                agent_instance_id: Some(failed_instance.clone()),
            }),
            window,
            cx,
        );
    });
    cx.run_until_parked();

    cx.update(|_, app| {
        let root = root.read(app);
        let fresh_pane = root.terminal.terminal_panes.get(&key).unwrap().read(app);
        assert_ne!(
            fresh_pane.agent_instance_id(),
            Some(&failed_instance),
            "the failed resume launch must not be reused"
        );
        let snapshot = root
            .workspace
            .project(&project_id)
            .unwrap()
            .tab_state("dev")
            .unwrap()
            .pane_states
            .iter()
            .find(|pane| pane.pane_id == "shell")
            .unwrap()
            .agent_snapshot
            .as_ref()
            .unwrap();
        assert_eq!(snapshot.provider_id.as_str(), "codex");
        assert!(snapshot.session.is_none());
    });
}

#[gpui::test]
fn local_agent_sessions_are_preloaded_before_the_tab_is_selected(cx: &mut TestAppContext) {
    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.agent.sessions_enabled = false;
    save_settings(&config_paths, &settings).unwrap();
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });

    root.update(cx, |root, _cx| {
        assert_eq!(
            root.project.active_panel_page,
            ProjectPanelPage::Files,
            "the sessions tab must remain unopened"
        );
        root.app_settings.agent.sessions_enabled = true;
        root.app_settings.agent.primary = Some(BuiltinAgent::OhMyPi);
        root.ensure_agent_session_scan_requested();

        assert!(root.agent_sessions.pending_scan);
        assert!(root.agent_sessions.loading);
        let untitled = AgentSession {
            provider: BuiltinAgent::OhMyPi,
            id: "opaque-session-id".to_string(),
            title: String::new(),
            model: None,
            transcript_path: None,
            updated_at_ms: 0,
        };
        assert_ne!(root.agent_session_title(&untitled), untitled.id);
        assert_eq!(
            root.agent_session_agents(),
            vec![BuiltinAgent::OhMyPi],
            "the primary agent must be the only default session provider"
        );
        assert_eq!(
            root.agent_sessions.key,
            Some(AgentSessionScanKey {
                project_id,
                agents: vec![BuiltinAgent::OhMyPi],
            })
        );
    });
}

#[gpui::test]
fn multiple_agent_providers_are_collapsed_into_groups(cx: &mut TestAppContext) {
    use std::sync::Arc;

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.agent.sessions_enabled = false;
    save_settings(&config_paths, &settings).unwrap();
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.app_settings.agent.sessions_enabled = true;
        root.app_settings.agent.primary = Some(BuiltinAgent::Codex);
        root.app_settings.agent.additional_session_agents = vec![BuiltinAgent::Claude];
        root.project.active_panel_page = ProjectPanelPage::AgentSessions;
        root.agent_sessions.key = Some(AgentSessionScanKey {
            project_id,
            agents: vec![BuiltinAgent::Codex, BuiltinAgent::Claude],
        });
        root.agent_sessions.sessions = Arc::new(vec![
            AgentSession {
                provider: BuiltinAgent::Codex,
                id: "codex-session-1".to_string(),
                title: "Codex work".to_string(),
                model: None,
                transcript_path: None,
                updated_at_ms: 2,
            },
            AgentSession {
                provider: BuiltinAgent::Claude,
                id: "claude-session-1".to_string(),
                title: "Claude work".to_string(),
                model: None,
                transcript_path: None,
                updated_at_ms: 1,
            },
        ]);
        cx.notify();
    });
    cx.refresh().unwrap();

    let codex_group = cx
        .debug_bounds("agent-session-provider-codex")
        .expect("multiple providers should render a Codex group");
    assert!(
        cx.debug_bounds("agent-session-provider-claude").is_some(),
        "multiple providers should render a Claude Code group"
    );
    assert!(
        cx.debug_bounds("agent-session-provider-icon-codex")
            .is_some(),
        "provider groups should use the Codex icon"
    );
    assert!(
        cx.debug_bounds("agent-session-row-0").is_none()
            && cx.debug_bounds("agent-session-row-1").is_none(),
        "provider groups should start collapsed"
    );

    cx.simulate_click(codex_group.center(), gpui::Modifiers::none());
    cx.run_until_parked();
    cx.refresh().unwrap();

    assert!(
        cx.debug_bounds("agent-session-row-0").is_some(),
        "expanding Codex should reveal its sessions"
    );
    assert!(
        cx.debug_bounds("agent-session-row-1").is_none(),
        "expanding Codex must not expand Claude Code"
    );
}

#[gpui::test]
fn agent_session_search_filters_visible_rows(cx: &mut TestAppContext) {
    use std::sync::Arc;

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.agent.sessions_enabled = false;
    save_settings(&config_paths, &settings).unwrap();
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.app_settings.agent.sessions_enabled = true;
        root.app_settings.agent.primary = Some(BuiltinAgent::Codex);
        root.app_settings.agent.additional_session_agents = vec![BuiltinAgent::Claude];
        root.project.active_panel_page = ProjectPanelPage::AgentSessions;
        root.agent_sessions.key = Some(AgentSessionScanKey {
            project_id,
            agents: vec![BuiltinAgent::Codex, BuiltinAgent::Claude],
        });
        root.agent_sessions.sessions = Arc::new(vec![
            AgentSession {
                provider: BuiltinAgent::Codex,
                id: "restore-auth".to_string(),
                title: "Restore authentication".to_string(),
                model: Some("gpt-5.6".to_string()),
                transcript_path: None,
                updated_at_ms: 3,
            },
            AgentSession {
                provider: BuiltinAgent::Claude,
                id: "render-session".to_string(),
                title: "Investigate rendering".to_string(),
                model: None,
                transcript_path: None,
                updated_at_ms: 2,
            },
            AgentSession {
                provider: BuiltinAgent::Codex,
                id: "ship-release".to_string(),
                title: "Ship release".to_string(),
                model: None,
                transcript_path: None,
                updated_at_ms: 1,
            },
        ]);
        cx.notify();
    });
    cx.refresh().unwrap();

    let search = cx
        .debug_bounds("agent-sessions-search")
        .expect("a populated session list must render its global search input");
    let search_input_bounds = cx
        .debug_bounds("agent-sessions-search-input")
        .expect("the global search input must render");
    assert!(
        search_input_bounds.origin.x >= search.origin.x + px(7.0)
            && search_input_bounds.origin.x + search_input_bounds.size.width
                <= search.origin.x + search.size.width - px(7.0)
            && search_input_bounds.origin.y > search.origin.y
            && search_input_bounds.size.height < search.size.height,
        "the search input must use compact dimensions with breathing room: container={search:?}, input={search_input_bounds:?}"
    );
    let search_input = cx.update(|_, app| {
        root.read(app)
            .agent_sessions
            .search_input
            .clone()
            .expect("session search input must be initialized")
    });
    search_input.update_in(cx, |input, window, cx| {
        input.set_value("RENDERING", window, cx);
    });
    cx.run_until_parked();
    cx.refresh().unwrap();

    assert!(
        cx.debug_bounds("agent-session-row-0").is_none()
            && cx.debug_bounds("agent-session-row-2").is_none(),
        "non-matching sessions must be hidden"
    );
    assert!(
        cx.debug_bounds("agent-session-row-1").is_some(),
        "session title matching must be case-insensitive"
    );

    search_input.update_in(cx, |input, window, cx| {
        input.set_value("no such session", window, cx);
    });
    cx.run_until_parked();
    cx.refresh().unwrap();

    assert!(
        cx.debug_bounds("agent-sessions-no-matches").is_some(),
        "an unmatched query must render an empty-search result"
    );
}

#[gpui::test]
fn overflowing_agent_session_list_scrolls_within_the_project_panel(cx: &mut TestAppContext) {
    use std::sync::Arc;

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.agent.sessions_enabled = false;
    save_settings(&config_paths, &settings).unwrap();
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.app_settings.agent.sessions_enabled = true;
        root.app_settings.agent.primary = Some(BuiltinAgent::Codex);
        root.project.active_panel_page = ProjectPanelPage::AgentSessions;
        root.agent_sessions.key = Some(AgentSessionScanKey {
            project_id,
            agents: vec![BuiltinAgent::Codex],
        });
        root.agent_sessions.sessions = Arc::new(
            (0..40)
                .map(|index| AgentSession {
                    provider: BuiltinAgent::Codex,
                    id: format!("codex-session-{index}"),
                    title: format!("Codex session {index}"),
                    model: None,
                    transcript_path: None,
                    updated_at_ms: 0,
                })
                .collect(),
        );
        cx.notify();
    });
    cx.refresh().unwrap();

    let surface = cx
        .debug_bounds("workbench-surface")
        .expect("workbench surface must render");
    let panel = cx
        .debug_bounds("project-file-panel")
        .expect("project panel must render");
    let page = cx
        .debug_bounds("project-panel-page-agent-sessions")
        .expect("session page must render");
    let list = cx
        .debug_bounds("agent-sessions-list")
        .expect("session list must render");
    assert!(
        panel.origin.y + panel.size.height <= surface.origin.y + surface.size.height,
        "project panel must stay within the workbench surface: surface={surface:?}, panel={panel:?}"
    );
    assert!(
        list.origin.y + list.size.height <= panel.origin.y + panel.size.height,
        "session list must stay within the project panel: panel={panel:?}, page={page:?}, list={list:?}"
    );
    let first_row = cx
        .debug_bounds("agent-session-row-0")
        .expect("first session row must render");
    for _ in 0..4 {
        cx.simulate_event(gpui::ScrollWheelEvent {
            position: first_row.center(),
            delta: gpui::ScrollDelta::Pixels(gpui::point(px(0.0), px(-120.0))),
            ..Default::default()
        });
    }
    cx.run_until_parked();
    cx.refresh().unwrap();

    let first_row_after_scroll = cx
        .debug_bounds("agent-session-row-0")
        .expect("first session row must remain rendered after scrolling");
    assert!(
        first_row_after_scroll.origin.y < first_row.origin.y - px(1.0),
        "session list must move rows in response to wheel input: before={first_row:?}, after={first_row_after_scroll:?}"
    );
}

#[gpui::test]
fn long_agent_session_tooltip_content_stays_within_its_layout(cx: &mut TestAppContext) {
    use std::sync::Arc;

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut settings = AppSettings::default();
    settings.agent.sessions_enabled = false;
    save_settings(&config_paths, &settings).unwrap();
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.app_settings.agent.sessions_enabled = true;
        root.app_settings.agent.primary = Some(BuiltinAgent::Codex);
        root.project.active_panel_page = ProjectPanelPage::AgentSessions;
        root.agent_sessions.key = Some(AgentSessionScanKey {
            project_id,
            agents: vec![BuiltinAgent::Codex],
        });
        root.agent_sessions.sessions = Arc::new(vec![AgentSession {
            provider: BuiltinAgent::Codex,
            id: format!("codex-session-{}", "0123456789".repeat(12)),
            title: "Convert a very long imported theme while preserving every color and syntax token setting ".repeat(4),
            model: Some(format!("openai-codex/{}", "gpt-5.6-specialized".repeat(8))),
            transcript_path: Some(PathBuf::from(format!(
                "{}.jsonl",
                "2026-07-14T04-44-44-session-transcript".repeat(5)
            ))),
            updated_at_ms: 1,
        }]);
        cx.notify();
    });
    cx.refresh().unwrap();

    let row = cx
        .debug_bounds("agent-session-row-0")
        .expect("session row must render");
    cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::none());
    cx.background_executor
        .advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.refresh().unwrap();

    let tooltip = cx
        .debug_bounds("agent-session-tooltip")
        .expect("session tooltip must render");
    let title = cx
        .debug_bounds("agent-session-tooltip-title")
        .expect("tooltip title must render");
    let session_id = cx
        .debug_bounds("agent-session-tooltip-session-id")
        .expect("tooltip session ID must render");
    let model = cx
        .debug_bounds("agent-session-tooltip-model")
        .expect("tooltip model must render");
    let transcript = cx
        .debug_bounds("agent-session-tooltip-transcript")
        .expect("tooltip transcript must render");

    for field in [title, session_id, model, transcript] {
        assert!(
            field.origin.x >= tooltip.origin.x
                && field.origin.x + field.size.width <= tooltip.origin.x + tooltip.size.width,
            "tooltip content must remain within its horizontal bounds: tooltip={tooltip:?}, field={field:?}"
        );
    }
    assert!(
        title.origin.y + title.size.height <= session_id.origin.y
            && session_id.origin.y + session_id.size.height <= model.origin.y
            && model.origin.y + model.size.height <= transcript.origin.y,
        "tooltip fields must not overlap vertically: title={title:?}, session_id={session_id:?}, model={model:?}, transcript={transcript:?}"
    );
}

#[gpui::test]
fn double_clicking_discovered_session_creates_an_agent_command_tab(cx: &mut TestAppContext) {
    use std::sync::Arc;

    cx.update(gpui_component::init);
    let temp = tempdir().unwrap();
    let project_path = temp.path().join("project");
    fs::create_dir_all(&project_path).unwrap();
    let config_paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(project_path), dev_fixture_layout())
        .unwrap();
    let (root, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(workspace, config_paths)
    });
    cx.run_until_parked();

    root.update(cx, |root, cx| {
        root.app_settings.agent.primary = Some(BuiltinAgent::Codex);
        root.project.active_panel_page = ProjectPanelPage::AgentSessions;
        root.agent_sessions.key = Some(AgentSessionScanKey {
            project_id: project_id.clone(),
            agents: vec![BuiltinAgent::Codex],
        });
        root.agent_sessions.sessions = Arc::new(vec![AgentSession {
            provider: BuiltinAgent::Codex,
            id: "codex-session-1".to_string(),
            title: "Restore auth session".to_string(),
            model: Some("gpt-5.6".to_string()),
            transcript_path: Some(PathBuf::from("codex-session-1.jsonl")),
            updated_at_ms: 1,
        }]);
        cx.notify();
    });
    cx.refresh().unwrap();

    let original_tab_count = cx.update(|_, app| {
        root.read(app)
            .workspace
            .project(&project_id)
            .unwrap()
            .layout
            .tabs
            .len()
    });
    let row = cx
        .debug_bounds("agent-session-row-0")
        .expect("discovered session row should render");
    assert!(
        cx.debug_bounds("agent-session-provider-codex").is_none(),
        "a single provider must not add a collapsible group"
    );
    assert!(
        cx.debug_bounds("agent-session-provider-icon-0").is_some(),
        "a single provider row should use its Agent icon"
    );
    cx.simulate_mouse_move(row.center(), None, gpui::Modifiers::none());
    cx.background_executor
        .advance_clock(Duration::from_millis(500));
    cx.run_until_parked();
    cx.refresh().unwrap();
    assert!(
        cx.debug_bounds("agent-session-tooltip").is_some(),
        "hovering a session row should show its metadata tooltip"
    );

    cx.simulate_click(row.center(), gpui::Modifiers::none());
    cx.update(|_, app| {
        assert_eq!(
            root.read(app)
                .workspace
                .project(&project_id)
                .unwrap()
                .layout
                .tabs
                .len(),
            original_tab_count,
            "a single click must not resume a session"
        );
    });

    cx.simulate_event(MouseDownEvent {
        position: row.center(),
        button: MouseButton::Left,
        modifiers: gpui::Modifiers::none(),
        click_count: 2,
        first_mouse: false,
    });
    cx.simulate_event(MouseUpEvent {
        position: row.center(),
        button: MouseButton::Left,
        modifiers: gpui::Modifiers::none(),
        click_count: 2,
    });

    cx.update(|_, app| {
        let root = root.read(app);
        let project = root.workspace.project(&project_id).unwrap();
        assert_eq!(project.layout.tabs.len(), original_tab_count + 1);
        let tab = project.layout.tab(&project.selected_tab_id).unwrap();
        let pane = tab.layout.find_pane("agent").unwrap();
        assert_eq!(tab.title, "Restore auth session");
        assert_eq!(pane.command, "codex");
        assert_eq!(pane.args, ["resume", "codex-session-1"]);
        assert_eq!(pane.kind, crate::model::layout::PaneKind::Agent);
        assert_eq!(
            pane.execution_mode,
            crate::model::layout::TerminalExecutionMode::Command
        );
    });
}
