//! A mobile session against a real Host over P2P: pair, connect, observe a
//! terminal without disturbing the desktop, then take control and type.
//! The Host fixture follows `yttt-client-core/tests/p2p_pairing.rs`.

use std::{
    fs,
    io::{Read as _, Write as _},
    path::Path,
    sync::{Arc, Mutex},
    time::Duration,
};

use yttt_client_core::{ClientCore, ConnectionState};
use yttt_core::model::{
    host_workspace::{ProjectPanel, WorkspaceIntent},
    ids::{ClientInstanceId, ProfileId, ProjectId, TerminalSessionId},
    layout::ProjectLayout,
    project::{ProjectDescriptor, ProjectLocation},
};
use yttt_mobile_core::account;
use yttt_mobile_core::files::{FileKind, SaveOutcome};
use yttt_mobile_core::keys::TerminalKey;
use yttt_mobile_core::overview::{AgentState, Overview, WaitingKind};
use yttt_mobile_core::session::{
    LinkPath, LinkState, MobileError, MobileSession, PairedHostRecord, RelayChoice, pair,
};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    p2p::{DevicePreset, PairingCode},
    project::ProjectRequest,
    remote_access::{PairingState, RemoteAccessRequest, RemoteAccessResponse, RemoteAccessStatus},
    terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalLeaseMode, TerminalSpawnSpec},
    workspace::{WorkspaceId, WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse},
};
use yttt_relay::{NodeConfig, OpenConfig, RelayNode, open_account_id};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};
use yttt_transport_iroh::endpoint_key;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

const TOKEN: [u8; 32] = [0x6c; 32];
const PROFILE: &str = "mobile-session";
const SECRET: [u8; 32] = [41; 32];

fn bootstrap(root: &Path) -> yttt_host::HostBootstrap {
    let runtime_root = root.join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    let auth_token_file = runtime_root.join("auth-token");
    fs::write(&auth_token_file, TOKEN).unwrap();
    #[cfg(unix)]
    {
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    yttt_host::HostBootstrap {
        profile_id: ProfileId::new(PROFILE),
        runtime_root,
        state_root: root.join("state"),
        config_root: root.join("state/config"),
        project_config: WorkspaceProjectConfig::Project,
        auth_token_file,
        ssh_host_keys_file: root.join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.mobile-session".to_string(),
        build: BuildIdentity {
            product_version: "0.5.0".to_string(),
            build_fingerprint: "mobile-session".to_string(),
            resource_compatibility: "mobile-session-v1".to_string(),
        },
        lifetime: yttt_host::HostLifetime::Independent,
    }
}

/// Starts a Host with P2P on, through `relay_url` when given, and returns its
/// local administrator, which plays the desktop.
async fn start_host(root: &Path, relay_url: Option<String>) -> ClientCore {
    start_host_with_connector(root, relay_url).await.0
}

/// Like `start_host`, also returning the Host's local connector, for a phone
/// whose link a test cuts.
async fn start_host_with_connector(
    root: &Path,
    relay_url: Option<String>,
) -> (ClientCore, yttt_transport::MemoryConnector) {
    let bootstrap = bootstrap(root);
    let (listener, connector) = memory_pair();
    let ready = bootstrap.ready_file();
    let build = bootstrap.build.clone();
    tokio::spawn(yttt_host::run_with_remote_work(bootstrap, || async move {
        Ok::<_, yttt_transport::TransportError>((listener, None))
    }));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Host ready timeout");
    let local = connector.clone();
    let admin = ClientCore::connect(
        connector,
        ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build,
            profile_id: ProfileId::new(PROFILE),
            client_instance_id: ClientInstanceId::new("desktop"),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: yttt_protocol::ConnectionChannel::Session,
        },
        AuthToken::from_bytes(TOKEN),
    )
    .await
    .unwrap();
    remote_access(
        &admin,
        RemoteAccessRequest::SetP2p {
            enabled: true,
            relay_url,
        },
    )
    .await;
    (admin, local)
}

async fn remote_access(admin: &ClientCore, request: RemoteAccessRequest) -> RemoteAccessResponse {
    match admin.request(Request::RemoteAccess(request)).await.unwrap() {
        Response::RemoteAccess(response) => response,
        other => panic!("unexpected response {other:?}"),
    }
}

async fn status(admin: &ClientCore) -> RemoteAccessStatus {
    match remote_access(admin, RemoteAccessRequest::Status).await {
        RemoteAccessResponse::Status(status) => status,
        other => panic!("unexpected response {other:?}"),
    }
}

/// Pairs the device the way the app does, the desktop user confirming.
async fn pair_device(admin: &ClientCore, relay: RelayChoice) -> PairedHostRecord {
    let RemoteAccessResponse::PairingInvitation { code, .. } =
        remote_access(admin, RemoteAccessRequest::StartPairing).await
    else {
        panic!("expected an invitation");
    };
    assert!(PairingCode::decode(&code.0).is_ok());
    let (shown, shown_rx) = tokio::sync::oneshot::channel();
    let text = code.0.clone();
    let pairing = tokio::spawn(async move {
        pair(&text, "phone", SECRET, relay, None, move |short_code| {
            let _ = shown.send(short_code);
        })
        .await
    });
    let (endpoint_id, short_code) = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(PairingState::AwaitingConfirmation {
                endpoint_id,
                short_code,
                ..
            }) = status(admin).await.p2p.pairing
            {
                return (endpoint_id, short_code);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the device did not ask to pair");
    assert_eq!(shown_rx.await.unwrap(), short_code);
    remote_access(
        admin,
        RemoteAccessRequest::ConfirmPairing {
            endpoint_id,
            preset: DevicePreset::Full,
        },
    )
    .await;
    pairing.await.unwrap().unwrap()
}

/// The desktop opens a project and starts a shell-like terminal in it for
/// each of `names`.
async fn desktop_terminals(
    admin: &ClientCore,
    root: &Path,
    names: &[&str],
) -> Vec<TerminalSessionId> {
    desktop_terminals_running(
        admin,
        root,
        names,
        "printf 'desktop-ready'; IFS= read -r line; printf '\\ngot:%s\\n' \"$line\"; sleep 30",
    )
    .await
}

/// Like `desktop_terminals`, each running `script` under `/bin/sh -c`.
async fn desktop_terminals_running(
    admin: &ClientCore,
    root: &Path,
    names: &[&str],
    script: &str,
) -> Vec<TerminalSessionId> {
    let project_root = root.join("project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("project");
    admin
        .request(Request::Project(ProjectRequest::Register {
            view_id: "desktop-view".to_string(),
            project_id: project_id.clone(),
            root: yttt_protocol::HostPath::from_path(&project_root).unwrap(),
        }))
        .await
        .unwrap();
    let mut terminals = Vec::new();
    for name in names {
        let session_id = TerminalSessionId::new(*name);
        let Response::TerminalSpawned { .. } = admin
            .request(Request::SpawnTerminal {
                spec: TerminalSpawnSpec {
                    session_id: session_id.clone(),
                    project_id: project_id.clone(),
                    cwd: yttt_protocol::ProjectRelativePath::root(),
                    execution: TerminalExecutionSpec::Command {
                        shell: "/bin/sh".to_string(),
                        program: "/bin/sh".to_string(),
                        args: vec!["-c".to_string(), script.to_string()],
                        return_to_shell: false,
                    },
                    geometry: TerminalGeometry {
                        cols: 100,
                        rows: 30,
                        cell_width: 8,
                        cell_height: 16,
                    },
                    geometry_epoch: 1,
                    query_palette: Vec::new(),
                    palette_revision: 1,
                    scrollback_limit: 1_000,
                    environment: Vec::new(),
                    removed_environment: Vec::new(),
                },
                start_id: format!("mobile-session-start-{name}"),
                expected_host_epoch: admin.host_epoch().unwrap(),
            })
            .await
            .unwrap()
        else {
            panic!("unexpected spawn response");
        };
        terminals.push(session_id);
    }
    terminals
}

/// Drains until the accumulated repaints contain `needle`.
async fn drain_until(session: &MobileSession, id: &str, needle: &str) -> Vec<u8> {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(bytes) = session.drain(id) {
                seen.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&seen).contains(needle) {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "never saw {needle:?} in {:?}",
            String::from_utf8_lossy(&seen)
        )
    });
    seen
}

async fn controller(admin: &ClientCore) -> Option<ClientInstanceId> {
    match admin
        .request(Request::ReadTerminalController)
        .await
        .unwrap()
    {
        Response::TerminalController(controller) => controller.controller,
        other => panic!("unexpected response {other:?}"),
    }
}

/// Whether the desktop holds `id`'s Interactive lease: asking again for one
/// it holds returns it, and one another Client took is refused.
async fn desktop_holds(admin: &ClientCore, id: &TerminalSessionId) -> bool {
    matches!(
        admin
            .request(Request::AcquireTerminalLease {
                session_id: id.clone(),
                mode: TerminalLeaseMode::Interactive,
            })
            .await,
        Ok(Response::TerminalLease(lease)) if lease.mode == TerminalLeaseMode::Interactive
    )
}

async fn geometry(admin: &ClientCore, id: &TerminalSessionId) -> TerminalGeometry {
    let Response::Resources(catalog) = admin.request(Request::ListResources).await.unwrap() else {
        panic!("expected resources");
    };
    catalog
        .terminals
        .into_iter()
        .find(|terminal| &terminal.session_id == id)
        .expect("terminal in catalog")
        .geometry
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_observes_without_disturbing_the_desktop_then_takes_control() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path(), None).await;
    let record = pair_device(&admin, RelayChoice::Off).await;
    assert_eq!(
        PairedHostRecord::from_bytes(&record.to_bytes()).unwrap(),
        record
    );

    let terminals = desktop_terminals(
        &admin,
        temp.path(),
        &["project:tab:pane", "project:tab:other"],
    )
    .await;
    let terminal = terminals[0].clone();
    let other = terminals[1].clone();
    // Asked to use only a relay the Host does not have, the phone says so at
    // once rather than after the connect timeout.
    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(2),
            MobileSession::connect(&record, SECRET, RelayChoice::OnlyFromHost, None, "phone"),
        )
        .await
        .expect("an immediate answer"),
        Err(MobileError::Transport(_))
    ));
    let id = terminal.to_string();
    let phone = MobileSession::connect(&record, SECRET, RelayChoice::Off, None, "phone")
        .await
        .unwrap();
    assert!(
        phone
            .terminals()
            .await
            .unwrap()
            .iter()
            .any(|info| info.session_id == id)
    );

    phone.attach(&id).await.unwrap();
    let first = drain_until(&phone, &id, "desktop-ready").await;
    assert!(
        String::from_utf8_lossy(&first).contains("\x1b[2J"),
        "first drain repaints all"
    );
    assert_eq!(phone.drain(&id), None, "nothing changed since");
    // A view that cleared itself, as when it changes size, asks for it all.
    phone.repaint(&id);
    assert!(String::from_utf8_lossy(&phone.drain(&id).expect("a repaint")).contains("\x1b[2J"));
    assert_eq!(
        controller(&admin).await,
        Some(ClientInstanceId::new("desktop"))
    );
    assert_eq!(
        (
            geometry(&admin, &terminal).await.cols,
            geometry(&admin, &terminal).await.rows
        ),
        (100, 30)
    );
    assert!(!phone.in_control(&id));
    assert!(phone.send_input(&id, b"x".to_vec()).is_err());

    // Taking control takes this one terminal, at the phone's size, before
    // any key goes out. The display keeps draining meanwhile, as on a phone:
    // repainting the view at its old size must not make the keys stale.
    {
        let take = phone.take_control(&id, 50, 20);
        tokio::pin!(take);
        loop {
            tokio::select! {
                taken = &mut take => break taken.unwrap(),
                _ = tokio::time::sleep(Duration::from_millis(1)) => {
                    phone.drain(&id);
                }
            }
        }
    }
    assert!(phone.in_control(&id));
    assert_eq!(
        (
            geometry(&admin, &terminal).await.cols,
            geometry(&admin, &terminal).await.rows
        ),
        (50, 20)
    );
    assert_eq!(
        controller(&admin).await,
        Some(ClientInstanceId::new("desktop"))
    );
    assert!(desktop_holds(&admin, &other).await);
    assert!(!desktop_holds(&admin, &terminal).await);
    phone.send_input(&id, b"hello\r".to_vec()).unwrap();
    drain_until(&phone, &id, "got:hello").await;

    // Giving it back hands it to the desktop.
    phone.give_back(Vec::new()).await.unwrap();
    assert!(!phone.in_control(&id));
    assert!(phone.send_input(&id, b"x".to_vec()).is_err());
    assert!(desktop_holds(&admin, &terminal).await);

    // The desktop taking it back ends the phone's control.
    phone.take_control(&id, 50, 20).await.unwrap();
    admin
        .request(Request::TakeTerminal {
            session_id: terminal.clone(),
        })
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while phone.in_control(&id) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the phone saw the desktop take the terminal back");

    let stats = phone.drain_stats();
    assert!(stats.frames > 0 && stats.bytes > 0);
    let first_id = phone.client_instance_id().to_string();
    assert!(first_id.starts_with("mobile-client-"));
    phone.close().await;

    // A restarted app connects again at once with the same record.
    let again = MobileSession::connect(&record, SECRET, RelayChoice::Off, None, "phone")
        .await
        .unwrap();
    assert_ne!(again.client_instance_id(), first_id);
    assert!(
        again
            .terminals()
            .await
            .unwrap()
            .iter()
            .any(|info| info.session_id == id)
    );
    again.close().await;
}

/// Waits until `probe` yields a value.
async fn eventually<T>(what: &str, mut probe: impl AsyncFnMut() -> Option<T>) -> T {
    tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(value) = probe().await {
                return value;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_works_through_the_relay_alone() {
    let mut config = NodeConfig::local("mobile-relay", "127.0.0.1:0".parse().unwrap(), &[]);
    config.open = Some(OpenConfig {
        rate_bps: 50_000_000,
        burst_bytes: 10_000_000,
    });
    let relay = RelayNode::spawn(config).await.unwrap();
    let relay_url = format!("http://{}", relay.addr());

    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path(), Some(relay_url.clone())).await;
    let host_key = status(&admin).await.p2p.endpoint_id.unwrap().0;
    eventually("the Host on the relay", async || {
        relay
            .account(open_account_id(&host_key))
            .filter(|account| account.connections > 0)
    })
    .await;

    let record = pair_device(&admin, RelayChoice::OnlyFromHost).await;
    assert_eq!(record.host.relay_url.as_deref(), Some(relay_url.as_str()));
    let terminal = desktop_terminals(&admin, temp.path(), &["project:tab:relay"]).await;
    let id = terminal[0].to_string();
    let phone = MobileSession::connect(&record, SECRET, RelayChoice::OnlyFromHost, None, "phone")
        .await
        .unwrap();
    phone.attach(&id).await.unwrap();
    drain_until(&phone, &id, "desktop-ready").await;
    phone.take_control(&id, 50, 20).await.unwrap();
    phone.send_input(&id, b"relayed\r".to_vec()).unwrap();
    drain_until(&phone, &id, "got:relayed").await;

    // Long enough for hole punching, which on one machine succeeds within
    // moments: with no direct path of its own the phone stays on the relay.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(phone.path().await, LinkPath::Relay);
    // And what the phone sent went through it.
    let phone_key = endpoint_key(&account::host_key(&SECRET, &record.host.host)).0;
    let relayed = relay
        .account(open_account_id(&phone_key))
        .expect("the phone on the relay")
        .relayed_bytes;
    assert!(relayed > 0, "the phone sent nothing through the relay");

    // A network change re-checks the link at once; a healthy session answers
    // the probe and is kept.
    let reconnects = phone.client().diagnostics().reconnects;
    tokio::time::timeout(Duration::from_secs(1), phone.network_changed())
        .await
        .expect("network_changed returns at once");
    tokio::time::sleep(Duration::from_secs(6)).await;
    assert_eq!(phone.client().diagnostics().reconnects, reconnects);
    assert!(matches!(
        phone.client().state(),
        ConnectionState::Ready { .. }
    ));
    assert_eq!(phone.path().await, LinkPath::Relay);
    phone.close().await;
    // After closing it is a no-op.
    tokio::time::timeout(Duration::from_secs(1), phone.network_changed())
        .await
        .expect("network_changed after close returns at once");
    relay.shutdown().await;
}

/// Sends one Agent hook event the way an Agent CLI's hook does.
fn post_hook(environment: &[String], event: &str, payload: &str) {
    let endpoint = environment[0].strip_prefix("http://").unwrap();
    let body = format!(r#"{{"event":"{event}","payload":{payload}}}"#);
    let mut stream = std::net::TcpStream::connect(endpoint).unwrap();
    write!(
        stream,
        "POST /hook/codex HTTP/1.1\r\nContent-Length: {}\r\nx-yttt-agent-hook-token: {}\r\nx-yttt-agent-hook-scope: {}\r\nConnection: close\r\n\r\n{body}",
        body.len(),
        environment[1],
        environment[2],
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 204"), "{event}: {response}");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_overview_follows_an_agent_into_waiting() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path(), None).await;
    let record = pair_device(&admin, RelayChoice::Off).await;

    // The desktop opens a project whose Agent pane the Host starts; the pane
    // writes the hook address the Host gave it, as an Agent CLI would read it.
    let project_root = temp.path().join("web");
    fs::create_dir_all(&project_root).unwrap();
    let environment_file = temp.path().join("agent-environment");
    let layout: ProjectLayout = toml::from_str(&format!(
        r#"
        [project]
        name = "Website"

        [[tabs]]
        id = "agents"
        title = "Agents"
        startup = "eager"
        layout = {{ type = "pane", id = "codex", title = "Codex", command = "printf '%s\\n%s\\n%s\\n' \"$YTTT_AGENT_HOOK_ENDPOINT\" \"$YTTT_AGENT_HOOK_TOKEN\" \"$YTTT_AGENT_HOOK_SCOPE\" > '{}'; sleep 60" }}
        "#,
        environment_file.display()
    ))
    .unwrap();
    let Response::Workspace(WorkspaceResponse::Applied { .. }) = admin
        .request(Request::Workspace(WorkspaceRequest::Apply {
            workspace_id: WorkspaceId::new("mobile-overview").unwrap(),
            intent: WorkspaceIntent::OpenProject {
                descriptor: ProjectDescriptor::new(
                    ProjectId::new("web"),
                    ProjectLocation::local(&project_root),
                ),
                layout,
                panel: ProjectPanel {
                    visible: true,
                    width: 240.0,
                },
            },
            expected_revision: None,
        }))
        .await
        .unwrap()
    else {
        panic!("workspace apply");
    };
    // The desktop window shows it, which makes it the active workspace.
    admin
        .request(Request::Workspace(WorkspaceRequest::SetActive {
            workspace_id: WorkspaceId::new("mobile-overview").unwrap(),
        }))
        .await
        .unwrap();

    let phone = MobileSession::connect(&record, SECRET, RelayChoice::Off, None, "phone")
        .await
        .unwrap();
    assert!(phone.open_active_workspace().await.unwrap().is_some());
    let seen = Arc::new(Mutex::new(Vec::<Overview>::new()));
    phone.watch_overview({
        let seen = seen.clone();
        move |overview| seen.lock().unwrap().push(overview)
    });
    let first = eventually("the first overview", async || {
        seen.lock().unwrap().first().cloned()
    })
    .await;
    assert_eq!(first.projects.len(), 1);
    assert_eq!(first.projects[0].name, "Website");
    assert_eq!(first.projects[0].tabs[0].title, "Agents");

    let environment = eventually("the Agent pane's hook address", async || {
        let contents = fs::read_to_string(&environment_file).ok()?;
        let values = contents.lines().map(str::to_string).collect::<Vec<_>>();
        (values.len() == 3 && values.iter().all(|value| !value.is_empty())).then_some(values)
    })
    .await;
    let before = seen.lock().unwrap().len();
    post_hook(&environment, "SessionStart", r#"{"session_id":"codex-1"}"#);
    post_hook(
        &environment,
        "UserPromptSubmit",
        r#"{"session_id":"codex-1","prompt":"Add a login page"}"#,
    );
    post_hook(
        &environment,
        "PermissionRequest",
        r#"{"session_id":"codex-1","tool_name":"Bash","tool_input":{"command":"npm install"}}"#,
    );

    let agent = eventually("the Agent waiting in the overview", async || {
        seen.lock()
            .unwrap()
            .last()
            .and_then(|overview| overview.agents.first().cloned())
            .filter(|agent| agent.state == AgentState::Waiting)
    })
    .await;
    assert_eq!(agent.session_id, "web:agents:codex");
    assert_eq!(
        (agent.project_id.as_str(), agent.tab_id.as_str()),
        ("web", "agents")
    );
    assert_eq!(agent.provider, "codex");
    assert_eq!(agent.waiting, Some(WaitingKind::Approval));
    assert_eq!(agent.task.as_deref(), Some("Add a login page"));
    // Three changes in quick succession reach the app as a few snapshots.
    tokio::time::sleep(Duration::from_millis(500)).await;
    let pushed = seen.lock().unwrap().len() - before;
    assert!(pushed <= 3, "{pushed} overviews for three quick changes");
    // Over the network the Host leaves boards out.
    assert!(
        phone
            .client()
            .agent_snapshots()
            .iter()
            .all(|update| update.board_omitted)
    );
    assert_eq!(
        phone.overview().agents,
        seen.lock().unwrap().last().unwrap().agents
    );
    phone.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_reads_edits_and_saves_a_project_file() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path(), None).await;
    let record = pair_device(&admin, RelayChoice::Off).await;

    let project_root = temp.path().join("site");
    fs::create_dir_all(project_root.join("src")).unwrap();
    fs::write(project_root.join("README.md"), "hello\n").unwrap();
    fs::write(project_root.join("src/app.js"), "console.log(1)\n").unwrap();
    let layout: ProjectLayout = toml::from_str(
        r#"
        [project]
        name = "Site"

        [[tabs]]
        id = "shell"
        title = "Shell"
        layout = { type = "pane", id = "1", title = "zsh", command = "sleep 60" }
        "#,
    )
    .unwrap();
    let workspace_id = WorkspaceId::new("mobile-files").unwrap();
    admin
        .request(Request::Workspace(WorkspaceRequest::Apply {
            workspace_id: workspace_id.clone(),
            intent: WorkspaceIntent::OpenProject {
                descriptor: ProjectDescriptor::new(
                    ProjectId::new("site"),
                    ProjectLocation::local(&project_root),
                ),
                layout,
                panel: ProjectPanel {
                    visible: true,
                    width: 240.0,
                },
            },
            expected_revision: None,
        }))
        .await
        .unwrap();
    admin
        .request(Request::Workspace(WorkspaceRequest::SetActive {
            workspace_id,
        }))
        .await
        .unwrap();

    let phone = MobileSession::connect(&record, SECRET, RelayChoice::Off, None, "phone")
        .await
        .unwrap();
    phone.open_active_workspace().await.unwrap();

    // The tree, a directory at a time.
    let root = phone.list_directory("site", "").await.unwrap();
    let names = root
        .iter()
        .map(|entry| (entry.name.as_str(), entry.path.as_str(), entry.kind))
        .collect::<Vec<_>>();
    assert!(
        names.contains(&("README.md", "README.md", FileKind::File)),
        "{names:?}"
    );
    assert!(
        names.contains(&("src", "src", FileKind::Directory)),
        "{names:?}"
    );
    let src = phone.list_directory("site", "src").await.unwrap();
    assert_eq!(src[0].path, "src/app.js");

    // Open, edit, save.
    let opened = phone.open_file("site", "README.md").await.unwrap();
    assert_eq!(opened.text, "hello\n");
    assert_eq!(
        phone
            .save_file("site", "README.md", "hello from the phone\n".into(), false)
            .await
            .unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(
        fs::read_to_string(project_root.join("README.md")).unwrap(),
        "hello from the phone\n"
    );

    // The desktop changed it meanwhile: saving again conflicts, forcing wins.
    fs::write(project_root.join("README.md"), "edited on the desktop\n").unwrap();
    assert_eq!(
        phone
            .save_file("site", "README.md", "phone again\n".into(), false)
            .await
            .unwrap(),
        SaveOutcome::Conflict
    );
    assert_eq!(
        fs::read_to_string(project_root.join("README.md")).unwrap(),
        "edited on the desktop\n"
    );
    assert_eq!(
        phone
            .save_file("site", "README.md", "phone again\n".into(), true)
            .await
            .unwrap(),
        SaveOutcome::Saved
    );
    assert_eq!(
        fs::read_to_string(project_root.join("README.md")).unwrap(),
        "phone again\n"
    );
    // Reloading takes the file as it is now; saving then does not conflict.
    assert_eq!(
        phone.open_file("site", "README.md").await.unwrap().text,
        "phone again\n"
    );
    assert_eq!(
        phone
            .save_file("site", "README.md", "last\n".into(), false)
            .await
            .unwrap(),
        SaveOutcome::Saved
    );
    // Saving a file never opened is refused.
    assert!(
        phone
            .save_file("site", "src/app.js", "x".into(), false)
            .await
            .is_err()
    );

    // No Agent has worked here.
    assert!(phone.agent_sessions("site").await.unwrap().is_empty());
    phone.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn keys_reach_the_program_as_it_expects_them() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path(), None).await;
    let record = pair_device(&admin, RelayChoice::Off).await;
    // Reads a key raw and prints its bytes, then turns on application cursor
    // keys (as vim and full-screen CLIs do) and does it again.
    let terminal = desktop_terminals_running(
        &admin,
        temp.path(),
        &["project:tab:keys"],
        concat!(
            "stty raw -echo; printf 'ready\\r\\n'; ",
            "a=$(dd bs=1 count=3 2>/dev/null | od -An -tx1 | tr -s ' '); printf 'first:%s\\r\\n' \"$a\"; ",
            "printf '\\033[?1h'; ",
            "b=$(dd bs=1 count=3 2>/dev/null | od -An -tx1 | tr -s ' '); printf 'second:%s\\r\\n' \"$b\"; ",
            "sleep 30"
        ),
    )
    .await;
    let id = terminal[0].to_string();
    let phone = MobileSession::connect(&record, SECRET, RelayChoice::Off, None, "phone")
        .await
        .unwrap();
    phone.attach(&id).await.unwrap();
    drain_until(&phone, &id, "ready").await;
    // Only while in control.
    assert!(phone.send_key(&id, TerminalKey::Up).is_err());
    phone.take_control(&id, 60, 20).await.unwrap();
    assert!(!phone.app_cursor(&id));

    phone.send_key(&id, TerminalKey::Up).unwrap();
    drain_until(&phone, &id, "first: 1b 5b 41").await;
    eventually("application cursor keys", async || {
        phone.app_cursor(&id).then_some(())
    })
    .await;
    phone.send_key(&id, TerminalKey::Up).unwrap();
    drain_until(&phone, &id, "second: 1b 4f 41").await;
    phone.close().await;
}

/// A link a test cuts: while cut, live streams close and new connects hang,
/// as on a phone that lost its network.
#[derive(Clone)]
struct Cuttable {
    inner: yttt_transport::MemoryConnector,
    cut: tokio::sync::watch::Sender<bool>,
}

impl yttt_transport::TransportConnector for Cuttable {
    fn connect(
        &self,
    ) -> yttt_transport::BoxFuture<
        '_,
        Result<yttt_transport::TransportStream, yttt_transport::TransportError>,
    > {
        Box::pin(async move {
            let mut cut = self.cut.subscribe();
            cut.wait_for(|cut| !*cut).await.unwrap();
            let mut stream = self.inner.connect().await?;
            let (client, mut peer) = tokio::io::duplex(64 * 1024);
            tokio::spawn(async move {
                tokio::select! {
                    _ = tokio::io::copy_bidirectional(&mut stream, &mut peer) => {}
                    _ = cut.wait_for(|cut| *cut) => {}
                }
            });
            Ok(Box::new(client) as yttt_transport::TransportStream)
        })
    }
}

async fn wait_for_link(phone: &MobileSession, what: &str, wanted: impl Fn(&LinkState) -> bool) {
    tokio::time::timeout(Duration::from_secs(20), async {
        while !wanted(&phone.link_state()) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("the link never became {what}: {:?}", phone.link_state()));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_terminal_opened_while_the_link_is_down_shows_once_it_is_back() {
    let temp = tempfile::tempdir().unwrap();
    let (admin, local) = start_host_with_connector(temp.path(), None).await;
    let record = pair_device(&admin, RelayChoice::Off).await;
    let terminal =
        desktop_terminals(&admin, temp.path(), &["project:tab:pane"]).await[0].to_string();
    let (cut, _) = tokio::sync::watch::channel(false);
    let phone = MobileSession::connect_with(
        Cuttable {
            inner: local,
            cut: cut.clone(),
        },
        &record,
        AuthToken::from_bytes(TOKEN),
    )
    .await
    .unwrap();
    assert_eq!(phone.link_state(), LinkState::Ready);

    cut.send_replace(true);
    wait_for_link(&phone, "reconnecting", |state| {
        matches!(state, LinkState::Reconnecting { .. })
    })
    .await;
    // The screen opens now: the attach waits for the link instead of failing.
    phone.attach(&terminal).await.unwrap();
    cut.send_replace(false);
    phone.retry_now();
    wait_for_link(&phone, "ready", |state| *state == LinkState::Ready).await;
    drain_until(&phone, &terminal, "desktop-ready").await;
    phone.close().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn control_and_the_workspace_survive_a_short_drop() {
    let temp = tempfile::tempdir().unwrap();
    let (admin, local) = start_host_with_connector(temp.path(), None).await;
    let record = pair_device(&admin, RelayChoice::Off).await;
    let terminal =
        desktop_terminals(&admin, temp.path(), &["project:tab:pane"]).await[0].to_string();
    let (cut, _) = tokio::sync::watch::channel(false);
    let phone = MobileSession::connect_with(
        Cuttable {
            inner: local,
            cut: cut.clone(),
        },
        &record,
        AuthToken::from_bytes(TOKEN),
    )
    .await
    .unwrap();
    phone.attach(&terminal).await.unwrap();
    drain_until(&phone, &terminal, "desktop-ready").await;
    phone.take_control(&terminal, 50, 20).await.unwrap();
    assert!(phone.in_control(&terminal));

    cut.send_replace(true);
    wait_for_link(&phone, "reconnecting", |state| {
        matches!(state, LinkState::Reconnecting { .. })
    })
    .await;
    cut.send_replace(false);
    phone.retry_now();
    wait_for_link(&phone, "ready", |state| *state == LinkState::Ready).await;
    // The Host kept this phone's control through the drop; so does the phone.
    tokio::time::timeout(Duration::from_secs(10), async {
        while !phone.in_control(&terminal) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the phone lost control over a short drop");
    tokio::time::timeout(Duration::from_secs(10), async {
        while phone.send_input(&terminal, b"again\r".to_vec()).is_err() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("typing after the drop");
    drain_until(&phone, &terminal, "got:again").await;
    phone.close().await;
}
