use super::*;
use crate::model::host_workspace::{NewTab, WorkspaceIntent};

fn wait_until(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<WorkbenchView>,
    what: &str,
    done: impl Fn(&WorkbenchView) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |root, _| done(root)) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: {:?}",
            view.read_with(cx, |root, _| root.workspace_persistence.last_error.clone())
        );
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[gpui::test]
fn a_host_layout_change_made_elsewhere_replaces_the_local_layout_without_an_echo_commit(
    cx: &mut gpui::TestAppContext,
) {
    use crate::ui::terminal::pane::recovery_tests::RecoveryHost;
    cx.update(gpui_component::init);
    cx.background_executor.allow_parking();
    cx.skip_drawing();
    let host = RecoveryHost::start();
    let runtime = host.client("host-mirror");
    let lease = runtime.claim_workspace_view(false).unwrap();
    let id = lease.id().clone();
    let project_id = ProjectId::new("mirror-project");
    let mut state = Workspace::new();
    state
        .open_project(
            ProjectDescriptor::new(
                project_id.clone(),
                ProjectLocation::local(host.root.path().to_path_buf()),
            ),
            dev_fixture_layout(),
        )
        .unwrap();
    state
        .mark_pane_running(&project_id, "dev", "shell")
        .unwrap();
    let paths = AppConfigPaths::from_config_dir(host.root.path().join("ui-config"));
    let (view, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(state, paths)
    });
    view.update_in(cx, |root, window, cx| {
        root.terminal.host_runtime = Some(runtime.clone());
        root.workspace_persistence.view = Some(lease);
        root.workspace_persistence.revision = Some(0);
        root.workspace_persistence.host_epoch = remote_host_epoch(&runtime);
        root.workspace_persistence.control_context =
            runtime.control_status().map(|status| status.context);
        root.workspace_persistence.mode = WorkspacePersistenceMode::Active;
        root.commit_remote_workspace_if_changed(runtime.clone(), id.clone(), window, cx);
    });
    wait_until(cx, &view, "initial commit", |root| {
        root.workspace_persistence.revision == Some(1)
    });
    assert!(matches!(
        runtime.workspace_request(WorkspaceRequest::Open {
            workspace_id: id.clone(),
        }),
        Ok(WorkspaceResponse::Opened { revision: 1, .. })
    ));
    view.update_in(cx, |root, window, cx| {
        root.ensure_host_layout_listener(&runtime, &id, window, cx);
    });

    // Another window of this profile adds a tab.
    assert!(matches!(
        runtime.workspace_request(WorkspaceRequest::Apply {
            workspace_id: id.clone(),
            intent: WorkspaceIntent::CreateTab {
                project_id: project_id.clone(),
                tab: NewTab::Shell {
                    command: String::new(),
                },
                tab_id: Some("mirrored".into()),
            },
            expected_revision: None,
        }),
        Ok(WorkspaceResponse::Applied { revision: 2, .. })
    ));
    wait_until(cx, &view, "adopting the Host layout", |root| {
        root.workspace_persistence.revision == Some(2)
    });
    view.read_with(cx, |root, _| {
        let project = root.workspace.project(&project_id).unwrap();
        assert!(project.layout.tab("mirrored").is_some());
        assert!(
            root.project
                .project_editor_runtime
                .workspace()
                .session(&project_id)
                .and_then(|session| {
                    session.group_items_containing(&WorkItemId::Terminal("mirrored".into()))
                })
                .is_some(),
            "the editor work area mirrors the Host tabs"
        );
    });

    // The adopted layout is what the Host holds, so nothing is republished.
    view.update_in(cx, |root, window, cx| {
        root.commit_remote_workspace_if_changed(runtime.clone(), id.clone(), window, cx);
    });
    cx.run_until_parked();
    std::thread::sleep(Duration::from_millis(100));
    cx.run_until_parked();
    assert_eq!(
        view.read_with(cx, |root, _| root.workspace_persistence.revision),
        Some(2)
    );
    assert_eq!(runtime.workspace_layout_revision(&id), Some(2));
}
