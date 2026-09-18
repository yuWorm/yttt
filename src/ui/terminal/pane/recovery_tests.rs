use super::*;
use gpui::{EntityInputHandler, TestAppContext, VisualTestContext};
use std::{fs, os::unix::fs::PermissionsExt, path::Path, time::Instant};
use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_host::{HostBootstrap, HostLifetime};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, HostPath, ProtocolRange, RESOURCE_PROTOCOL_VERSION,
    ResourceCatalog,
    project::ProjectRequest,
    session::ProfileControlRequest,
    terminal::{AttachTerminal, TerminalLeaseMode},
    workspace::{WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse},
};
use yttt_transport_local::{
    AuthToken, ClientIdentity, LocalConnector, LocalEndpoint, LocalListener,
};

const COMMAND: &str = "printf 'start\\n' >> starts; while IFS= read -r line; do printf '%s\\n' \"$line\" >> inputs; done";

struct RecoveryHost {
    _runtime: tokio::runtime::Runtime,
    root: tempfile::TempDir,
    bootstrap: HostBootstrap,
}

impl RecoveryHost {
    fn start() -> Self {
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

    fn client(&self, id: &str) -> Arc<DesktopHostRuntime> {
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
            channel: ConnectionChannel::Control,
            terminal_session_id: None,
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
        if client.control_status().unwrap().owner.is_none() {
            runtime
                .block_on(client.request(Request::ProfileControl(
                    ProfileControlRequest::RequestControl,
                )))
                .unwrap();
        }
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
    runtime
        .request_blocking_typed(Request::Project(ProjectRequest::Register {
            project_id: ProjectId::new("recovery-project"),
            view_id: "recovery-view".into(),
            root: HostPath::from_path(root).unwrap(),
        }))
        .unwrap();
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
        pane: PaneConfig {
            id: "shell".into(),
            title: "Shell".into(),
            command: "/bin/sh".into(),
            args: vec!["-c".into(), COMMAND.into()],
            execution_mode: TerminalExecutionMode::Command,
            exit_behavior: ProcessExitBehavior::ManualRestart,
            kind: PaneKind::Shell,
            notify_on_exit: false,
            detector: None,
        },
        shell: "/bin/sh".into(),
        environment: Arc::new(RwLock::new(BTreeMap::new())),
        is_focused: true,
        terminal_input_gate: TerminalInputGate::default(),
        ssh: None,
        agent_launch: None,
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

fn send_input(pane: &Entity<TerminalPaneView>, cx: &mut VisualTestContext) {
    let terminal = cx.read(|app| pane.read(app).terminal.clone().unwrap());
    cx.update(|window, cx| {
        terminal.update(cx, |terminal, cx| {
            terminal.replace_text_in_range(None, "x\r", window, cx);
        })
    });
}

#[gpui::test]
fn reclaiming_profile_restores_existing_pane_input_and_close(cx: &mut TestAppContext) {
    let host = RecoveryHost::start();
    let first = host.client("first");
    let second = host.client("second");
    let (pane, cx) = open_pane(cx, first.clone(), host.root.path());
    pump_until(cx, "initial terminal", |cx| {
        cx.read(|app| pane.read(app).is_running()) && host.root.path().join("starts").exists()
    });
    let original = catalog(&first).terminals.remove(0);
    second
        .request_blocking_typed(Request::ProfileControl(
            ProfileControlRequest::RequestControl,
        ))
        .unwrap();
    pump_until(cx, "second controller", |_| second.shared_editing_enabled());
    second
        .request_blocking_typed(Request::AttachTerminal(AttachTerminal {
            session_id: original.session_id.clone(),
            known_session_epoch: Some(original.session_epoch),
            after_sequence: None,
            mode: TerminalLeaseMode::Interactive,
            geometry: original.geometry,
            geometry_epoch: original.geometry_epoch + 1,
            query_palette: vec![],
            palette_revision: 1,
        }))
        .unwrap();
    first
        .request_blocking_typed(Request::ProfileControl(
            ProfileControlRequest::RequestControl,
        ))
        .unwrap();
    pump_until(cx, "first controller", |_| first.shared_editing_enabled());
    pump_until(cx, "reclaimed terminal ownership", |cx| {
        cx.read(|app| pane.read(app).is_running())
            && catalog(&first).terminals.iter().any(|placement| {
                placement.session_id == original.session_id
                    && placement.owner == first.control_status().unwrap().owner
            })
    });
    send_input(&pane, cx);
    pump_until(cx, "input reaches original process", |_| {
        fs::read_to_string(host.root.path().join("inputs"))
            .ok()
            .as_deref()
            == Some("x\n")
    });
    let restored = catalog(&first);
    assert_eq!(restored.terminals.len(), 1);
    assert_eq!(restored.terminals[0].session_epoch, original.session_epoch);
    assert_eq!(
        fs::read_to_string(host.root.path().join("starts")).unwrap(),
        "start\n"
    );
    first
        .terminate_many_confirmed(vec![original.session_id])
        .unwrap();
    assert!(catalog(&first).terminals.is_empty());
    pump_until(cx, "closed pane reconciled", |cx| {
        cx.read(|app| matches!(pane.read(app).lifecycle, PaneLifecycle::Lost { .. }))
    });
    second.shutdown_client();
    first.shutdown_client();
}

#[gpui::test]
fn unavailable_pane_reconnects_when_session_reappears_without_spawning(cx: &mut TestAppContext) {
    let host = RecoveryHost::start();
    let runtime = host.client("first");
    let (pane, cx) = open_pane(cx, runtime.clone(), host.root.path());
    pump_until(cx, "initial terminal", |cx| {
        cx.read(|app| pane.read(app).is_running()) && host.root.path().join("starts").exists()
    });
    let original = catalog(&runtime).terminals.remove(0);
    runtime
        .terminate_many_confirmed(vec![original.session_id.clone()])
        .unwrap();
    pump_until(cx, "unavailable pane", |cx| {
        cx.read(|app| matches!(pane.read(app).lifecycle, PaneLifecycle::Lost { .. }))
    });
    cx.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            pane.start_host_terminal(window, cx, TerminalStartIntent::Attach);
        })
    });
    pump_until(cx, "failed reconnect settles", |cx| {
        cx.read(|app| matches!(pane.read(app).lifecycle, PaneLifecycle::Lost { .. }))
    });
    assert!(catalog(&runtime).terminals.is_empty());
    assert_eq!(
        fs::read_to_string(host.root.path().join("starts")).unwrap(),
        "start\n"
    );
    let response = runtime
        .request_blocking_typed(Request::SpawnTerminal {
            start_id: "explicit-replacement".into(),
            expected_host_epoch: catalog(&runtime).host_epoch,
            spec: TerminalSpawnSpec {
                session_id: original.session_id.clone(),
                project_id: original.project_id,
                cwd: yttt_protocol::ProjectRelativePath::root(),
                execution: TerminalExecutionSpec::Command {
                    shell: "/bin/sh".into(),
                    program: "/bin/sh".into(),
                    args: vec!["-c".into(), COMMAND.into()],
                    return_to_shell: false,
                },
                geometry: original.geometry,
                geometry_epoch: 1,
                query_palette: vec![],
                palette_revision: 1,
                scrollback_limit: 1000,
                environment: vec![],
                removed_environment: vec![],
            },
        })
        .unwrap();
    let Response::TerminalSpawned { session_epoch, .. } = response else {
        panic!("replacement did not spawn")
    };
    assert_ne!(session_epoch, original.session_epoch);
    pump_until(cx, "reattached replacement", |cx| {
        cx.read(|app| pane.read(app).is_running())
    });
    send_input(&pane, cx);
    pump_until(cx, "input reaches replacement", |_| {
        fs::read_to_string(host.root.path().join("inputs"))
            .ok()
            .as_deref()
            == Some("x\n")
    });
    assert_eq!(
        fs::read_to_string(host.root.path().join("starts")).unwrap(),
        "start\nstart\n"
    );
    runtime
        .terminate_many_confirmed(vec![original.session_id])
        .unwrap();
    pump_until(cx, "closed pane reconciled", |cx| {
        cx.read(|app| matches!(pane.read(app).lifecycle, PaneLifecycle::Lost { .. }))
    });
    runtime.shutdown_client();
}

#[gpui::test]
fn missing_terminal_can_be_explicitly_started_from_recovery(cx: &mut TestAppContext) {
    let host = RecoveryHost::start();
    let runtime = host.client("first");
    let (pane, cx) = open_pane(cx, runtime.clone(), host.root.path());
    pump_until(cx, "initial terminal", |cx| {
        cx.read(|app| pane.read(app).is_running()) && host.root.path().join("starts").exists()
    });
    let original = catalog(&runtime).terminals.remove(0);
    runtime
        .terminate_many_confirmed(vec![original.session_id.clone()])
        .unwrap();
    pump_until(cx, "unavailable pane", |cx| {
        cx.read(|app| matches!(pane.read(app).lifecycle, PaneLifecycle::Lost { .. }))
    });
    assert!(catalog(&runtime).terminals.is_empty());
    assert_eq!(
        fs::read_to_string(host.root.path().join("starts")).unwrap(),
        "start\n"
    );

    cx.update(|window, cx| pane.update(cx, |pane, cx| pane.retry_terminal(window, cx)));
    pump_until(cx, "explicit replacement terminal", |cx| {
        cx.read(|app| pane.read(app).is_running())
            && fs::read_to_string(host.root.path().join("starts"))
                .ok()
                .as_deref()
                == Some("start\nstart\n")
    });
    let replacement = catalog(&runtime).terminals.remove(0);
    assert_eq!(replacement.session_id, original.session_id);
    assert_ne!(replacement.session_epoch, original.session_epoch);
    send_input(&pane, cx);
    pump_until(cx, "replacement accepts input", |_| {
        fs::read_to_string(host.root.path().join("inputs"))
            .ok()
            .as_deref()
            == Some("x\n")
    });
    runtime
        .terminate_many_confirmed(vec![original.session_id])
        .unwrap();
    pump_until(cx, "replacement closed", |cx| {
        cx.read(|app| matches!(pane.read(app).lifecycle, PaneLifecycle::Lost { .. }))
    });
    runtime.shutdown_client();
}

#[gpui::test]
fn missing_terminal_waits_for_control_without_automatically_replaying(cx: &mut TestAppContext) {
    let host = RecoveryHost::start();
    let runtime = host.client("first");
    let (pane, cx) = open_pane(cx, runtime.clone(), host.root.path());
    pump_until(cx, "initial terminal", |cx| {
        cx.read(|app| pane.read(app).is_running()) && host.root.path().join("starts").exists()
    });
    let original = catalog(&runtime).terminals.remove(0);
    runtime
        .terminate_many_confirmed(vec![original.session_id.clone()])
        .unwrap();
    pump_until(cx, "unavailable pane", |cx| {
        cx.read(|app| matches!(pane.read(app).lifecycle, PaneLifecycle::Lost { .. }))
    });
    let controller = host.client("second");
    controller
        .request_blocking_typed(Request::ProfileControl(
            ProfileControlRequest::RequestControl,
        ))
        .unwrap();
    pump_until(cx, "control transferred", |_| {
        controller.shared_editing_enabled() && !runtime.shared_editing_enabled()
    });

    // A restoration that races control transfer must remain recoverable.
    cx.update(|window, cx| {
        pane.update(cx, |pane, cx| {
            pane.start_restored_terminal(window, cx, false);
        })
    });
    pump_until(cx, "restoration settles", |cx| {
        cx.read(|app| {
            matches!(
                pane.read(app).lifecycle,
                PaneLifecycle::Lost { .. } | PaneLifecycle::SpawnFailed { .. }
            )
        })
    });
    cx.read(|app| {
        assert!(matches!(
            pane.read(app).lifecycle,
            PaneLifecycle::Lost { .. }
        ))
    });
    cx.update(|window, cx| pane.update(cx, |pane, cx| pane.retry_terminal(window, cx)));
    cx.run_until_parked();
    assert!(catalog(&runtime).terminals.is_empty());
    runtime
        .request_blocking_typed(Request::ProfileControl(
            ProfileControlRequest::RequestControl,
        ))
        .unwrap();
    pump_until(cx, "control reclaimed", |_| {
        runtime.shared_editing_enabled()
    });
    cx.run_until_parked();
    assert!(
        catalog(&runtime).terminals.is_empty(),
        "taking control is not consent to replay a command"
    );
    cx.update(|window, cx| pane.update(cx, |pane, cx| pane.retry_terminal(window, cx)));
    pump_until(cx, "explicit replacement", |cx| {
        cx.read(|app| pane.read(app).is_running())
            && fs::read_to_string(host.root.path().join("starts"))
                .ok()
                .as_deref()
                == Some("start\nstart\n")
    });
    runtime
        .terminate_many_confirmed(vec![original.session_id])
        .unwrap();
    pump_until(cx, "replacement closed", |cx| {
        cx.read(|app| matches!(pane.read(app).lifecycle, PaneLifecycle::Lost { .. }))
    });
    controller.shutdown_client();
    runtime.shutdown_client();
}
