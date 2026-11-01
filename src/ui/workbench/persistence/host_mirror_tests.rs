use super::*;
use crate::model::host_workspace::{NewTab, WorkspaceIntent};

/// Drives both the windows and the persistence tick until `done` holds.
pub(super) fn wait_until(
    cx: &mut gpui::VisualTestContext,
    view: &Entity<WorkbenchView>,
    what: &str,
    done: impl Fn(&WorkbenchView, &App) -> bool,
) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |root, cx| done(root, cx)) {
            return;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "{what}: {:?}",
            view.read_with(cx, |root, _| root.workspace_persistence.last_error.clone())
        );
        cx.executor().advance_clock(WORKSPACE_PERSISTENCE_INTERVAL);
        std::thread::sleep(Duration::from_millis(10));
    }
}

/// Opens a window on `runtime` whose workspace starts with the project at
/// `root`, and waits until the Host workspace shows that project.
pub(super) fn open_window(
    cx: &mut gpui::TestAppContext,
    runtime: &Arc<crate::host_runtime::DesktopHostRuntime>,
    root: &Path,
    config: PathBuf,
    view: Option<crate::session_coordinator::WorkspaceViewLease>,
) -> (Entity<WorkbenchView>, gpui::VisualTestContext) {
    let mut state = Workspace::new();
    state
        .open_project(
            ProjectDescriptor::new(
                ProjectId::new("mirror-project"),
                ProjectLocation::local(root.to_path_buf()),
            ),
            dev_fixture_layout(),
        )
        .unwrap();
    let paths = AppConfigPaths::from_config_dir(config);
    let handle =
        cx.add_window(|_, _| WorkbenchView::with_workspace_for_test_and_config_paths(state, paths));
    let window = handle.root(cx).unwrap();
    let mut visual = gpui::VisualTestContext::from_window(handle.into(), cx);
    window.update_in(&mut visual, |root, window, cx| {
        root.terminal.host_runtime = Some(runtime.clone());
        if let Some(view) = view {
            root.set_workspace_view(view);
        }
        root.start_workspace_persistence(false, window, cx);
    });
    wait_until(
        &mut visual,
        &window,
        "opening the Host workspace",
        |root, _| {
            root.workspace_persistence.mode == WorkspacePersistenceMode::Active
                && !root.workspace.opened_projects().is_empty()
        },
    );
    (window, visual)
}

#[gpui::test]
fn a_host_layout_change_made_elsewhere_replaces_the_local_layout(cx: &mut gpui::TestAppContext) {
    use crate::ui::terminal::pane::recovery_tests::RecoveryHost;
    cx.update(yttt_ui::init);
    cx.background_executor.allow_parking();
    cx.skip_drawing();
    let host = RecoveryHost::start();
    let runtime = host.client("host-mirror");
    let (view, mut cx) = open_window(
        cx,
        &runtime,
        host.root.path(),
        host.root.path().join("ui-config"),
        None,
    );
    let cx = &mut cx;
    let (id, project_id) = view.read_with(cx, |root, _| {
        (
            root.workspace_persistence
                .view
                .as_ref()
                .unwrap()
                .id()
                .clone(),
            root.workspace.opened_projects()[0].id.clone(),
        )
    });

    // Another Client adds a tab.
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
        Ok(WorkspaceResponse::Applied { .. })
    ));
    // Starting panes moves the Host revision too; the Client follows each one.
    wait_until(cx, &view, "adopting the Host layout", |root, _| {
        root.workspace
            .project(&project_id)
            .is_some_and(|project| project.layout.tab("mirrored").is_some())
            && root.workspace_persistence.revision == runtime.workspace_layout_revision(&id)
    });
    view.read_with(cx, |root, _| {
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
}
