use super::*;
use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::Path,
    time::{Duration, Instant},
};
use yttt_client_core::ClientCore;
use yttt_core::model::{
    host_workspace::{ProjectPanel, WorkspaceIntent},
    ids::{ClientInstanceId, ProfileId, ProjectId},
    layout::{
        LayoutNode, ProcessExitBehavior, ProjectConfig, ProjectLayout, TabConfig, TabStartup,
        TerminalExecutionMode,
    },
    project::{ProjectDescriptor, ProjectLocation},
};
use yttt_host::{HostBootstrap, HostLifetime};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, HostPath, ProtocolRange, RESOURCE_PROTOCOL_VERSION,
    ResourceCatalog,
    project::ProjectRequest,
    workspace::{WorkspaceId, WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse},
};
use yttt_transport_local::{
    AuthToken, ClientIdentity, LocalConnector, LocalEndpoint, LocalListener,
};

const COMMAND: &str = "printf 'start\\n' >> starts; while IFS= read -r line; do printf '%s\\n' \"$line\" >> inputs; done";

pub(crate) struct RecoveryHost {
    _runtime: tokio::runtime::Runtime,
    pub(crate) root: tempfile::TempDir,
    bootstrap: HostBootstrap,
}

impl RecoveryHost {
    pub(crate) fn start() -> Self {
        let root = tempfile::tempdir().unwrap();
        let runtime_root = root.path().join("runtime");
        fs::create_dir_all(&runtime_root).unwrap();
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        let auth_token_file = runtime_root.join("auth-token");
        fs::write(&auth_token_file, [0x5a; 32]).unwrap();
        fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
        let bootstrap = HostBootstrap {
            profile_id: ProfileId::new("terminal-recovery"),
            runtime_root,
            state_root: root.path().join("state"),
            config_root: root.path().join("config"),
            project_config: WorkspaceProjectConfig::Project,
            auth_token_file,
            ssh_host_keys_file: root.path().join("known-hosts"),
            credential_namespace: "dev.yttt.recovery-test".into(),
            build: BuildIdentity {
                product_version: "0.3.1".into(),
                build_fingerprint: "recovery-test".into(),
                resource_compatibility: "recovery-test".into(),
            },
            lifetime: HostLifetime::Independent,
        };
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let endpoint = LocalEndpoint::for_profile(
            bootstrap.profile_id.clone(),
            bootstrap.runtime_root.clone(),
        );
        runtime.spawn(yttt_host::run(bootstrap.clone(), || {
            LocalListener::bind(endpoint)
        }));
        runtime.block_on(async {
            tokio::time::timeout(Duration::from_secs(5), async {
                while !bootstrap.ready_file().exists() {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
            })
            .await
            .unwrap();
        });
        Self {
            _runtime: runtime,
            root,
            bootstrap,
        }
    }

    pub(crate) fn client(&self, id: &str) -> Arc<DesktopHostRuntime> {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let connector =
            yttt_transport::SharedConnector::new(LocalConnector::new(LocalEndpoint::for_profile(
                self.bootstrap.profile_id.clone(),
                self.bootstrap.runtime_root.clone(),
            )));
        let identity = ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: self.bootstrap.build.clone(),
            profile_id: self.bootstrap.profile_id.clone(),
            client_instance_id: ClientInstanceId::new(id),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: ConnectionChannel::Session,
        };
        let token = AuthToken::from_bytes([0x5a; 32]);
        let client = Arc::new(
            runtime
                .block_on(ClientCore::connect(
                    connector.clone(),
                    identity.clone(),
                    token.clone(),
                ))
                .unwrap(),
        );
        let Response::Workspace(WorkspaceResponse::Environment(environment)) = runtime
            .block_on(client.request(Request::Workspace(WorkspaceRequest::Environment)))
            .unwrap()
        else {
            panic!("missing environment")
        };
        let storage = Arc::new(crate::host_storage::HostStorage::new(
            client.clone(),
            runtime.handle().clone(),
            environment.config_root.to_path().unwrap(),
            environment.clone(),
            false,
        ));
        DesktopHostRuntime::from_remote(
            runtime,
            client,
            storage,
            connector,
            identity,
            token,
            environment,
            id.into(),
        )
        .unwrap()
    }
}

fn catalog(runtime: &DesktopHostRuntime) -> ResourceCatalog {
    let Response::Resources(catalog) = runtime
        .request_blocking_typed(Request::ListResources)
        .unwrap()
    else {
        panic!("expected resource catalog")
    };
    catalog
}

fn pump_until(
    cx: &mut VisualTestContext,
    label: &str,
    mut ready: impl FnMut(&mut VisualTestContext) -> bool,
) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if ready(cx) {
            return;
        }
        assert!(Instant::now() < deadline, "timed out: {label}");
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn open_pane<'a>(
    cx: &'a mut TestAppContext,
    runtime: Arc<DesktopHostRuntime>,
    root: &Path,
) -> (Entity<TerminalPaneView>, &'a mut VisualTestContext) {
    open_pane_with_command(cx, runtime, root, COMMAND)
}

/// Opens a workspace whose only pane runs `command`; the Host starts it and
/// the pane view attaches.
fn open_pane_with_command<'a>(
    cx: &'a mut TestAppContext,
    runtime: Arc<DesktopHostRuntime>,
    root: &Path,
    command: &str,
) -> (Entity<TerminalPaneView>, &'a mut VisualTestContext) {
    let project_id = ProjectId::new("recovery-project");
    runtime
        .request_blocking_typed(Request::Project(ProjectRequest::Register {
            project_id: project_id.clone(),
            view_id: "recovery-view".into(),
            root: HostPath::from_path(root).unwrap(),
        }))
        .unwrap();
    let pane = PaneConfig {
        id: "shell".into(),
        title: "Shell".into(),
        command: "/bin/sh".into(),
        args: vec!["-c".into(), command.into()],
        execution_mode: TerminalExecutionMode::Command,
        exit_behavior: ProcessExitBehavior::ManualRestart,
        kind: PaneKind::Shell,
        notify_on_exit: false,
        detector: None,
    };
    let layout = ProjectLayout {
        project: ProjectConfig {
            name: "Recovery".into(),
            default_tab: Some("tab".into()),
        },
        tabs: vec![TabConfig {
            id: "tab".into(),
            title: "Tab".into(),
            cwd: None,
            startup: TabStartup::Eager,
            layout: LayoutNode::Pane(pane.clone()),
        }],
    };
    let view = runtime.claim_workspace_view(false).unwrap();
    let workspace_id = view.id().clone();
    // The lease only names the workspace; the Host keeps it open.
    drop(view);
    let response = runtime.workspace_request(WorkspaceRequest::Apply {
        workspace_id,
        intent: WorkspaceIntent::OpenProject {
            descriptor: ProjectDescriptor::new(project_id, ProjectLocation::local(root)),
            layout,
            panel: ProjectPanel {
                visible: true,
                width: 240.0,
            },
        },
        expected_revision: None,
    });
    assert!(
        matches!(response, Ok(WorkspaceResponse::Applied { .. })),
        "{response:?}"
    );
    // The Host and PTY use real IO threads rather than the simulated clock.
    cx.background_executor.allow_parking();
    cx.skip_drawing();
    cx.update(|cx| {
        gpui_component::init(cx);
        yttt_terminal::init(cx);
        cx.set_global(HostRuntimeGlobal::ready(runtime));
    });
    let context = TerminalPaneContext {
        project_id: "recovery-project".into(),
        project_path: root.to_path_buf(),
        project_title: "Recovery".into(),
        tab_id: "tab".into(),
        tab_title: "Tab".into(),
        pane,
        is_focused: true,
        terminal_input_gate: TerminalInputGate::default(),
    };
    cx.add_window_view(|window, cx| {
        TerminalPaneView::new(
            context,
            TerminalConfig::default(),
            WorkbenchTheme::one_dark(),
            window,
            cx,
        )
    })
}

/// Asks the Host to start the pane again, as its restart button does.
fn restart_pane(runtime: &DesktopHostRuntime) {
    let workspace_id = runtime_workspace(runtime);
    let response = runtime.workspace_request(WorkspaceRequest::Apply {
        workspace_id,
        intent: WorkspaceIntent::RestartPane {
            project_id: ProjectId::new("recovery-project"),
            tab_id: "tab".into(),
            pane_id: "shell".into(),
            resume: false,
        },
        expected_revision: None,
    });
    assert!(
        matches!(response, Ok(WorkspaceResponse::Applied { .. })),
        "{response:?}"
    );
}

fn runtime_workspace(runtime: &DesktopHostRuntime) -> WorkspaceId {
    let Ok(WorkspaceResponse::Workspaces(index)) =
        runtime.workspace_request(WorkspaceRequest::List)
    else {
        panic!("workspace list")
    };
    index[0].workspace_id.clone()
}

fn send_input(pane: &Entity<TerminalPaneView>, cx: &mut VisualTestContext) {
    let terminal = cx.read(|app| pane.read(app).terminal.clone().unwrap());
    cx.update(|window, cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.replace_text_in_range(None, "x\r", window, cx);
        })
    });
}

fn received_input_count(root: &Path, input: &str) -> usize {
    // portable-pty sends a newline before EOF when its writer closes. The old
    // process may record it during termination; it is not user input.
    fs::read_to_string(root.join("inputs")).map_or(0, |recorded| {
        recorded.lines().filter(|line| *line == input).count()
    })
}

/// The terminal is gone and nothing replays it: shown as lost until the Host
/// records the process exit, then as exited.
fn stopped(lifecycle: &PaneLifecycle) -> bool {
    matches!(
        lifecycle,
        PaneLifecycle::Lost { .. } | PaneLifecycle::Exited { .. }
    )
}

#[gpui::test]
fn immediate_process_exit_is_finalized_after_spawn(cx: &mut TestAppContext) {
    let host = RecoveryHost::start();
    let runtime = host.client("first");
    let (pane, cx) = open_pane_with_command(cx, runtime.clone(), host.root.path(), "exit 7");
    pump_until(cx, "immediate exit finalized", |cx| {
        cx.read(|app| pane.read(app).exit_emitted)
    });
    cx.read(|app| {
        let pane = pane.read(app);
        assert!(!pane.is_running());
        assert!(pane.terminal.is_none());
    });
    pump_until(cx, "exited session acknowledged", |_| {
        catalog(&runtime).terminals.is_empty()
    });
    runtime.shutdown_client();
}

#[gpui::test]
fn a_lost_pane_attaches_to_the_terminal_the_host_restarts(cx: &mut TestAppContext) {
    let host = RecoveryHost::start();
    let runtime = host.client("first");
    let (pane, cx) = open_pane(cx, runtime.clone(), host.root.path());
    pump_until(cx, "initial terminal", |cx| {
        cx.read(|app| pane.read(app).is_running())
            && fs::read_to_string(host.root.path().join("starts"))
                .ok()
                .as_deref()
                == Some("start\n")
    });
    let original = catalog(&runtime).terminals.remove(0);
    pump_until(cx, "initial terminal metadata", |_| {
        runtime.terminal_metadata(&original.session_id).is_some()
    });
    let mut final_metadata = runtime.terminal_metadata(&original.session_id).unwrap();
    final_metadata.process_state = TerminalProcessState::Exited { code: Some(0) };
    runtime
        .terminate_many_confirmed(vec![original.session_id.clone()])
        .unwrap();
    pump_until(cx, "lost pane", |cx| {
        cx.read(|app| stopped(&pane.read(app).lifecycle))
    });

    // Nothing replays the command until the pane is restarted.
    cx.run_until_parked();
    assert!(catalog(&runtime).terminals.is_empty());
    assert_eq!(
        fs::read_to_string(host.root.path().join("starts")).unwrap(),
        "start\n"
    );
    cx.read(|app| assert!(stopped(&pane.read(app).lifecycle)));

    restart_pane(&runtime);
    pump_until(cx, "restarted terminal", |cx| {
        cx.read(|app| pane.read(app).is_running())
            && fs::read_to_string(host.root.path().join("starts"))
                .ok()
                .as_deref()
                == Some("start\nstart\n")
    });
    let replacement = catalog(&runtime).terminals.remove(0);
    assert_eq!(replacement.session_id, original.session_id);
    assert_ne!(replacement.session_epoch, original.session_epoch);
    // Late events about the terminal the Host replaced must not stop the
    // replacement.
    cx.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            pane.handle_host_viewport_metadata(&final_metadata, &runtime, cx);
            assert!(pane.is_running(), "old exit stopped the replacement");
            pane.handle_host_event(
                ClientEvent::TerminalUnavailable(original.session_id.clone()),
                &runtime,
                window,
                cx,
            );
            assert!(pane.is_running(), "old removal stopped the replacement");
        });
    });
    send_input(&pane, cx);
    pump_until(cx, "replacement accepts input", |_| {
        received_input_count(host.root.path(), "x") == 1
    });
    runtime
        .terminate_many_confirmed(vec![original.session_id])
        .unwrap();
    pump_until(cx, "replacement closed", |cx| {
        cx.read(|app| stopped(&pane.read(app).lifecycle))
    });
    runtime.shutdown_client();
}
