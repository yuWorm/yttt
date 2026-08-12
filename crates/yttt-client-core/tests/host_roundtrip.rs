#![cfg(unix)]

use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpStream,
    os::unix::ffi::OsStrExt as _,
    os::unix::fs::PermissionsExt as _,
    process::Command,
    time::Duration,
};

use tempfile::TempDir;

use yttt_client_core::{ClientCore, ClientCoreError, ClientEvent, ConnectionState};
use yttt_core::model::ids::{
    ClientInstanceId, PaneId, ProfileId, ProjectId, TabId, TerminalSessionId,
};
use yttt_host::{HostBootstrap, run};
use yttt_protocol::{
    FailureCode, PROTOCOL_VERSION, ProtocolRange, Request, Response,
    project::{
        PlatformPath, ProjectFileState, ProjectRequest, ProjectResponse, ProjectSaveMode,
        ProjectSaveResult,
    },
    terminal::{
        AttachTerminal, TerminalExecutionSpec, TerminalGeometry, TerminalInput, TerminalSpawnSpec,
        TerminationMode,
    },
};
use yttt_transport_local::{AuthToken, ClientIdentity};

struct RunningHost {
    _temp: TempDir,
    bootstrap: HostBootstrap,
    token: [u8; 32],
    task: tokio::task::JoinHandle<Result<(), yttt_host::HostError>>,
}

impl RunningHost {
    async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let runtime_root = temp.path().join("runtime");
        fs::create_dir_all(&runtime_root).unwrap();
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        let auth_token_file = runtime_root.join("auth-token");
        let token = [0x5a; 32];
        fs::write(&auth_token_file, token).unwrap();
        fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
        let bootstrap = HostBootstrap {
            profile_id: ProfileId::new("integration"),
            runtime_root,
            auth_token_file,
            ssh_host_keys_file: temp.path().join("ssh-host-keys.toml"),
            credential_namespace: "dev.yttt.ssh.integration-test".to_string(),
            build_id: "integration-build".to_string(),
        };
        let task = tokio::spawn(run(bootstrap.clone()));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !bootstrap.ready_file().exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Host ready timeout");
        Self {
            _temp: temp,
            bootstrap,
            token,
            task,
        }
    }

    async fn client(&self, id: &str) -> ClientCore {
        ClientCore::connect(
            self.bootstrap.endpoint(),
            ClientIdentity {
                supported: ProtocolRange::exact(PROTOCOL_VERSION),
                build_id: self.bootstrap.build_id.clone(),
                profile_id: self.bootstrap.profile_id.clone(),
                client_instance_id: ClientInstanceId::new(id),
                host_epoch_hint: None,
            },
            AuthToken::from_bytes(self.token),
        )
        .await
        .unwrap()
    }
}

fn geometry() -> TerminalGeometry {
    TerminalGeometry {
        cols: 80,
        rows: 24,
        cell_width: 8,
        cell_height: 16,
    }
}

fn spawn_spec() -> TerminalSpawnSpec {
    TerminalSpawnSpec {
        session_id: TerminalSessionId::new("host-owned"),
        project_id: ProjectId::new("project"),
        tab_id: TabId::new("tab"),
        pane_id: PaneId::new("pane"),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        execution: TerminalExecutionSpec::Command {
            shell: "/bin/sh".to_string(),
            program: "/bin/sh".to_string(),
            args: vec![
                "-lc".to_string(),
                "printf client-core-ready; sleep 30".to_string(),
            ],
            return_to_shell: false,
        },
        geometry: geometry(),
        geometry_epoch: 1,
        scrollback_limit: 1_000,
        environment: Vec::new(),
        removed_environment: Vec::new(),
    }
}

fn viewport_text(viewport: &yttt_protocol::terminal::SemanticViewport) -> String {
    viewport
        .rows
        .iter()
        .flat_map(|row| row.spans.iter().map(|span| span.text.as_str()))
        .collect::<Vec<_>>()
        .join("\n")
}

async fn wait_for_mirror(client: &ClientCore, expected: &str) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .terminal_snapshot(&TerminalSessionId::new("host-owned"))
                .is_some_and(|viewport| viewport_text(&viewport).contains(expected))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal mirror timeout");
}

#[tokio::test]
async fn host_terminal_survives_client_disconnect_and_reattaches_with_a_checkpoint() {
    let host = RunningHost::start().await;
    let first = host.client("first-client").await;
    let spawned = first
        .request(Request::SpawnTerminal(spawn_spec()))
        .await
        .unwrap();
    let Response::TerminalSpawned { session_epoch, .. } = spawned else {
        panic!("unexpected spawn response: {spawned:?}");
    };
    wait_for_mirror(&first, "client-core-ready").await;
    drop(first);

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!host.task.is_finished());
    let second = host.client("second-client").await;
    wait_for_mirror(&second, "client-core-ready").await;

    let attached = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            match second
                .request(Request::AttachTerminal(AttachTerminal {
                    session_id: TerminalSessionId::new("host-owned"),
                    known_session_epoch: Some(session_epoch),
                    after_sequence: None,
                    geometry: geometry(),
                    geometry_epoch: 1,
                }))
                .await
            {
                Ok(response) => break response,
                Err(_) => tokio::time::sleep(Duration::from_millis(10)).await,
            }
        }
    })
    .await
    .expect("terminal lease handoff timeout");
    let Response::TerminalAttached { checkpoint, .. } = attached else {
        panic!("unexpected attach response: {attached:?}");
    };
    assert!(viewport_text(&checkpoint.viewport).contains("client-core-ready"));

    assert_eq!(
        second
            .request(Request::TerminateTerminal {
                session_id: TerminalSessionId::new("host-owned"),
                mode: TerminationMode::Terminate,
            })
            .await
            .unwrap(),
        Response::TerminalTerminated
    );
    assert_eq!(
        second
            .request(Request::AcknowledgeTerminalExit {
                session_id: TerminalSessionId::new("host-owned"),
                session_epoch,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
    assert_eq!(
        second.request(Request::DrainAndStop).await.unwrap(),
        Response::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn host_roundtrip_preserves_input_resize_scroll_environment_and_title() {
    let host = RunningHost::start().await;
    let client = host.client("interactive-client").await;
    let mut spec = spawn_spec();
    spec.session_id = TerminalSessionId::new("interactive");
    spec.pane_id = PaneId::new("interactive");
    spec.environment = vec![("YTTT_HOST_TEST".to_string(), "environment-ok".to_string())];
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            concat!(
                "printf '\\033]2;host-title\\007%s\\nready\\n' \"$YTTT_HOST_TEST\"; ",
                "IFS= read -r line; printf 'input:%s\\n' \"$line\"; sleep 1; ",
                "i=1; while [ \"$i\" -le 80 ]; do printf 'line-%s\\n' \"$i\"; i=$((i+1)); done; ",
                "sleep 30"
            )
            .to_string(),
        ],
        return_to_shell: false,
    };
    let spawned = client.request(Request::SpawnTerminal(spec)).await.unwrap();
    let Response::TerminalSpawned { .. } = spawned else {
        panic!("unexpected spawn response: {spawned:?}");
    };
    let session_id = TerminalSessionId::new("interactive");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| {
                    viewport_text(&viewport).contains("environment-ok")
                        && viewport.modes.title.as_deref() == Some("host-title")
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("environment and title mirror timeout");

    assert_eq!(
        client
            .request(Request::TerminalInput(TerminalInput {
                session_id: session_id.clone(),
                client_sequence: 1,
                bytes: b"roundtrip\r".to_vec(),
            }))
            .await
            .unwrap(),
        Response::TerminalInputAccepted { client_sequence: 1 }
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport_text(&viewport).contains("input:roundtrip"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal input mirror timeout");

    let resized = TerminalGeometry {
        cols: 100,
        rows: 20,
        cell_width: 0,
        cell_height: 0,
    };
    assert_eq!(
        client
            .request(Request::ResizeTerminal {
                session_id: session_id.clone(),
                geometry: resized,
                geometry_epoch: 2,
            })
            .await
            .unwrap(),
        Response::TerminalResized { geometry_epoch: 2 }
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport.geometry == resized && viewport.history_size > 0)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal resize mirror timeout");

    assert_eq!(
        client
            .request(Request::ScrollTerminal {
                session_id: session_id.clone(),
                display_offset: 5,
            })
            .await
            .unwrap(),
        Response::TerminalScrolled { display_offset: 5 }
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport.display_offset == 5)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal scroll mirror timeout");

    assert_eq!(
        client
            .request(Request::TerminateMany {
                session_ids: vec![session_id.clone()],
            })
            .await
            .unwrap(),
        Response::TerminalsTerminated { terminated: 1 }
    );
    let Response::Resources(resources) = client.request(Request::ListResources).await.unwrap()
    else {
        panic!("expected resource catalog");
    };
    assert!(
        resources
            .terminals
            .iter()
            .all(|terminal| terminal.session_id != session_id)
    );
    assert_eq!(
        client.request(Request::DrainAndStop).await.unwrap(),
        Response::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn client_reconciles_stale_terminal_after_host_restart() {
    let host = RunningHost::start().await;
    let client = host.client("restart-client").await;
    let mut events = client.subscribe_events();
    let old_host_epoch = match client.state() {
        ConnectionState::Ready { host_epoch, .. } => host_epoch,
        state => panic!("expected ready client, got {state:?}"),
    };
    let session_id = spawn_spec().session_id.clone();
    assert!(matches!(
        client
            .request(Request::SpawnTerminal(spawn_spec()))
            .await
            .unwrap(),
        Response::TerminalSpawned { .. }
    ));
    wait_for_mirror(&client, "client-core-ready").await;
    assert!(client.known_terminal_ids().contains(&session_id));

    assert_eq!(
        client.request(Request::DrainAndStop).await.unwrap(),
        Response::Draining
    );
    let bootstrap = host.bootstrap.clone();
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("first Host shutdown timeout")
        .unwrap()
        .unwrap();
    assert!(
        client.terminal_snapshot(&session_id).is_some(),
        "checkpoint must remain available while the Host is down"
    );
    assert!(client.known_terminal_ids().contains(&session_id));

    let restarted = tokio::spawn(run(bootstrap.clone()));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !bootstrap.ready_file().exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("restarted Host ready timeout");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                client.state(),
                ConnectionState::Ready { host_epoch, .. } if host_epoch != old_host_epoch
            ) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("client reconnect timeout");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if matches!(
                events.recv().await,
                Ok(ClientEvent::TerminalUnavailable(unavailable)) if unavailable == session_id
            ) {
                break;
            }
        }
    })
    .await
    .expect("stale terminal reconciliation timeout");
    assert!(client.terminal_snapshot(&session_id).is_none());
    assert!(!client.known_terminal_ids().contains(&session_id));

    assert_eq!(
        client.request(Request::DrainAndStop).await.unwrap(),
        Response::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), restarted)
        .await
        .expect("restarted Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn host_owns_authenticated_agent_hook_ingress() {
    let host = RunningHost::start().await;
    let client = host.client("agent-hooks").await;
    let mut events = client.subscribe_events();
    let scope = yttt_protocol::agent::AgentHookScope {
        project_id: "project".to_string(),
        tab_id: "tab".to_string(),
        pane_id: "pane".to_string(),
        generation: 7,
    };
    let Response::AgentHookEnvironment(environment) = client
        .request(Request::AgentHookEnvironment(scope.clone()))
        .await
        .unwrap()
    else {
        panic!("unexpected Agent hook environment response");
    };
    let endpoint = environment.variables["YTTT_AGENT_HOOK_ENDPOINT"]
        .strip_prefix("http://")
        .unwrap();
    let token = &environment.variables["YTTT_AGENT_HOOK_TOKEN"];
    let encoded_scope = &environment.variables["YTTT_AGENT_HOOK_SCOPE"];
    let body = br#"{"event":"turn-start","payload":{"status":"working"}}"#;
    let mut stream = TcpStream::connect(endpoint).unwrap();
    write!(
        stream,
        "POST /hook/codex HTTP/1.1\r\nContent-Length: {}\r\nx-yttt-agent-hook-token: {}\r\nx-yttt-agent-hook-scope: {}\r\nConnection: close\r\n\r\n",
        body.len(),
        token,
        encoded_scope,
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();

    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");
    let event = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(ClientEvent::Server(event)) = events.recv().await
                && let yttt_protocol::ServerEvent::AgentHook(event) = event.body
            {
                break event;
            }
        }
    })
    .await
    .expect("Agent hook event timeout");
    assert_eq!(event.scope, scope);
    assert_eq!(event.source, "codex");
    assert_eq!(event.event, "turn-start");
    assert_eq!(event.payload_json, br#"{"status":"working"}"#);

    assert_eq!(
        client.request(Request::DrainAndStop).await.unwrap(),
        Response::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn host_owns_project_files_and_publishes_watcher_events() {
    let host = RunningHost::start().await;
    let project_root = host._temp.path().join("project");
    fs::create_dir_all(project_root.join("src")).unwrap();
    fs::write(project_root.join("notes.txt"), "before").unwrap();
    let client = host.client("project-client").await;
    let mut events = client.subscribe_events();
    let project_id = ProjectId::new("host-project");
    let path = |path: &std::path::Path| PlatformPath::Unix(path.as_os_str().as_bytes().to_vec());

    let Response::Project(ProjectResponse::Registered {
        registration_epoch,
        watch_error,
        ..
    }) = client
        .request(Request::Project(ProjectRequest::Register {
            project_id: project_id.clone(),
            root: path(&project_root),
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected project registration response");
    };
    assert_eq!(watch_error, None);

    let Response::Resources(resources) = client.request(Request::ListResources).await.unwrap()
    else {
        panic!("unexpected Host resource catalog response");
    };
    assert!(resources.projects.contains(&project_id));

    let Response::Project(ProjectResponse::File(file)) = client
        .request(Request::Project(ProjectRequest::ReadFile {
            project_id: project_id.clone(),
            relative_path: path(std::path::Path::new("notes.txt")),
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected project read response");
    };
    assert_eq!(file.text, "before");

    let Response::Project(ProjectResponse::Save(ProjectSaveResult::Saved(saved))) = client
        .request(Request::Project(ProjectRequest::SaveFile {
            project_id: project_id.clone(),
            relative_path: path(std::path::Path::new("notes.txt")),
            text: "after".to_string(),
            mode: ProjectSaveMode::Check(file.fingerprint),
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected project save response");
    };
    assert_eq!(
        fs::read_to_string(project_root.join("notes.txt")).unwrap(),
        "after"
    );

    fs::write(project_root.join("notes.txt"), "external").unwrap();
    let Response::Project(ProjectResponse::Save(ProjectSaveResult::Conflict(
        ProjectFileState::Present(current),
    ))) = client
        .request(Request::Project(ProjectRequest::SaveFile {
            project_id: project_id.clone(),
            relative_path: path(std::path::Path::new("notes.txt")),
            text: "stale".to_string(),
            mode: ProjectSaveMode::Check(saved),
        }))
        .await
        .unwrap()
    else {
        panic!("stale project save did not report a conflict");
    };
    assert_ne!(current.content_hash, 0);

    let change = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(ClientEvent::Server(event)) = events.recv().await
                && let yttt_protocol::ServerEvent::ProjectChanged(change) = event.body
                && change.project_id == project_id
                && change.registration_epoch == registration_epoch
            {
                break change;
            }
        }
    })
    .await
    .expect("project watcher event timeout");
    assert!(change.refresh_status);

    assert_eq!(
        client
            .request(Request::Project(ProjectRequest::Close {
                project_id: project_id.clone(),
                registration_epoch,
            }))
            .await
            .unwrap(),
        Response::Project(ProjectResponse::Closed)
    );
    let missing = client
        .request(Request::Project(ProjectRequest::ScanDirectory {
            project_id,
            relative_directory: path(std::path::Path::new("")),
            show_hidden: false,
        }))
        .await
        .unwrap_err();
    assert!(matches!(
        missing,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::NotFound
    ));
    assert_eq!(
        client.request(Request::DrainAndStop).await.unwrap(),
        Response::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "manual Host terminal performance probe"]
async fn host_terminal_performance_probe() {
    let host = RunningHost::start().await;
    let client = host.client("host-performance").await;
    let session_id = TerminalSessionId::new("host-performance");
    let mut spec = spawn_spec();
    spec.session_id = session_id.clone();
    spec.scrollback_limit = 1_000;
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            "yes 0123456789abcdef | head -c 8388608; printf '\\nHOST_BURST_DONE\\n'".to_string(),
        ],
        return_to_shell: false,
    };
    let started = std::time::Instant::now();
    assert!(matches!(
        client.request(Request::SpawnTerminal(spec)).await.unwrap(),
        Response::TerminalSpawned { .. }
    ));
    tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport_text(&viewport).contains("HOST_BURST_DONE"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("Host burst output timeout");
    let output_elapsed = started.elapsed();

    let mut round_trips = Vec::with_capacity(200);
    for sequence in 0..200 {
        let started = std::time::Instant::now();
        assert!(matches!(
            client
                .request(Request::Ping {
                    sent_millis: sequence,
                })
                .await
                .unwrap(),
            Response::Pong { .. }
        ));
        round_trips.push(started.elapsed());
    }
    round_trips.sort();
    let p50 = round_trips[round_trips.len() / 2];
    let p95 = round_trips[round_trips.len() * 95 / 100];
    let rss_kib = Command::new("ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|rss| rss.trim().parse::<u64>().ok());
    eprintln!(
        "HOST_PERF bytes=8388608 elapsed_ms={} throughput_mib_s={:.2} ping_p50_us={} ping_p95_us={} rss_kib={}",
        output_elapsed.as_millis(),
        8.0 / output_elapsed.as_secs_f64(),
        p50.as_micros(),
        p95.as_micros(),
        rss_kib.map_or_else(|| "unavailable".to_string(), |rss| rss.to_string()),
    );

    let _ = client
        .request(Request::TerminateTerminal {
            session_id,
            mode: TerminationMode::Terminate,
        })
        .await;
    assert_eq!(
        client.request(Request::DrainAndStop).await.unwrap(),
        Response::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}
