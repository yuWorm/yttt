#![cfg(unix)]

use std::{
    fs,
    io::{Read as _, Write as _},
    net::TcpStream,
    os::unix::fs::PermissionsExt as _,
    process::Command,
    time::Duration,
};

use tempfile::TempDir;

use yttt_client_core::{ClientCore, ClientCoreError, ClientEvent, ConnectionState, TerminalMirror};
use yttt_core::model::ids::{ClientInstanceId, ProfileId, ProjectId, TerminalSessionId};
use yttt_host::{HostBootstrap, diagnostics::HostDiagnosticsSnapshot, run};
use yttt_protocol::{
    BuildIdentity, ClientRequest, ControlMessage, FailureCode, LIFECYCLE_PROTOCOL_VERSION,
    LifecycleMessage, LifecycleRequest, LifecycleRequestEnvelope, LifecycleResponse,
    LifecycleResponseEnvelope, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    TerminalInteractiveMessage,
    agent::AgentSnapshotCursor,
    project::{
        ProjectFileState, ProjectGitOperation, ProjectRequest, ProjectResponse, ProjectSaveMode,
        ProjectSaveResult,
    },
    ssh::{
        CredentialAnswer, CredentialChallengeKind, HostKeyDecision, RemoteCommandRequest,
        RemoteFileRequest, RemoteFileResponse, SensitiveBytes, SshAuthentication, SshConnectSpec,
        SshConnectionState, SshEndpoint,
    },
    terminal::{
        AttachTerminal, ReadTerminalViewport, RemoteTerminalExecutionSpec, ResizeTerminal,
        ScrollTerminal, SearchTerminal, TerminalExecutionSpec, TerminalGeometry, TerminalInput,
        TerminalLeaseMode, TerminalMutationContext, TerminalProcessState, TerminalSpawnSpec,
        TerminalViewportAnchor, TerminateTerminalRequest, TerminationMode,
    },
};
use yttt_transport_local::{
    AuthToken, ClientIdentity, LocalConnector, LocalEndpoint, LocalListener, client_handshake,
    connect, receive_control, receive_lifecycle, receive_terminal_interactive, send_control,
    send_lifecycle, send_terminal_interactive,
};

fn host_path(path: &std::path::Path) -> yttt_protocol::HostPath {
    yttt_protocol::HostPath::from_path(path).unwrap()
}

fn rel_path(path: &std::path::Path) -> yttt_protocol::ProjectRelativePath {
    yttt_protocol::ProjectRelativePath::from_path(path).unwrap()
}

fn remote_rel(path: &str) -> yttt_protocol::ProjectRelativePath {
    yttt_protocol::ProjectRelativePath::from_utf8(path.trim_start_matches('/')).unwrap()
}

fn host_endpoint(bootstrap: &HostBootstrap) -> LocalEndpoint {
    LocalEndpoint::for_profile(bootstrap.profile_id.clone(), bootstrap.runtime_root.clone())
}

struct RunningHost {
    _temp: TempDir,
    bootstrap: HostBootstrap,
    token: [u8; 32],
    task: tokio::task::JoinHandle<Result<(), yttt_host::HostError>>,
    session_nonces: std::sync::Mutex<std::collections::HashMap<String, yttt_protocol::Nonce>>,
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
            state_root: temp.path().join("state"),
            config_root: temp.path().join("state/config"),
            auth_token_file,
            ssh_host_keys_file: temp.path().join("ssh-host-keys.toml"),
            credential_namespace: "dev.yttt.ssh.integration-test".to_string(),
            build: BuildIdentity {
                product_version: "0.2.0".to_string(),
                build_fingerprint: "integration-build".to_string(),
                resource_compatibility: "integration-resource-v1".to_string(),
            },
            lifetime: yttt_host::HostLifetime::Independent,
        };
        let endpoint = host_endpoint(&bootstrap);
        let task = tokio::spawn(run(bootstrap.clone(), || LocalListener::bind(endpoint)));
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
            session_nonces: Default::default(),
        }
    }

    fn session_nonce(&self, id: &str) -> yttt_protocol::Nonce {
        *self
            .session_nonces
            .lock()
            .unwrap()
            .entry(id.to_string())
            .or_insert_with(yttt_transport::new_session_nonce)
    }

    async fn client(&self, id: &str) -> ClientCore {
        let client = ClientCore::connect(
            LocalConnector::new(host_endpoint(&self.bootstrap)),
            ClientIdentity {
                expected_environment: None,
                credential_generation: 0,
                session_nonce: self.session_nonce(id),
                supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
                build: self.bootstrap.build.clone(),
                profile_id: self.bootstrap.profile_id.clone(),
                client_instance_id: ClientInstanceId::new(id),
                host_epoch_hint: None,
                can_force_stop: false,
                channel: yttt_protocol::ConnectionChannel::Control,
                terminal_session_id: None,
            },
            AuthToken::from_bytes(self.token),
        )
        .await
        .unwrap();
        if client.control_status().unwrap().owner.is_none() {
            client
                .request(Request::ProfileControl(
                    yttt_protocol::session::ProfileControlRequest::RequestControl,
                ))
                .await
                .unwrap();
        }
        client
    }

    async fn lifecycle_request(
        &self,
        body: LifecycleRequest,
        can_force_stop: bool,
    ) -> LifecycleResponse {
        let mut client = raw_lifecycle_client(self, "lifecycle-client", can_force_stop).await;
        raw_lifecycle_request(&mut client, 1, body).await
    }
}

async fn raw_client(host: &RunningHost, id: &str) -> yttt_transport_local::LocalStream {
    let mut stream = connect(&host_endpoint(&host.bootstrap)).await.unwrap();
    client_handshake(
        &mut stream,
        &ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: host.session_nonce(id),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: host.bootstrap.build.clone(),
            profile_id: host.bootstrap.profile_id.clone(),
            client_instance_id: ClientInstanceId::new(id),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: yttt_protocol::ConnectionChannel::Control,
            terminal_session_id: None,
        },
        &AuthToken::from_bytes(host.token),
    )
    .await
    .unwrap();
    stream
}

async fn raw_lifecycle_client(
    host: &RunningHost,
    id: &str,
    can_force_stop: bool,
) -> yttt_transport_local::LocalStream {
    raw_lifecycle_client_for(&host.bootstrap, host.token, id, can_force_stop).await
}

async fn raw_lifecycle_client_for(
    bootstrap: &HostBootstrap,
    token: [u8; 32],
    id: &str,
    can_force_stop: bool,
) -> yttt_transport_local::LocalStream {
    let mut stream = connect(&host_endpoint(bootstrap)).await.unwrap();
    client_handshake(
        &mut stream,
        &ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(LIFECYCLE_PROTOCOL_VERSION),
            build: BuildIdentity {
                product_version: "0.3.0".to_string(),
                build_fingerprint: "next-build".to_string(),
                resource_compatibility: "next-resource".to_string(),
            },
            profile_id: bootstrap.profile_id.clone(),
            client_instance_id: ClientInstanceId::new(id),
            host_epoch_hint: None,
            can_force_stop,
            channel: yttt_protocol::ConnectionChannel::Lifecycle,
            terminal_session_id: None,
        },
        &AuthToken::from_bytes(token),
    )
    .await
    .unwrap();
    stream
}

async fn raw_terminal_data_client(
    host: &RunningHost,
    id: &str,
    session_id: TerminalSessionId,
) -> yttt_transport_local::LocalStream {
    let mut stream = connect(&host_endpoint(&host.bootstrap)).await.unwrap();
    client_handshake(
        &mut stream,
        &ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: host.session_nonce(id),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: host.bootstrap.build.clone(),
            profile_id: host.bootstrap.profile_id.clone(),
            client_instance_id: ClientInstanceId::new(id),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: yttt_protocol::ConnectionChannel::TerminalData,
            terminal_session_id: Some(session_id),
        },
        &AuthToken::from_bytes(host.token),
    )
    .await
    .unwrap();
    stream
}

async fn raw_terminal_interactive_client(
    host: &RunningHost,
    id: &str,
) -> yttt_transport_local::LocalStream {
    let mut stream = connect(&host_endpoint(&host.bootstrap)).await.unwrap();
    client_handshake(
        &mut stream,
        &ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: host.session_nonce(id),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: host.bootstrap.build.clone(),
            profile_id: host.bootstrap.profile_id.clone(),
            client_instance_id: ClientInstanceId::new(id),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: yttt_protocol::ConnectionChannel::TerminalInteractive,
            terminal_session_id: None,
        },
        &AuthToken::from_bytes(host.token),
    )
    .await
    .unwrap();
    stream
}

async fn raw_interactive_request(
    stream: &mut yttt_transport_local::LocalStream,
    request_id: u64,
    body: Request,
    control: Option<yttt_protocol::session::ControlContext>,
) -> Result<Response, yttt_protocol::ProtocolFailure> {
    let mut request = ClientRequest::new(request_id, body);
    request.control = control;
    send_terminal_interactive(stream, &TerminalInteractiveMessage::Request(request))
        .await
        .unwrap();
    let TerminalInteractiveMessage::Response(response) =
        receive_terminal_interactive(stream).await.unwrap()
    else {
        panic!("unexpected interactive response");
    };
    assert_eq!(response.request_id, request_id);
    response.result
}

async fn raw_request(
    stream: &mut yttt_transport_local::LocalStream,
    request_id: u64,
    body: Request,
) -> Result<Response, yttt_protocol::ProtocolFailure> {
    let mut request = ClientRequest::new(request_id, body);
    if matches!(
        request.body,
        Request::SpawnTerminal(_)
            | Request::TerminateTerminal { .. }
            | Request::AcknowledgeTerminalExit { .. }
    ) {
        let Response::ProfileControl(mut status) = raw_send(
            stream,
            ClientRequest::new(
                u64::MAX,
                Request::ProfileControl(yttt_protocol::session::ProfileControlRequest::Status),
            ),
        )
        .await?
        else {
            panic!("profile status")
        };
        if status.owner.is_none() {
            let Response::ProfileControl(acquired) = raw_send(
                stream,
                ClientRequest::new(
                    u64::MAX,
                    Request::ProfileControl(
                        yttt_protocol::session::ProfileControlRequest::RequestControl,
                    ),
                ),
            )
            .await?
            else {
                panic!("profile control")
            };
            status = acquired;
        }
        request.control = Some(status.context);
    }
    raw_send(stream, request).await
}

async fn raw_send(
    stream: &mut yttt_transport_local::LocalStream,
    request: ClientRequest,
) -> Result<Response, yttt_protocol::ProtocolFailure> {
    let request_id = request.request_id;
    send_control(stream, &ControlMessage::Request(request))
        .await
        .unwrap();
    loop {
        match receive_control(stream).await.unwrap() {
            ControlMessage::Response(response) if response.request_id == request_id => {
                return response.result;
            }
            ControlMessage::Event(_) => {}
            message => panic!("unexpected raw client message: {message:?}"),
        }
    }
}

async fn raw_lifecycle_request(
    stream: &mut yttt_transport_local::LocalStream,
    request_id: u64,
    body: LifecycleRequest,
) -> LifecycleResponse {
    send_lifecycle(
        stream,
        &LifecycleMessage::Request(LifecycleRequestEnvelope { request_id, body }),
    )
    .await
    .unwrap();
    match receive_lifecycle(stream).await.unwrap() {
        LifecycleMessage::Response(LifecycleResponseEnvelope {
            request_id: response_id,
            result,
        }) if response_id == request_id => result,
        message => panic!("unexpected lifecycle message: {message:?}"),
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

fn mutation_context(
    client: &ClientCore,
    session_epoch: u64,
    lease_epoch: u64,
    geometry_epoch: u64,
    client_sequence: u64,
) -> TerminalMutationContext {
    let ConnectionState::Ready { host_epoch, .. } = client.state() else {
        panic!("client is not ready");
    };
    TerminalMutationContext {
        host_epoch,
        session_epoch,
        lease_epoch,
        geometry_epoch,
        client_sequence,
    }
}

fn spawn_spec() -> TerminalSpawnSpec {
    TerminalSpawnSpec {
        session_id: TerminalSessionId::new("host-owned"),
        project_id: ProjectId::new("project"),
        cwd: yttt_protocol::ProjectRelativePath::root(),
        execution: TerminalExecutionSpec::Command {
            shell: "/bin/sh".to_string(),
            program: "/bin/sh".to_string(),
            args: vec![
                "-lc".to_string(),
                "printf client-core-ready; IFS= read -r line; printf '\\nreattached:%s\\n' \"$line\"; sleep 30".to_string(),
            ],
            return_to_shell: false,
        },
        geometry: geometry(),
        geometry_epoch: 1,
        query_palette: Vec::new(),
        palette_revision: 1,
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

#[tokio::test]
async fn terminal_output_uses_a_dedicated_data_connection() {
    let host = RunningHost::start().await;
    let client_id = "split-stream-client";
    let mut control = raw_client(&host, client_id).await;
    let mut interactive = raw_terminal_interactive_client(&host, client_id).await;
    let mut spec = spawn_spec();
    spec.session_id = TerminalSessionId::new("split-stream");
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            concat!(
                "IFS= read -r line; ",
                "printf 'data-channel-first:%s\\n' \"$line\"; ",
                "sleep 1; printf 'data-channel-final\\n'; sleep 30"
            )
            .to_string(),
        ],
        return_to_shell: false,
    };
    let session_id = spec.session_id.clone();
    let Response::TerminalSpawned {
        lease,
        session_epoch,
    } = raw_request(&mut control, 1, Request::SpawnTerminal(spec))
        .await
        .unwrap()
    else {
        panic!("unexpected spawn response");
    };
    let Response::Resources(resources) = raw_request(&mut control, 2, Request::ListResources)
        .await
        .unwrap()
    else {
        panic!("unexpected catalog response");
    };
    let mut data = raw_terminal_data_client(&host, client_id, session_id.clone()).await;
    let initial = tokio::time::timeout(Duration::from_secs(5), receive_control(&mut data))
        .await
        .expect("terminal data snapshot timeout")
        .unwrap();
    let ControlMessage::Event(yttt_protocol::HostEvent {
        body:
            yttt_protocol::ServerEvent::Terminal(
                yttt_protocol::terminal::TerminalStreamUpdate::Snapshot(viewport),
            ),
        ..
    }) = initial
    else {
        panic!("unexpected initial terminal data message: {initial:?}");
    };
    let geometry_epoch = viewport.geometry_epoch;
    let mut mirror = TerminalMirror::new(viewport);
    let Response::ProfileControl(control_status) = raw_request(
        &mut control,
        99,
        Request::ProfileControl(yttt_protocol::session::ProfileControlRequest::Status),
    )
    .await
    .unwrap() else {
        panic!("control status")
    };
    assert_eq!(
        raw_interactive_request(
            &mut interactive,
            3,
            Request::TerminalInput(TerminalInput {
                session_id: session_id.clone(),
                context: TerminalMutationContext {
                    host_epoch: resources.host_epoch,
                    session_epoch,
                    lease_epoch: lease.lease_epoch,
                    geometry_epoch,
                    client_sequence: 1,
                },
                bytes: b"separate\r".to_vec(),
            }),
            Some(control_status.context),
        )
        .await
        .unwrap(),
        Response::TerminalInputAccepted { client_sequence: 1 }
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let message = receive_control(&mut data).await.unwrap();
            let ControlMessage::Event(yttt_protocol::HostEvent {
                body: yttt_protocol::ServerEvent::Terminal(update),
                ..
            }) = message
            else {
                panic!("unexpected terminal data message: {message:?}");
            };
            mirror.apply(&update);
            if viewport_text(mirror.viewport()).contains("data-channel-final") {
                break;
            }
        }
    })
    .await
    .expect("terminal data output timeout");
    assert!(
        tokio::time::timeout(Duration::from_millis(200), receive_control(&mut control),)
            .await
            .is_err(),
        "control connection received terminal output"
    );
    let terminated = raw_request(
        &mut control,
        4,
        Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        },
    )
    .await
    .unwrap();
    let Response::TerminalTerminated(terminated) = terminated else {
        panic!("unexpected terminate response");
    };
    assert_eq!(terminated.session_epoch, session_epoch);
    assert_eq!(
        raw_request(
            &mut control,
            5,
            Request::AcknowledgeTerminalExit {
                session_id,
                session_epoch,
                final_sequence: terminated.final_sequence,
            },
        )
        .await
        .unwrap(),
        Response::TerminalExitAcknowledged
    );
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn blocked_project_request_does_not_delay_terminal_input() {
    let host = RunningHost::start().await;
    let client = host.client("mixed-load-client").await;
    let project_root = host._temp.path().join("blocking-project");
    fs::create_dir_all(&project_root).unwrap();
    assert!(
        Command::new("git")
            .args(["init", "-q"])
            .arg(&project_root)
            .status()
            .expect("git init must run")
            .success()
    );
    let fsmonitor = project_root.join("blocking-fsmonitor.sh");
    fs::write(&fsmonitor, "#!/bin/sh\nsleep 2\nprintf '/\\n'\n").unwrap();
    let mut permissions = fs::metadata(&fsmonitor).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&fsmonitor, permissions).unwrap();
    assert!(
        Command::new("git")
            .arg("-C")
            .arg(&project_root)
            .args(["config", "core.fsmonitor"])
            .arg(&fsmonitor)
            .status()
            .expect("git config must run")
            .success()
    );
    let project_id = ProjectId::new("blocking-project");
    assert!(matches!(
        client
            .request(Request::Project(ProjectRequest::Register {
                view_id: "test-view".to_string(),
                project_id: project_id.clone(),
                root: host_path(&project_root),
            }))
            .await
            .unwrap(),
        Response::Project(ProjectResponse::Registered { .. })
    ));
    let mut events = client.subscribe_events();

    let mut spec = spawn_spec();
    spec.session_id = TerminalSessionId::new("mixed-load-terminal");
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            "IFS= read -r line; printf 'interactive:%s\\n' \"$line\"; sleep 30".to_string(),
        ],
        return_to_shell: false,
    };
    let session_id = spec.session_id.clone();
    let Response::TerminalSpawned {
        lease,
        session_epoch,
    } = client.request(Request::SpawnTerminal(spec)).await.unwrap()
    else {
        panic!("unexpected spawn response");
    };
    let geometry_epoch = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Some(viewport) = client.terminal_snapshot(&session_id) {
                break viewport.geometry_epoch;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("initial terminal snapshot timeout");

    let blocked_git = client
        .enqueue_request(Request::Project(ProjectRequest::Git {
            project_id: project_id.clone(),
            operation: ProjectGitOperation::Status { work_tree: None },
        }))
        .unwrap();
    while events.try_recv().is_ok() {}
    tokio::time::sleep(Duration::from_millis(100)).await;
    client
        .send_terminal_input(TerminalInput {
            session_id: session_id.clone(),
            context: mutation_context(&client, session_epoch, lease.lease_epoch, geometry_epoch, 1),
            bytes: b"while-control-blocked\r".to_vec(),
        })
        .unwrap();
    fs::write(project_root.join("state-event.txt"), "changed").unwrap();
    tokio::time::timeout(Duration::from_secs(1), async {
        let mut received_input = false;
        let mut received_project_event = false;
        while !received_input || !received_project_event {
            received_input |= client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| {
                    viewport_text(&viewport).contains("interactive:while-control-blocked")
                });
            tokio::select! {
                event = events.recv() => {
                    if let Ok(ClientEvent::Server(yttt_protocol::HostEvent {
                        body: yttt_protocol::ServerEvent::ProjectChanged(change),
                        ..
                    })) = event
                    {
                        received_project_event |= change.project_id == project_id;
                    }
                }
                _ = tokio::time::sleep(Duration::from_millis(10)) => {}
            }
        }
    })
    .await
    .expect("interactive input or a state event was delayed by a blocked project request");

    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), blocked_git.wait())
            .await
            .expect("blocked project request did not resume")
            .unwrap(),
        Response::Project(ProjectResponse::Git(_))
    ));

    let Response::TerminalTerminated(terminated) = client
        .request(Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        })
        .await
        .unwrap()
    else {
        panic!("unexpected terminate response");
    };
    assert_eq!(
        client
            .request(Request::AcknowledgeTerminalExit {
                session_id,
                session_epoch,
                final_sequence: terminated.final_sequence,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
}

#[tokio::test]
async fn many_terminals_keep_catalog_and_interactive_lanes_responsive() {
    const TERMINAL_COUNT: usize = 16;

    let host = RunningHost::start().await;
    let client = host.client("many-terminals").await;
    let mut session_ids = Vec::with_capacity(TERMINAL_COUNT);
    let mut first_lease = None;
    for index in 0..TERMINAL_COUNT {
        let mut spec = spawn_spec();
        spec.session_id = TerminalSessionId::new(format!("many-terminal-{index}"));
        spec.execution = TerminalExecutionSpec::Command {
            shell: "/bin/sh".to_string(),
            program: "/bin/sh".to_string(),
            args: vec![
                "-lc".to_string(),
                "IFS= read -r line; printf 'many:%s\\n' \"$line\"; sleep 30".to_string(),
            ],
            return_to_shell: false,
        };
        let session_id = spec.session_id.clone();
        let Response::TerminalSpawned {
            lease,
            session_epoch,
        } = client.request(Request::SpawnTerminal(spec)).await.unwrap()
        else {
            panic!("unexpected spawn response");
        };
        if index == 0 {
            first_lease = Some((lease, session_epoch));
        }
        session_ids.push(session_id);
    }

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client.resource_catalog().is_some_and(|catalog| {
                session_ids.iter().all(|session_id| {
                    catalog
                        .terminals
                        .iter()
                        .any(|terminal| &terminal.session_id == session_id)
                })
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("many-terminal resource catalog convergence timeout");

    let (first_lease, first_session_epoch) = first_lease.unwrap();
    client
        .send_terminal_input(TerminalInput {
            session_id: session_ids[0].clone(),
            context: mutation_context(&client, first_session_epoch, first_lease.lease_epoch, 1, 1),
            bytes: b"responsive\r".to_vec(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if client
                .terminal_snapshot(&session_ids[0])
                .is_some_and(|viewport| viewport_text(&viewport).contains("many:responsive"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("interactive input stalled with many terminals");

    assert!(matches!(
        tokio::time::timeout(
            Duration::from_secs(1),
            client.request(Request::Ping { sent_millis: 1 })
        )
        .await
        .expect("control request stalled with many terminals")
        .unwrap(),
        Response::Pong { .. }
    ));

    let response = client
        .request(Request::TerminateMany {
            requests: session_ids
                .into_iter()
                .enumerate()
                .map(|(index, session_id)| TerminateTerminalRequest {
                    request_id: index as u64,
                    session_id,
                })
                .collect(),
        })
        .await
        .unwrap();
    let Response::TerminalsTerminated { results } = response else {
        panic!("unexpected terminate-many response");
    };
    assert_eq!(results.len(), TERMINAL_COUNT);
    assert!(results.iter().all(|result| result.result.is_ok()));
}

#[tokio::test]
#[ignore = "manual Host terminal scaling performance probe"]
async fn host_terminal_scale_performance_probe() {
    const SAMPLE_COUNT: u64 = 40;
    const TERMINAL_COUNTS: [usize; 3] = [1, 8, 24];

    let host = RunningHost::start().await;
    let client = host.client("host-scale-performance").await;
    let mut session_ids = Vec::with_capacity(*TERMINAL_COUNTS.last().unwrap());
    let mut first_lease = None;
    let mut client_sequence = 0_u64;
    let mut p95_measurements = Vec::new();

    for terminal_count in TERMINAL_COUNTS {
        while session_ids.len() < terminal_count {
            let index = session_ids.len();
            let mut spec = spawn_spec();
            spec.session_id = TerminalSessionId::new(format!("host-scale-{index}"));
            spec.scrollback_limit = 1_000;
            spec.execution = TerminalExecutionSpec::Command {
                shell: "/bin/sh".to_string(),
                program: "/bin/sh".to_string(),
                args: vec![
                    "-lc".to_string(),
                    "while IFS= read -r line; do printf 'scale:%s\\n' \"$line\"; done".to_string(),
                ],
                return_to_shell: false,
            };
            let session_id = spec.session_id.clone();
            let Response::TerminalSpawned {
                lease,
                session_epoch,
            } = client.request(Request::SpawnTerminal(spec)).await.unwrap()
            else {
                panic!("unexpected scale terminal spawn response");
            };
            if first_lease.is_none() {
                first_lease = Some((lease, session_epoch));
            }
            session_ids.push(session_id);
        }

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if client.terminal_snapshot(&session_ids[0]).is_some() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(1)).await;
            }
        })
        .await
        .expect("scale terminal initial snapshot timeout");

        let (lease, session_epoch) = first_lease.as_ref().unwrap();
        let mut latencies = Vec::with_capacity(SAMPLE_COUNT as usize);
        for _ in 0..SAMPLE_COUNT {
            client_sequence = client_sequence.saturating_add(1);
            let marker = format!("probe-{client_sequence}");
            let expected = format!("scale:{marker}");
            let started = std::time::Instant::now();
            client
                .send_terminal_input(TerminalInput {
                    session_id: session_ids[0].clone(),
                    context: mutation_context(
                        &client,
                        *session_epoch,
                        lease.lease_epoch,
                        1,
                        client_sequence,
                    ),
                    bytes: format!("{marker}\r").into_bytes(),
                })
                .unwrap();
            tokio::time::timeout(Duration::from_secs(2), async {
                loop {
                    if client
                        .terminal_snapshot(&session_ids[0])
                        .is_some_and(|viewport| viewport_text(&viewport).contains(&expected))
                    {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
            })
            .await
            .expect("scale terminal input-to-mirror timeout");
            latencies.push(started.elapsed());
        }
        latencies.sort_unstable();
        let p50 = latencies[latencies.len() / 2];
        let p95 = latencies[latencies.len() * 95 / 100];
        let rss_kib = Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .filter(|output| output.status.success())
            .and_then(|output| String::from_utf8(output.stdout).ok())
            .and_then(|rss| rss.trim().parse::<u64>().ok());
        eprintln!(
            "HOST_SCALE terminals={terminal_count} input_to_mirror_p50_us={} input_to_mirror_p95_us={} rss_kib={}",
            p50.as_micros(),
            p95.as_micros(),
            rss_kib.map_or_else(|| "unavailable".to_string(), |rss| rss.to_string()),
        );
        p95_measurements.push(p95);
    }

    let low_scale_p95 = p95_measurements[0];
    let high_scale_p95 = *p95_measurements.last().unwrap();
    assert!(
        high_scale_p95
            <= low_scale_p95
                .saturating_mul(4)
                .max(Duration::from_millis(100)),
        "24-terminal p95 {high_scale_p95:?} regressed from one-terminal p95 {low_scale_p95:?}"
    );

    let response = client
        .request(Request::TerminateMany {
            requests: session_ids
                .into_iter()
                .enumerate()
                .map(|(index, session_id)| TerminateTerminalRequest {
                    request_id: index as u64,
                    session_id,
                })
                .collect(),
        })
        .await
        .unwrap();
    let Response::TerminalsTerminated { results } = response else {
        panic!("unexpected scale terminate-many response");
    };
    assert_eq!(results.len(), *TERMINAL_COUNTS.last().unwrap());
    assert!(results.iter().all(|result| result.result.is_ok()));
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
async fn force_capable_control_client_uses_restricted_terminal_data_channel_and_reattaches() {
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
    let diagnostics = first.diagnostics();
    assert!(diagnostics.ipc_payload_bytes > 0);
    assert!(diagnostics.ipc_read_and_decode.samples > 0);
    assert!(diagnostics.terminal_merge.samples > 0);
    assert_eq!(
        first
            .request(Request::DetachTerminal {
                session_id: TerminalSessionId::new("host-owned"),
            })
            .await
            .unwrap(),
        Response::TerminalDetached
    );
    drop(first);

    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(!host.task.is_finished());
    let second = host.client("second-client").await;
    wait_for_mirror(&second, "client-core-ready").await;

    let reattached_geometry = TerminalGeometry {
        cols: 100,
        rows: 30,
        cell_width: 8,
        cell_height: 16,
    };
    let attached = second
        .request(Request::AttachTerminal(AttachTerminal {
            session_id: TerminalSessionId::new("host-owned"),
            known_session_epoch: Some(session_epoch),
            after_sequence: second
                .terminal_snapshot(&TerminalSessionId::new("host-owned"))
                .map(|viewport| viewport.sequence),
            mode: yttt_protocol::terminal::TerminalLeaseMode::Interactive,
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry: reattached_geometry,
            geometry_epoch: 2,
        }))
        .await
        .unwrap();
    let Response::TerminalAttached { checkpoint, lease } = attached else {
        panic!("unexpected attach response: {attached:?}");
    };
    assert_eq!(lease.owner.as_str(), "second-client");
    assert_eq!(checkpoint.viewport.geometry, reattached_geometry);
    assert!(viewport_text(&checkpoint.viewport).contains("client-core-ready"));
    assert_eq!(
        second
            .request(Request::TerminalInput(TerminalInput {
                session_id: TerminalSessionId::new("host-owned"),
                context: mutation_context(&second, session_epoch, lease.lease_epoch, 2, 1),
                bytes: b"after-detach\r".to_vec(),
            }))
            .await
            .unwrap(),
        Response::TerminalInputAccepted { client_sequence: 1 }
    );
    wait_for_mirror(&second, "reattached:after-detach").await;

    let terminated = second
        .request(Request::TerminateTerminal {
            session_id: TerminalSessionId::new("host-owned"),
            mode: TerminationMode::Terminate,
        })
        .await
        .unwrap();
    let Response::TerminalTerminated(terminated) = terminated else {
        panic!("unexpected terminate response");
    };
    assert_eq!(
        second
            .request(Request::AcknowledgeTerminalExit {
                session_id: TerminalSessionId::new("host-owned"),
                session_epoch,
                final_sequence: terminated.final_sequence,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn forced_takeover_waits_for_an_admitted_git_write_to_finish() {
    use yttt_protocol::session::{ProfileControlRequest, TransferPhase};
    let host = RunningHost::start().await;
    let owner = host.client("slow-writer").await;
    let successor = host.client("waiting-writer").await;
    let project_root = host._temp.path().join("fenced-git");
    fs::create_dir_all(&project_root).unwrap();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.name", "Smoke"],
        vec!["config", "user.email", "smoke@example.invalid"],
        vec!["commit", "--allow-empty", "-qm", "initial"],
        vec!["branch", "next"],
    ] {
        assert!(
            Command::new("git")
                .arg("-C")
                .arg(&project_root)
                .args(args)
                .status()
                .unwrap()
                .success()
        );
    }
    let hook = project_root.join(".git/hooks/post-checkout");
    fs::write(&hook, "#!/bin/sh\ntouch .git/write-entered\ni=0\nwhile [ ! -f .git/write-release ]; do i=$((i + 1)); [ \"$i\" -lt 400 ] || exit 1; sleep 0.05; done\n").unwrap();
    fs::set_permissions(&hook, fs::Permissions::from_mode(0o700)).unwrap();
    let project_id = ProjectId::new("fenced-git");
    owner
        .request(Request::Project(ProjectRequest::Register {
            view_id: "writer".into(),
            project_id: project_id.clone(),
            root: host_path(&project_root),
        }))
        .await
        .unwrap();
    let write = owner
        .enqueue_request(Request::Project(ProjectRequest::Git {
            project_id,
            operation: ProjectGitOperation::Switch {
                name: "next".into(),
                track_remote: false,
            },
        }))
        .unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while !project_root.join(".git/write-entered").exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("real Git hook must enter the admitted mutation");
    let Response::ProfileControl(preparing) = successor
        .request(Request::ProfileControl(
            ProfileControlRequest::RequestControl,
        ))
        .await
        .unwrap()
    else {
        panic!("preparing")
    };
    let transfer_id = preparing.transfer.unwrap().id;
    tokio::time::timeout(Duration::from_secs(6), async {
        loop {
            let Response::ProfileControl(status) = successor
                .request(Request::ProfileControl(ProfileControlRequest::Status))
                .await
                .unwrap()
            else {
                panic!("status")
            };
            assert_eq!(
                status.owner.as_ref(),
                Some(owner.client_id()),
                "deadline alone must not grant control"
            );
            if status.transfer.unwrap().phase == TransferPhase::ForceConfirmationRequired {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("force confirmation deadline");
    let forced = successor
        .enqueue_request(Request::ProfileControl(
            ProfileControlRequest::ConfirmForce { transfer_id },
        ))
        .unwrap();
    let mut forced = Box::pin(forced.wait());
    assert!(
        tokio::time::timeout(Duration::from_millis(100), &mut forced)
            .await
            .is_err(),
        "takeover must drain the in-flight Git mutation"
    );
    fs::write(project_root.join(".git/write-release"), "").unwrap();
    write.wait().await.unwrap();
    let Response::ProfileControl(granted) = tokio::time::timeout(Duration::from_secs(5), forced)
        .await
        .unwrap()
        .unwrap()
    else {
        panic!("granted")
    };
    assert_eq!(granted.owner.as_ref(), Some(successor.client_id()));
    assert_eq!(
        fs::read_to_string(project_root.join(".git/HEAD"))
            .unwrap()
            .trim(),
        "ref: refs/heads/next"
    );
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::ForceStop, true)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .unwrap()
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn request_journal_replays_mutations_without_repeating_side_effects() {
    let host = RunningHost::start().await;
    let mut client = raw_client(&host, "journal-client").await;
    let spec = spawn_spec();
    let session_id = spec.session_id.clone();

    let first = raw_request(&mut client, 41, Request::SpawnTerminal(spec.clone()))
        .await
        .unwrap();
    let replay = raw_request(&mut client, 41, Request::SpawnTerminal(spec.clone()))
        .await
        .unwrap();
    assert_eq!(replay, first);

    let conflicting = raw_request(
        &mut client,
        41,
        Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(conflicting.code, FailureCode::InvalidRequest);

    let terminated = raw_request(
        &mut client,
        42,
        Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        },
    )
    .await
    .unwrap();
    assert!(matches!(terminated, Response::TerminalTerminated(_)));
    assert_eq!(
        raw_request(
            &mut client,
            42,
            Request::TerminateTerminal {
                session_id: session_id.clone(),
                mode: TerminationMode::Terminate,
            },
        )
        .await
        .unwrap(),
        terminated
    );
    let Response::TerminalSpawned { session_epoch, .. } = first else {
        panic!("unexpected spawn response");
    };
    let Response::TerminalTerminated(termination) = &terminated else {
        panic!("unexpected terminate response");
    };
    assert_eq!(termination.session_epoch, session_epoch);
    let acknowledged = raw_request(
        &mut client,
        43,
        Request::AcknowledgeTerminalExit {
            session_id: session_id.clone(),
            session_epoch,
            final_sequence: termination.final_sequence,
        },
    )
    .await
    .unwrap();
    assert_eq!(acknowledged, Response::TerminalExitAcknowledged);
    assert_eq!(
        raw_request(
            &mut client,
            43,
            Request::AcknowledgeTerminalExit {
                session_id,
                session_epoch,
                final_sequence: termination.final_sequence,
            },
        )
        .await
        .unwrap(),
        acknowledged
    );

    let Response::ProfileControl(old_control) = raw_request(
        &mut client,
        70,
        Request::ProfileControl(yttt_protocol::session::ProfileControlRequest::Status),
    )
    .await
    .unwrap() else {
        panic!("old control")
    };
    let successor = host.client("journal-successor").await;
    let Response::ProfileControl(preparing) = successor
        .request(Request::ProfileControl(
            yttt_protocol::session::ProfileControlRequest::RequestControl,
        ))
        .await
        .unwrap()
    else {
        panic!("preparing")
    };
    raw_request(
        &mut client,
        71,
        Request::ProfileControl(yttt_protocol::session::ProfileControlRequest::Ready {
            transfer_id: preparing.transfer.unwrap().id,
            revisions: vec![],
        }),
    )
    .await
    .unwrap();
    let mut stale_replay = ClientRequest::new(41, Request::SpawnTerminal(spec.clone()));
    stale_replay.control = Some(old_control.context);
    assert_eq!(
        raw_send(&mut client, stale_replay).await.unwrap_err().code,
        FailureCode::StaleEpoch
    );
    assert_eq!(
        raw_request(&mut client, 41, Request::SpawnTerminal(spec))
            .await
            .unwrap_err()
            .code,
        FailureCode::PermissionDenied
    );
    let Response::Resources(resources) = successor.request(Request::ListResources).await.unwrap()
    else {
        panic!("resources")
    };
    assert!(
        resources.terminals.is_empty(),
        "denied replay must not respawn acknowledged terminals"
    );

    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn exited_terminal_reconnects_with_its_final_checkpoint_until_acknowledged() {
    let host = RunningHost::start().await;
    let first = host.client("exit-first").await;
    let mut spec = spawn_spec();
    spec.session_id = TerminalSessionId::new("exited-reconnect");
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            "printf 'final-before-exit\\n'; sleep 1; exit 7".to_string(),
        ],
        return_to_shell: false,
    };
    let session_id = spec.session_id.clone();
    let spawned = first.request(Request::SpawnTerminal(spec)).await.unwrap();
    let Response::TerminalSpawned { session_epoch, .. } = spawned else {
        panic!("unexpected spawn response: {spawned:?}");
    };
    drop(first);
    tokio::time::sleep(Duration::from_millis(1_200)).await;

    let second = host.client("exit-second").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if second
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| {
                    viewport_text(&viewport).contains("final-before-exit")
                        && viewport.process_state == TerminalProcessState::Exited { code: Some(7) }
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("reconnected final checkpoint timeout");
    let attached = second
        .request(Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: Some(session_epoch),
            after_sequence: second
                .terminal_snapshot(&session_id)
                .map(|viewport| viewport.sequence),
            mode: TerminalLeaseMode::Observer,
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry: geometry(),
            geometry_epoch: 1,
        }))
        .await
        .unwrap();
    let Response::TerminalAttached { checkpoint, .. } = attached else {
        panic!("unexpected attach response: {attached:?}");
    };
    assert_eq!(
        checkpoint.viewport.process_state,
        TerminalProcessState::Exited { code: Some(7) }
    );
    assert!(viewport_text(&checkpoint.viewport).contains("final-before-exit"));
    let final_sequence = checkpoint.viewport.sequence;
    assert_eq!(
        second
            .request(Request::AcknowledgeTerminalExit {
                session_id,
                session_epoch,
                final_sequence,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::StopIfIdle, false)
            .await,
        LifecycleResponse::Stopping
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn stop_if_idle_is_atomic_and_drain_waits_for_terminal_ack() {
    let host = RunningHost::start().await;
    let client = host.client("lifecycle-blockers").await;
    let mut spec = spawn_spec();
    spec.session_id = TerminalSessionId::new("lifecycle-blocker");
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec!["-lc".to_string(), "exit 0".to_string()],
        return_to_shell: false,
    };
    let session_id = spec.session_id.clone();
    let Response::TerminalSpawned { session_epoch, .. } =
        client.request(Request::SpawnTerminal(spec)).await.unwrap()
    else {
        panic!("unexpected spawn response");
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| {
                    matches!(viewport.process_state, TerminalProcessState::Exited { .. })
                })
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("terminal exit timeout");
    let final_sequence = client.terminal_snapshot(&session_id).unwrap().sequence;

    let busy = host
        .lifecycle_request(LifecycleRequest::StopIfIdle, false)
        .await;
    assert!(matches!(
        busy,
        LifecycleResponse::Busy { blockers }
            if blockers.iter().any(|blocker| matches!(
                blocker,
                yttt_protocol::HostBlocker::ExitedTerminalAwaitingAck {
                    session_id: blocked,
                    session_epoch: blocked_epoch,
                    final_sequence,
                } if blocked == &session_id
                    && *blocked_epoch == session_epoch
                    && *final_sequence != 0
            ))
    ));
    assert!(!host.task.is_finished());

    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    assert!(!host.task.is_finished());
    assert_eq!(
        client
            .request(Request::AcknowledgeTerminalExit {
                session_id,
                session_epoch,
                final_sequence,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("draining Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn force_stop_requires_an_authenticated_lifecycle_capability() {
    let host = RunningHost::start().await;
    let mut ordinary = raw_lifecycle_client(&host, "ordinary-lifecycle-client", false).await;
    assert_eq!(
        raw_lifecycle_request(&mut ordinary, 1, LifecycleRequest::ForceStop).await,
        LifecycleResponse::PermissionDenied
    );
    assert!(!host.task.is_finished());

    let mut privileged = raw_lifecycle_client(&host, "desktop-lifecycle-client", true).await;
    assert_eq!(
        raw_lifecycle_request(&mut privileged, 1, LifecycleRequest::ForceStop).await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("force-stopped Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn lifecycle_channel_survives_resource_build_mismatch_and_is_lifecycle_only() {
    let host = RunningHost::start().await;
    let mut client = raw_lifecycle_client(&host, "compatibility-lifecycle", false).await;
    let LifecycleResponse::Status(status) =
        raw_lifecycle_request(&mut client, 1, LifecycleRequest::Status).await
    else {
        panic!("lifecycle status response was not typed");
    };
    assert_eq!(status.build, host.bootstrap.build);
    assert_eq!(status.lifecycle_protocol, LIFECYCLE_PROTOCOL_VERSION);
    assert_eq!(status.resource_protocol, RESOURCE_PROTOCOL_VERSION);
    assert!(!host.task.is_finished());
    assert_eq!(
        raw_lifecycle_request(&mut client, 2, LifecycleRequest::StopIfIdle).await,
        LifecycleResponse::Stopping
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("compatibility lifecycle Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn independent_host_remains_available_after_all_clients_disconnect() {
    let host = RunningHost::start().await;
    let client = host.client("independent-client").await;
    client.shutdown().await;

    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(60)).await;
    tokio::task::yield_now().await;
    assert!(!host.task.is_finished());
    tokio::time::resume();

    assert_eq!(
        host.lifecycle_request(LifecycleRequest::StopIfIdle, false)
            .await,
        LifecycleResponse::Stopping
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("independent Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn host_rejects_terminal_session_address_collisions() {
    let host = RunningHost::start().await;
    let client = host.client("address-conflict").await;
    let original = spawn_spec();
    client
        .request(Request::SpawnTerminal(original.clone()))
        .await
        .unwrap();

    let mut conflicting = original.clone();
    conflicting.cwd = yttt_protocol::ProjectRelativePath::from_utf8("different-address").unwrap();
    let error = client
        .request(Request::SpawnTerminal(conflicting))
        .await
        .unwrap_err();
    assert!(matches!(
        error,
        ClientCoreError::Protocol(failure)
            if failure.code == FailureCode::AddressConflict && !failure.retryable
    ));

    let terminated = client
        .request(Request::TerminateTerminal {
            session_id: original.session_id.clone(),
            mode: TerminationMode::Terminate,
        })
        .await
        .unwrap();
    let Response::TerminalTerminated(terminated) = terminated else {
        panic!("unexpected terminate response");
    };
    assert_eq!(
        client
            .request(Request::AcknowledgeTerminalExit {
                session_id: original.session_id,
                session_epoch: 1,
                final_sequence: terminated.final_sequence,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
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
    let Response::TerminalSpawned {
        lease,
        session_epoch,
    } = spawned
    else {
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
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client.resource_catalog().is_some_and(|catalog| {
                catalog
                    .terminals
                    .iter()
                    .any(|terminal| terminal.session_id == session_id)
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("event-driven resource catalog add timeout");

    client
        .send_terminal_input(TerminalInput {
            session_id: session_id.clone(),
            context: mutation_context(&client, session_epoch, lease.lease_epoch, 1, 1),
            bytes: b"roundtrip\r".to_vec(),
        })
        .unwrap();
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

    let duplicate_input = client
        .request(Request::TerminalInput(TerminalInput {
            session_id: session_id.clone(),
            context: mutation_context(&client, session_epoch, lease.lease_epoch, 1, 1),
            bytes: b"duplicate\r".to_vec(),
        }))
        .await
        .unwrap_err();
    assert!(matches!(
        duplicate_input,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::StaleSequence
    ));

    let next_context = mutation_context(&client, session_epoch, lease.lease_epoch, 1, 2);
    let mut stale_host = next_context;
    stale_host.host_epoch = stale_host.host_epoch.saturating_sub(1);
    let mut stale_session = next_context;
    stale_session.session_epoch = stale_session.session_epoch.saturating_sub(1);
    let mut stale_lease = next_context;
    stale_lease.lease_epoch = stale_lease.lease_epoch.saturating_sub(1);
    let mut stale_geometry = next_context;
    stale_geometry.geometry_epoch = stale_geometry.geometry_epoch.saturating_sub(1);
    for context in [stale_host, stale_session, stale_lease, stale_geometry] {
        let stale_input = client
            .request(Request::TerminalInput(TerminalInput {
                session_id: session_id.clone(),
                context,
                bytes: b"stale\r".to_vec(),
            }))
            .await
            .unwrap_err();
        assert!(matches!(
            stale_input,
            ClientCoreError::Protocol(failure) if failure.code == FailureCode::StaleEpoch
        ));
    }

    let resized = TerminalGeometry {
        cols: 100,
        rows: 20,
        cell_width: 0,
        cell_height: 0,
    };
    assert_eq!(
        client
            .request(Request::ResizeTerminal(ResizeTerminal {
                session_id: session_id.clone(),
                context: mutation_context(&client, session_epoch, lease.lease_epoch, 2, 2),
                geometry: resized,
            }))
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

    let scroll_response = client
        .request(Request::ScrollTerminal(ScrollTerminal {
            session_id: session_id.clone(),
            context: mutation_context(&client, session_epoch, lease.lease_epoch, 2, 3),
            display_offset: 5,
        }))
        .await
        .unwrap();
    let Response::TerminalScrolled(scroll_read) = scroll_response else {
        panic!("unexpected scroll response: {scroll_response:?}");
    };
    assert_eq!(scroll_read.viewport.display_offset, 5);
    assert_eq!(scroll_read.unseen_output, 0);
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

    let response = client
        .request(Request::TerminateMany {
            requests: vec![TerminateTerminalRequest {
                request_id: 1,
                session_id: session_id.clone(),
            }],
        })
        .await
        .unwrap();
    let Response::TerminalsTerminated { results } = response else {
        panic!("unexpected terminate-many response: {response:?}");
    };
    assert_eq!(results.len(), 1);
    assert_eq!(results[0].request_id, 1);
    assert_eq!(results[0].session_id, session_id);
    assert!(results[0].result.is_ok());
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if client.resource_catalog().is_some_and(|catalog| {
                catalog
                    .terminals
                    .iter()
                    .all(|terminal| terminal.session_id != session_id)
            }) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("event-driven resource catalog removal timeout");
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn multiple_clients_keep_independent_viewports_and_a_single_input_owner() {
    let host = RunningHost::start().await;
    let owner = host.client("viewport-owner").await;
    let mut spec = spawn_spec();
    spec.session_id = TerminalSessionId::new("shared-terminal");
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            concat!(
                "i=1; while [ \"$i\" -le 80 ]; do printf 'line-%s\\n' \"$i\"; i=$((i+1)); done; ",
                "IFS= read -r line; printf 'owner:%s\\n' \"$line\"; sleep 30"
            )
            .to_string(),
        ],
        return_to_shell: false,
    };
    let session_id = spec.session_id.clone();
    let spawned = owner.request(Request::SpawnTerminal(spec)).await.unwrap();
    let Response::TerminalSpawned {
        lease: owner_lease,
        session_epoch,
    } = spawned
    else {
        panic!("unexpected spawn response: {spawned:?}");
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if owner
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport.history_size > 0)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("shared terminal history timeout");

    let observer = host.client("viewport-observer").await;
    let Response::Resources(owner_catalog) = owner.request(Request::ListResources).await.unwrap()
    else {
        panic!("unexpected owner resource catalog response");
    };
    let Response::Resources(observer_catalog) =
        observer.request(Request::ListResources).await.unwrap()
    else {
        panic!("unexpected observer resource catalog response");
    };
    assert_eq!(owner_catalog.profile_id, observer_catalog.profile_id);
    assert_eq!(owner_catalog.host_id, observer_catalog.host_id);
    assert_eq!(owner_catalog.host_epoch, observer_catalog.host_epoch);
    assert!(observer_catalog.revision >= owner_catalog.revision);
    let terminal_ids = |catalog: &yttt_protocol::ResourceCatalog| {
        catalog
            .terminals
            .iter()
            .map(|placement| placement.session_id.clone())
            .collect::<Vec<_>>()
    };
    assert_eq!(terminal_ids(&owner_catalog), vec![session_id.clone()]);
    assert_eq!(terminal_ids(&observer_catalog), vec![session_id.clone()]);
    assert_eq!(
        owner_catalog.ssh_connections,
        observer_catalog.ssh_connections
    );
    assert_eq!(owner_catalog.projects, observer_catalog.projects);
    let attached = observer
        .request(Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: Some(session_epoch),
            after_sequence: observer
                .terminal_snapshot(&session_id)
                .map(|viewport| viewport.sequence),
            mode: TerminalLeaseMode::Observer,
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry: geometry(),
            geometry_epoch: 1,
        }))
        .await
        .unwrap();
    let Response::TerminalAttached {
        lease: observer_lease,
        checkpoint,
    } = attached
    else {
        panic!("unexpected observer attach response: {attached:?}");
    };
    assert_eq!(observer_lease.mode, TerminalLeaseMode::Observer);
    assert_eq!(checkpoint.viewport.display_offset, 0);

    let observer_input = observer
        .request(Request::TerminalInput(TerminalInput {
            session_id: session_id.clone(),
            context: mutation_context(&observer, session_epoch, observer_lease.lease_epoch, 1, 1),
            bytes: b"observer-must-not-write\r".to_vec(),
        }))
        .await
        .unwrap_err();
    assert!(matches!(
        observer_input,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::PermissionDenied
    ));

    let scroll_response = owner
        .request(Request::ScrollTerminal(ScrollTerminal {
            session_id: session_id.clone(),
            context: mutation_context(&owner, session_epoch, owner_lease.lease_epoch, 1, 1),
            display_offset: 5,
        }))
        .await
        .unwrap();
    let Response::TerminalScrolled(scrolled) = scroll_response else {
        panic!("unexpected scroll response: {scroll_response:?}");
    };
    assert!(scrolled.viewport.display_offset > 0);
    assert_eq!(
        observer
            .terminal_snapshot(&session_id)
            .unwrap()
            .display_offset,
        0
    );

    assert_eq!(
        owner
            .request(Request::TerminalInput(TerminalInput {
                session_id: session_id.clone(),
                context: mutation_context(&owner, session_epoch, owner_lease.lease_epoch, 1, 2),
                bytes: b"still-owner\r".to_vec(),
            }))
            .await
            .unwrap(),
        Response::TerminalInputAccepted { client_sequence: 2 }
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if observer
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport_text(&viewport).contains("owner:still-owner"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("observer output timeout");
    assert_eq!(
        owner.terminal_snapshot(&session_id).unwrap().display_offset,
        scrolled.viewport.display_offset
    );

    let bottom_response = owner
        .request(Request::ReadTerminalViewport(ReadTerminalViewport {
            session_id: session_id.clone(),
            session_epoch,
            scrollback_epoch: scrolled.viewport.scrollback_epoch,
            anchor: TerminalViewportAnchor::Bottom,
        }))
        .await
        .unwrap();
    let Response::TerminalViewport(bottom) = bottom_response else {
        panic!("unexpected viewport response: {bottom_response:?}");
    };
    assert_eq!(bottom.viewport.display_offset, 0);
    assert!(bottom.unseen_output > 0);

    let search_response = owner
        .request(Request::SearchTerminal(SearchTerminal {
            session_id: session_id.clone(),
            session_epoch,
            scrollback_epoch: bottom.viewport.scrollback_epoch,
            generation: 2,
            query: "line-40".to_string(),
            case_sensitive: true,
            max_results: 10,
        }))
        .await
        .unwrap();
    let Response::TerminalSearch(results) = search_response else {
        panic!("unexpected search response: {search_response:?}");
    };
    assert_eq!(results.generation, 2);
    let line_id = results.matches.first().expect("line-40 match").line_id;

    let line_response = owner
        .request(Request::ReadTerminalViewport(ReadTerminalViewport {
            session_id: session_id.clone(),
            session_epoch,
            scrollback_epoch: results.scrollback_epoch,
            anchor: TerminalViewportAnchor::LineId(line_id),
        }))
        .await
        .unwrap();
    let Response::TerminalViewport(line_read) = line_response else {
        panic!("unexpected line viewport response: {line_response:?}");
    };
    assert!(viewport_text(&line_read.viewport).contains("line-40"));

    let stale_search = owner
        .request(Request::SearchTerminal(SearchTerminal {
            session_id: session_id.clone(),
            session_epoch,
            scrollback_epoch: results.scrollback_epoch,
            generation: 1,
            query: "line".to_string(),
            case_sensitive: true,
            max_results: 1,
        }))
        .await
        .unwrap_err();
    assert!(matches!(
        stale_search,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::StaleSequence
    ));

    let unknown_line = owner
        .request(Request::ReadTerminalViewport(ReadTerminalViewport {
            session_id: session_id.clone(),
            session_epoch,
            scrollback_epoch: results.scrollback_epoch,
            anchor: TerminalViewportAnchor::LineId(u64::MAX),
        }))
        .await
        .unwrap_err();
    assert!(matches!(
        unknown_line,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::StaleSequence
    ));
    let stale_scrollback = owner
        .request(Request::ReadTerminalViewport(ReadTerminalViewport {
            session_id: session_id.clone(),
            session_epoch,
            scrollback_epoch: results.scrollback_epoch.saturating_add(1),
            anchor: TerminalViewportAnchor::Bottom,
        }))
        .await
        .unwrap_err();
    assert!(matches!(
        stale_scrollback,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::StaleEpoch
    ));

    let conflict = observer
        .request(Request::AcquireTerminalLease {
            session_id: session_id.clone(),
            mode: TerminalLeaseMode::Interactive,
        })
        .await
        .unwrap_err();
    assert!(matches!(
        conflict,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::PermissionDenied
    ));
    use yttt_protocol::session::ProfileControlRequest;
    let Response::ProfileControl(preparing) = observer
        .request(Request::ProfileControl(
            ProfileControlRequest::RequestControl,
        ))
        .await
        .unwrap()
    else {
        panic!("profile transfer")
    };
    let transfer = preparing
        .transfer
        .expect("previous controller must publish");
    owner
        .request(Request::ProfileControl(ProfileControlRequest::Ready {
            transfer_id: transfer.id,
            revisions: Vec::new(),
        }))
        .await
        .unwrap();
    observer
        .request(Request::ProfileControl(ProfileControlRequest::Status))
        .await
        .unwrap();
    let Response::TerminalLease(new_owner_lease) = observer
        .request(Request::AcquireTerminalLease {
            session_id: session_id.clone(),
            mode: TerminalLeaseMode::Interactive,
        })
        .await
        .unwrap()
    else {
        panic!("new controller must attach the existing terminal")
    };
    let former_owner_input = owner
        .request(Request::TerminalInput(TerminalInput {
            session_id: session_id.clone(),
            context: mutation_context(&owner, session_epoch, owner_lease.lease_epoch, 1, 3),
            bytes: b"former-owner\r".to_vec(),
        }))
        .await
        .unwrap_err();
    assert!(matches!(
        former_owner_input,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::PermissionDenied
    ));
    assert_eq!(
        observer
            .request(Request::TerminalInput(TerminalInput {
                session_id: session_id.clone(),
                context: mutation_context(
                    &observer,
                    session_epoch,
                    new_owner_lease.lease_epoch,
                    1,
                    1,
                ),
                bytes: b"new-owner\r".to_vec(),
            }))
            .await
            .unwrap(),
        Response::TerminalInputAccepted { client_sequence: 1 }
    );

    let terminated = observer
        .request(Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        })
        .await
        .unwrap();
    let Response::TerminalTerminated(terminated) = terminated else {
        panic!("unexpected terminate response");
    };
    observer
        .request(Request::AcknowledgeTerminalExit {
            session_id,
            session_epoch,
            final_sequence: terminated.final_sequence,
        })
        .await
        .unwrap();
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn slow_observer_does_not_block_the_owner_or_change_canonical_geometry() {
    let host = RunningHost::start().await;
    let owner = host.client("slow-owner").await;
    let mut spec = spawn_spec();
    spec.session_id = TerminalSessionId::new("slow-observer");
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            "IFS= read -r line; printf 'owner:%s\\n' \"$line\"; sleep 1; yes output | head -n 20000; printf 'burst-done\\n'; sleep 30".to_string(),
        ],
        return_to_shell: false,
    };
    let session_id = spec.session_id.clone();
    let spawned = owner.request(Request::SpawnTerminal(spec)).await.unwrap();
    let Response::TerminalSpawned {
        lease: owner_lease,
        session_epoch,
    } = spawned
    else {
        panic!("unexpected spawn response: {spawned:?}");
    };
    let mut slow = raw_client(&host, "slow-raw-observer").await;
    let mut slow_interactive = raw_terminal_interactive_client(&host, "slow-raw-observer").await;
    let observer_geometry = TerminalGeometry {
        cols: 41,
        rows: 12,
        cell_width: 7,
        cell_height: 14,
    };
    let attached = raw_request(
        &mut slow,
        1,
        Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: Some(session_epoch),
            after_sequence: None,
            mode: TerminalLeaseMode::Observer,
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry: observer_geometry,
            geometry_epoch: 99,
        }),
    )
    .await
    .unwrap();
    let Response::TerminalAttached {
        lease: observer_lease,
        checkpoint,
    } = attached
    else {
        panic!("unexpected observer attach response: {attached:?}");
    };
    assert_eq!(checkpoint.viewport.geometry, geometry());
    let mut slow_data =
        raw_terminal_data_client(&host, "slow-raw-observer", session_id.clone()).await;
    assert!(matches!(
        tokio::time::timeout(Duration::from_secs(5), receive_control(&mut slow_data))
            .await
            .expect("slow observer snapshot timeout")
            .unwrap(),
        ControlMessage::Event(yttt_protocol::HostEvent {
            body: yttt_protocol::ServerEvent::Terminal(
                yttt_protocol::terminal::TerminalStreamUpdate::Snapshot(_)
            ),
            ..
        })
    ));
    let observer_resize = raw_interactive_request(
        &mut slow_interactive,
        2,
        Request::ResizeTerminal(ResizeTerminal {
            session_id: session_id.clone(),
            context: mutation_context(&owner, session_epoch, observer_lease.lease_epoch, 100, 1),
            geometry: observer_geometry,
        }),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(observer_resize.code, FailureCode::PermissionDenied);

    assert_eq!(
        owner
            .request(Request::TerminalInput(TerminalInput {
                session_id: session_id.clone(),
                context: mutation_context(&owner, session_epoch, owner_lease.lease_epoch, 1, 1),
                bytes: b"live-owner\r".to_vec(),
            }))
            .await
            .unwrap(),
        Response::TerminalInputAccepted { client_sequence: 1 }
    );
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if owner
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport_text(&viewport).contains("owner:live-owner"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owner input mirror timeout");
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if owner
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport_text(&viewport).contains("burst-done"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owner stalled behind slow observer");
    assert_eq!(
        owner.terminal_snapshot(&session_id).unwrap().geometry,
        geometry()
    );
    drop(slow_data);

    let terminated = owner
        .request(Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        })
        .await
        .unwrap();
    let Response::TerminalTerminated(terminated) = terminated else {
        panic!("unexpected terminate response");
    };
    owner
        .request(Request::AcknowledgeTerminalExit {
            session_id,
            session_epoch,
            final_sequence: terminated.final_sequence,
        })
        .await
        .unwrap();
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
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
        host.lifecycle_request(LifecycleRequest::ForceStop, true)
            .await,
        LifecycleResponse::Draining
    );
    let bootstrap = host.bootstrap.clone();
    let token = host.token;
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

    let restart_endpoint = host_endpoint(&bootstrap);
    let restarted = tokio::spawn(run(bootstrap.clone(), || {
        LocalListener::bind(restart_endpoint)
    }));
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

    let mut lifecycle =
        raw_lifecycle_client_for(&bootstrap, token, "restart-lifecycle", false).await;
    assert_eq!(
        raw_lifecycle_request(&mut lifecycle, 1, LifecycleRequest::StopIfIdle).await,
        LifecycleResponse::Stopping
    );
    tokio::time::timeout(Duration::from_secs(5), restarted)
        .await
        .expect("restarted Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn host_owns_authenticated_agent_state_and_resyncs_snapshots() {
    let host = RunningHost::start().await;
    let client = host.client("agent-hooks").await;
    let environment_file = host._temp.path().join("agent-environment");
    let session_id = TerminalSessionId::new("agent-terminal");
    let mut spec = spawn_spec();
    spec.session_id = session_id.clone();
    spec.project_id = ProjectId::new("project");
    spec.environment.push((
        "YTTT_TEST_AGENT_ENVIRONMENT_FILE".to_string(),
        environment_file.to_string_lossy().into_owned(),
    ));
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            concat!(
                "printf '%s\\n%s\\n%s\\n' ",
                "\"$YTTT_AGENT_HOOK_ENDPOINT\" ",
                "\"$YTTT_AGENT_HOOK_TOKEN\" ",
                "\"$YTTT_AGENT_HOOK_SCOPE\" ",
                "> \"$YTTT_TEST_AGENT_ENVIRONMENT_FILE\"; sleep 30"
            )
            .to_string(),
        ],
        return_to_shell: false,
    };
    let Response::TerminalSpawned { session_epoch, .. } =
        client.request(Request::SpawnTerminal(spec)).await.unwrap()
    else {
        panic!("unexpected terminal spawn response");
    };

    let environment = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let Ok(contents) = fs::read_to_string(&environment_file) {
                let values = contents.lines().map(str::to_string).collect::<Vec<_>>();
                if values.len() == 3 && values.iter().all(|value| !value.is_empty()) {
                    break values;
                }
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Host-injected Agent environment timeout");
    client.shutdown().await;
    let endpoint = environment[0].strip_prefix("http://").unwrap();
    let body = br#"{"event":"SessionStart","payload":{"session_id":"codex-session-1"}}"#;
    let mut stream = TcpStream::connect(endpoint).unwrap();
    write!(
        stream,
        "POST /hook/codex HTTP/1.1\r\nContent-Length: {}\r\nx-yttt-agent-hook-token: {}\r\nx-yttt-agent-hook-scope: {}\r\nConnection: close\r\n\r\n",
        body.len(),
        environment[1],
        environment[2],
    )
    .unwrap();
    stream.write_all(body).unwrap();
    stream.flush().unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.starts_with("HTTP/1.1 204"), "{response}");

    let observer = host.client("agent-observer").await;
    let Response::AgentSnapshots(observer_replay) = observer
        .request(Request::ReadAgentSnapshots {
            acknowledged: Vec::new(),
        })
        .await
        .unwrap()
    else {
        panic!("unexpected Agent snapshot observer response");
    };
    let [update] = observer_replay.as_slice() else {
        panic!("expected one replayed Agent snapshot: {observer_replay:?}");
    };
    let update = update.clone();
    assert_eq!(update.scope.project_id, "project");
    assert_eq!(update.scope.tab_id, session_id.as_str());
    assert_eq!(update.scope.pane_id, session_id.as_str());
    assert_eq!(update.terminal_session_id, session_id);
    assert_eq!(
        update
            .snapshot
            .session
            .as_ref()
            .and_then(|session| session.session_id.as_deref()),
        Some("codex-session-1")
    );
    assert!(update.sequence > 0);

    let second_observer = host.client("second-agent-observer").await;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if second_observer.agent_snapshots() == vec![update.clone()] {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("automatic Agent snapshot replay timeout");
    let Response::AgentSnapshots(second_replay) = second_observer
        .request(Request::ReadAgentSnapshots {
            acknowledged: Vec::new(),
        })
        .await
        .unwrap()
    else {
        panic!("unexpected second Agent snapshot observer response");
    };
    assert_eq!(second_replay, vec![update.clone()]);
    second_observer.shutdown().await;
    let Response::AgentSnapshots(current) = observer
        .request(Request::ReadAgentSnapshots {
            acknowledged: vec![AgentSnapshotCursor {
                scope: update.scope.clone(),
                host_epoch: update.host_epoch,
                sequence: update.sequence,
            }],
        })
        .await
        .unwrap()
    else {
        panic!("unexpected Agent snapshot acknowledgement response");
    };
    assert!(current.is_empty());
    let attached = observer
        .request(Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: Some(session_epoch),
            after_sequence: observer
                .terminal_snapshot(&session_id)
                .map(|viewport| viewport.sequence),
            mode: yttt_protocol::terminal::TerminalLeaseMode::Interactive,
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry: geometry(),
            geometry_epoch: 2,
        }))
        .await
        .unwrap();
    assert!(
        matches!(attached, Response::TerminalAttached { .. }),
        "unexpected Agent terminal reattach response: {attached:?}"
    );

    let Response::TerminalTerminated(terminated) = observer
        .request(Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        })
        .await
        .unwrap()
    else {
        panic!("unexpected terminal terminate response");
    };
    let Response::AgentSnapshots(after_exit) = observer
        .request(Request::ReadAgentSnapshots {
            acknowledged: Vec::new(),
        })
        .await
        .unwrap()
    else {
        panic!("unexpected Agent snapshot after process termination");
    };
    let after_exit = after_exit
        .iter()
        .find(|update| update.terminal_session_id == session_id)
        .expect("terminated Agent snapshot must remain available for reconnect");
    assert_eq!(
        after_exit.snapshot.process_state,
        yttt_agent_core::AgentProcessState::Exited
    );
    assert_eq!(
        observer
            .request(Request::AcknowledgeTerminalExit {
                session_id,
                session_epoch,
                final_sequence: terminated.final_sequence,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
    let diagnostics_path = host.bootstrap.runtime_root.join("host-diagnostics.jsonl");
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
    let diagnostics = fs::read_to_string(diagnostics_path).unwrap();
    let final_snapshot: HostDiagnosticsSnapshot =
        serde_json::from_str(diagnostics.lines().next_back().unwrap()).unwrap();
    assert_eq!(
        final_snapshot.schema_version,
        yttt_host::diagnostics::DIAGNOSTICS_SCHEMA_VERSION
    );
    assert!(final_snapshot.host_epoch > 0);
    assert!(final_snapshot.rss_bytes.is_some());
    assert!(!final_snapshot.queues.is_empty());
    assert!(
        final_snapshot.queues.iter().all(|queue| queue.current == 0),
        "run-end queue backlog: {:?}",
        final_snapshot.queues
    );
    assert!(
        final_snapshot
            .queues
            .iter()
            .any(|queue| queue.service.samples > 0)
    );
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

    let Response::Project(ProjectResponse::Registered {
        registration_epoch,
        watch_error,
        ..
    }) = client
        .request(Request::Project(ProjectRequest::Register {
            project_id: project_id.clone(),
            root: host_path(&project_root),
            view_id: "test-view".to_string(),
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected project registration response");
    };
    assert_eq!(watch_error, None);
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::StopIfIdle, false)
            .await,
        LifecycleResponse::Busy {
            blockers: vec![yttt_protocol::HostBlocker::Project(project_id.clone())],
        }
    );
    assert!(!host.task.is_finished());

    let Response::Resources(resources) = client.request(Request::ListResources).await.unwrap()
    else {
        panic!("unexpected Host resource catalog response");
    };
    assert!(resources.projects.contains(&project_id));

    let Response::Project(ProjectResponse::File(file)) = client
        .request(Request::Project(ProjectRequest::ReadFile {
            project_id: project_id.clone(),
            relative_path: rel_path(std::path::Path::new("notes.txt")),
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
            relative_path: rel_path(std::path::Path::new("notes.txt")),
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
            relative_path: rel_path(std::path::Path::new("notes.txt")),
            text: "stale".to_string(),
            mode: ProjectSaveMode::Check(saved),
        }))
        .await
        .unwrap()
    else {
        panic!("stale project save did not report a conflict");
    };
    assert_ne!(current.content_hash, 0);
    assert_ne!(current.revision.workspace_epoch, 0);
    assert_ne!(current.revision.revision_number, 0);
    assert_ne!(current.revision.content_sha256, [0; 32]);

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
                view_id: "test-view".to_string(),
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
            relative_directory: rel_path(std::path::Path::new("")),
            show_hidden: false,
        }))
        .await
        .unwrap_err();
    assert!(matches!(
        missing,
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::NotFound
    ));
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn remote_project_operations_require_a_host_registration() {
    let host = RunningHost::start().await;
    let client = host.client("remote-project-registration").await;
    let project_id = ProjectId::new("remote-project");

    let error = client
        .request(Request::RemoteFile(RemoteFileRequest::Read {
            project_id: project_id.clone(),
            relative_path: "notes.txt".to_string(),
            maximum_bytes: 1024,
        }))
        .await
        .unwrap_err();
    let ClientCoreError::Protocol(failure) = error else {
        panic!("unexpected unregistered remote project error: {error}");
    };
    assert_eq!(failure.code, FailureCode::NotFound);
    assert_eq!(
        failure.message,
        "project is not registered with Host: remote-project"
    );

    let Response::Project(ProjectResponse::Registered {
        registration_epoch, ..
    }) = client
        .request(Request::Project(ProjectRequest::RegisterSsh {
            project_id: project_id.clone(),
            connection_id: "connection".to_string(),
            root: remote_rel("/remote"),
            view_id: "test-view".to_string(),
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected remote project registration response");
    };
    let Response::Resources(resources) = client.request(Request::ListResources).await.unwrap()
    else {
        panic!("unexpected resource catalog response");
    };
    assert!(resources.projects.contains(&project_id));

    assert_eq!(
        client
            .request(Request::Project(ProjectRequest::Close {
                view_id: "test-view".to_string(),
                project_id: project_id.clone(),
                registration_epoch,
            }))
            .await
            .unwrap(),
        Response::Project(ProjectResponse::Closed)
    );
    let Response::Resources(resources) = client.request(Request::ListResources).await.unwrap()
    else {
        panic!("unexpected resource catalog response");
    };
    assert!(!resources.projects.contains(&project_id));

    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn terminate_many_reports_each_terminal_result_without_rolling_back_successes() {
    let host = RunningHost::start().await;
    let client = host.client("terminate-many").await;
    let spec = spawn_spec();
    let session_id = spec.session_id.clone();
    let Response::TerminalSpawned { session_epoch, .. } =
        client.request(Request::SpawnTerminal(spec)).await.unwrap()
    else {
        panic!("unexpected spawn response");
    };

    let missing_session_id = TerminalSessionId::new("missing");
    let response = client
        .request(Request::TerminateMany {
            requests: vec![
                TerminateTerminalRequest {
                    request_id: 41,
                    session_id: session_id.clone(),
                },
                TerminateTerminalRequest {
                    request_id: 42,
                    session_id: missing_session_id.clone(),
                },
            ],
        })
        .await
        .unwrap();
    let Response::TerminalsTerminated { results } = response else {
        panic!("unexpected terminate-many response: {response:?}");
    };
    assert_eq!(results.len(), 2);
    assert_eq!(results[0].request_id, 41);
    assert_eq!(results[0].session_id, session_id);
    assert_eq!(
        results[0]
            .result
            .as_ref()
            .expect("live terminal should terminate")
            .session_epoch,
        session_epoch
    );
    assert_eq!(results[1].request_id, 42);
    assert_eq!(results[1].session_id, missing_session_id);
    assert!(matches!(
        &results[1].result,
        Err(failure) if failure.code == FailureCode::PermissionDenied
    ));

    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires the SSH endpoint configured by YTTT_SSH_SMOKE_*"]
async fn host_ssh_product_smoke_covers_host_key_sftp_git_and_terminal() {
    let ssh_host = std::env::var("YTTT_SSH_SMOKE_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let ssh_port = std::env::var("YTTT_SSH_SMOKE_PORT")
        .expect("YTTT_SSH_SMOKE_PORT is required")
        .parse::<u16>()
        .expect("YTTT_SSH_SMOKE_PORT must be a valid port");
    let ssh_username =
        std::env::var("YTTT_SSH_SMOKE_USERNAME").expect("YTTT_SSH_SMOKE_USERNAME is required");
    let ssh_password =
        std::env::var("YTTT_SSH_SMOKE_PASSWORD").expect("YTTT_SSH_SMOKE_PASSWORD is required");

    let host = RunningHost::start().await;
    let client = host.client("ssh-product-smoke").await;
    let mut events = client.subscribe_events();
    let response = client
        .request(Request::SshConnect(SshConnectSpec {
            connection_id: "ssh-smoke".to_string(),
            endpoint: SshEndpoint {
                host: ssh_host,
                port: ssh_port,
                username: ssh_username,
            },
            authentication: SshAuthentication::Password {
                secret: SensitiveBytes::new(ssh_password.into_bytes()),
                save_as: None,
            },
            reconnect: false,
        }))
        .await
        .unwrap();
    assert!(matches!(
        response,
        Response::SshConnected {
            connection_id,
            ..
        } if connection_id == "ssh-smoke"
    ));

    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let ClientEvent::Server(event) = events.recv().await.unwrap() else {
                continue;
            };
            match event.body {
                yttt_protocol::ServerEvent::CredentialChallenge(challenge)
                    if matches!(challenge.kind, CredentialChallengeKind::HostKey { .. }) =>
                {
                    assert_eq!(
                        client
                            .request(Request::CredentialAnswer {
                                challenge_id: challenge.challenge_id,
                                answer: CredentialAnswer::HostKey(HostKeyDecision::AcceptAndStore,),
                            })
                            .await
                            .unwrap(),
                        Response::CredentialAccepted
                    );
                }
                yttt_protocol::ServerEvent::SshStateChanged(status)
                    if status.connection_id == "ssh-smoke"
                        && status.state == SshConnectionState::Connected =>
                {
                    break;
                }
                _ => {}
            }
        }
    })
    .await
    .expect("SSH connection timeout");
    assert!(host.bootstrap.ssh_host_keys_file.exists());

    let Response::RemoteFile(RemoteFileResponse::Home(home)) = client
        .request(Request::RemoteFile(RemoteFileRequest::ResolveHome {
            connection_id: "ssh-smoke".to_string(),
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected SSH home response");
    };
    let project_id = ProjectId::new("ssh-smoke-project");
    let Response::Project(ProjectResponse::Registered {
        registration_epoch, ..
    }) = client
        .request(Request::Project(ProjectRequest::RegisterSsh {
            project_id: project_id.clone(),
            connection_id: "ssh-smoke".to_string(),
            root: remote_rel(&home),
            view_id: "test-view".to_string(),
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected SSH project registration response");
    };
    assert_eq!(
        client
            .request(Request::RemoteFile(RemoteFileRequest::Create {
                project_id: project_id.clone(),
                relative_path: "yttt-smoke".to_string(),
                directory: true,
            }))
            .await
            .unwrap(),
        Response::RemoteFile(RemoteFileResponse::Mutation(
            yttt_protocol::ssh::RemoteEntryMutation {
                relative_path: "yttt-smoke".to_string(),
                kind: yttt_protocol::ssh::RemoteFileKind::Directory,
            }
        ))
    );

    assert!(matches!(
        client
            .request(Request::RemoteFile(RemoteFileRequest::Create {
                project_id: project_id.clone(),
                relative_path: "yttt-smoke/repo".to_string(),
                directory: true,
            }))
            .await
            .unwrap(),
        Response::RemoteFile(RemoteFileResponse::Mutation(_))
    ));
    let Response::RemoteCommand(initialized) = client
        .request(Request::RemoteCommand(RemoteCommandRequest {
            project_id: project_id.clone(),
            command: yttt_protocol::ssh::RemoteHostCommand::Git {
                operation: yttt_protocol::ProjectGitOperation::Init {
                    quiet: true,
                    work_tree: Some(remote_rel("yttt-smoke/repo")),
                },
            },
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected remote git init response");
    };
    assert_eq!(initialized.exit_status, 0, "{initialized:?}");

    let note = b"Host-owned SSH smoke\n".to_vec();
    assert!(matches!(
        client
            .request(Request::RemoteFile(RemoteFileRequest::Save {
                project_id: project_id.clone(),
                relative_path: "yttt-smoke/repo/note.txt".to_string(),
                expected: None,
                force: true,
                maximum_bytes: 1024,
                bytes: note.clone(),
            }))
            .await
            .unwrap(),
        Response::RemoteFile(RemoteFileResponse::Save(
            yttt_protocol::ssh::RemoteSaveResult::Saved(_)
        ))
    ));
    let Response::RemoteFile(RemoteFileResponse::File(saved)) = client
        .request(Request::RemoteFile(RemoteFileRequest::Read {
            project_id: project_id.clone(),
            relative_path: "yttt-smoke/repo/note.txt".to_string(),
            maximum_bytes: 1024,
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected remote file response");
    };
    assert_eq!(saved.bytes, note);

    let Response::RemoteCommand(status) = client
        .request(Request::RemoteCommand(RemoteCommandRequest {
            project_id: project_id.clone(),
            command: yttt_protocol::ssh::RemoteHostCommand::Git {
                operation: yttt_protocol::ProjectGitOperation::Status {
                    work_tree: Some(remote_rel("yttt-smoke/repo")),
                },
            },
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected remote git status response");
    };
    assert_eq!(status.exit_status, 0, "{status:?}");
    let stdout = String::from_utf8(status.stdout).unwrap();
    assert!(stdout.contains("?? note.txt"), "{stdout}");

    let session_id = TerminalSessionId::new("ssh-product-terminal");
    let mut spec = spawn_spec();
    spec.session_id = session_id.clone();
    spec.project_id = ProjectId::new("ssh-product-project");
    spec.cwd = remote_rel(&home);
    spec.execution = TerminalExecutionSpec::Ssh {
        connection_id: "ssh-smoke".to_string(),
        execution: RemoteTerminalExecutionSpec::Command {
            program: "/bin/sh".to_string(),
            args: vec![
                "-lc".to_string(),
                concat!(
                    "printf 'HOST_SSH_TERMINAL_OK\\n'; ",
                    "IFS= read -r line; printf 'HOST_SSH_INPUT:%s\\n' \"$line\"; sleep 30"
                )
                .to_string(),
            ],
        },
    };
    let Response::TerminalSpawned {
        lease,
        session_epoch,
    } = client.request(Request::SpawnTerminal(spec)).await.unwrap()
    else {
        panic!("unexpected SSH terminal spawn response");
    };
    let mirror = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport_text(&viewport).contains("HOST_SSH_TERMINAL_OK"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await;
    if mirror.is_err() {
        let checkpoint = client
            .request(Request::RequestCheckpoint {
                session_id: session_id.clone(),
                after_sequence: None,
            })
            .await;
        panic!(
            "SSH terminal mirror timeout; checkpoint={checkpoint:?}; catalog={:?}",
            client.resource_catalog()
        );
    }
    client
        .send_terminal_input(TerminalInput {
            session_id: session_id.clone(),
            context: mutation_context(&client, session_epoch, lease.lease_epoch, 1, 1),
            bytes: b"parity\r".to_vec(),
        })
        .unwrap();
    tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            if client
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport_text(&viewport).contains("HOST_SSH_INPUT:parity"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("SSH terminal interactive input mirror timeout");
    let diagnostics_path = host.bootstrap.runtime_root.join("host-diagnostics.jsonl");
    let diagnostics = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Ok(contents) = fs::read_to_string(&diagnostics_path)
                && let Some(snapshot) = contents
                    .lines()
                    .filter_map(|line| serde_json::from_str::<HostDiagnosticsSnapshot>(line).ok())
                    .next_back()
                && snapshot.sessions == 1
                && snapshot.attachments == 1
                && snapshot
                    .terminals
                    .iter()
                    .any(|terminal| terminal.session_id == session_id.as_str())
            {
                break snapshot;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("SSH terminal diagnostics timeout");
    assert!(diagnostics.rss_bytes.is_some());
    let terminal = diagnostics
        .terminals
        .iter()
        .find(|terminal| terminal.session_id == session_id.as_str())
        .unwrap();
    assert!(terminal.bytes_parsed > 0);
    assert!(terminal.subscribers > 0);
    assert!(
        terminal
            .queues
            .iter()
            .all(|queue| queue.capacity > 0 && queue.high_water < queue.capacity)
    );

    let Response::TerminalTerminated(terminated) = client
        .request(Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        })
        .await
        .unwrap()
    else {
        panic!("unexpected SSH terminal terminate response");
    };
    assert_eq!(
        client
            .request(Request::AcknowledgeTerminalExit {
                session_id,
                session_epoch: terminated.session_epoch,
                final_sequence: terminated.final_sequence,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
    assert_eq!(
        client
            .request(Request::Project(ProjectRequest::Close {
                view_id: "test-view".to_string(),
                project_id,
                registration_epoch,
            }))
            .await
            .unwrap(),
        Response::Project(ProjectResponse::Closed)
    );
    assert_eq!(
        client
            .request(Request::SshDisconnect {
                connection_id: "ssh-smoke".to_string(),
            })
            .await
            .unwrap(),
        Response::SshDisconnected
    );
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(15), host.task)
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
    let Response::TerminalSpawned { session_epoch, .. } =
        client.request(Request::SpawnTerminal(spec)).await.unwrap()
    else {
        panic!("unexpected performance terminal spawn response");
    };
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

    let Response::TerminalTerminated(terminated) = client
        .request(Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        })
        .await
        .unwrap()
    else {
        panic!("unexpected performance terminal termination response");
    };
    assert_eq!(
        client
            .request(Request::AcknowledgeTerminalExit {
                session_id,
                session_epoch,
                final_sequence: terminated.final_sequence,
            })
            .await
            .unwrap(),
        Response::TerminalExitAcknowledged
    );
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn attach_and_checkpoint_omit_raw_replay_tail_and_recover_visible_output() {
    let host = RunningHost::start().await;
    let first = host.client("checkpoint-owner").await;
    let mut spec = spawn_spec();
    spec.session_id = TerminalSessionId::new("checkpoint-limit");
    spec.execution = TerminalExecutionSpec::Command {
        shell: "/bin/sh".to_string(),
        program: "/bin/sh".to_string(),
        args: vec![
            "-lc".to_string(),
            "printf 'recover-me\\n'; sleep 30".to_string(),
        ],
        return_to_shell: false,
    };
    let session_id = spec.session_id.clone();
    let Response::TerminalSpawned { session_epoch, .. } =
        first.request(Request::SpawnTerminal(spec)).await.unwrap()
    else {
        panic!("unexpected spawn response");
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if first
                .terminal_snapshot(&session_id)
                .is_some_and(|viewport| viewport_text(&viewport).contains("recover-me"))
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("spawned output timeout");
    first
        .request(Request::DetachTerminal {
            session_id: session_id.clone(),
        })
        .await
        .unwrap();
    drop(first);

    let second = host.client("checkpoint-reattach").await;
    let attached = second
        .request(Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: Some(session_epoch),
            after_sequence: second
                .terminal_snapshot(&session_id)
                .map(|viewport| viewport.sequence),
            mode: TerminalLeaseMode::Interactive,
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry: geometry(),
            geometry_epoch: 2,
        }))
        .await
        .unwrap();
    let Response::TerminalAttached { checkpoint, .. } = attached else {
        panic!("unexpected attach response: {attached:?}");
    };
    assert!(checkpoint.raw_replay_tail.is_empty());
    assert!(viewport_text(&checkpoint.viewport).contains("recover-me"));

    let Response::TerminalCheckpoint(requested) = second
        .request(Request::RequestCheckpoint {
            session_id: session_id.clone(),
            after_sequence: None,
        })
        .await
        .unwrap()
    else {
        panic!("unexpected checkpoint response");
    };
    assert!(requested.raw_replay_tail.is_empty());
    assert!(viewport_text(&requested.viewport).contains("recover-me"));
    assert!(
        matches!(second.state(), ConnectionState::Ready { .. }),
        "client disconnected before termination: {:?}",
        second.state()
    );

    let termination = second
        .request(Request::TerminateTerminal {
            session_id: session_id.clone(),
            mode: TerminationMode::Terminate,
        })
        .await;
    let Ok(Response::TerminalTerminated(terminated)) = termination else {
        panic!(
            "unexpected terminate result: {termination:?}; state={:?}",
            second.state()
        );
    };
    second
        .request(Request::AcknowledgeTerminalExit {
            session_id,
            session_epoch,
            final_sequence: terminated.final_sequence,
        })
        .await
        .unwrap();
    drop(second);
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn project_file_limit_is_frame_safe_and_oversized_files_return_resource_limit() {
    let host = RunningHost::start().await;
    let project_root = host._temp.path().join("limit-project");
    fs::create_dir_all(&project_root).unwrap();
    let allowed = "ok".repeat(512 * 1024);
    fs::write(project_root.join("ok.txt"), &allowed).unwrap();
    fs::write(
        project_root.join("too-big.txt"),
        "x".repeat(9 * 1024 * 1024),
    )
    .unwrap();
    let client = host.client("file-limit-client").await;
    let project_id = ProjectId::new("file-limit");

    let Response::Project(ProjectResponse::Registered {
        registration_epoch, ..
    }) = client
        .request(Request::Project(ProjectRequest::Register {
            project_id: project_id.clone(),
            root: host_path(&project_root),
            view_id: "test-view".to_string(),
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected project registration response");
    };

    let Response::Project(ProjectResponse::File(file)) = client
        .request(Request::Project(ProjectRequest::ReadFile {
            project_id: project_id.clone(),
            relative_path: rel_path(std::path::Path::new("ok.txt")),
        }))
        .await
        .unwrap()
    else {
        panic!("in-limit file read failed");
    };
    assert_eq!(file.text, allowed);

    let oversized = client
        .request(Request::Project(ProjectRequest::ReadFile {
            project_id: project_id.clone(),
            relative_path: rel_path(std::path::Path::new("too-big.txt")),
        }))
        .await
        .unwrap_err();
    let ClientCoreError::Protocol(failure) = oversized else {
        panic!("unexpected oversized read error: {oversized}");
    };
    assert_eq!(failure.code, FailureCode::ResourceLimit);
    assert!(failure.message.contains("9437184"));
    assert!(failure.message.contains("6291456"));

    let save_error = client
        .request(Request::Project(ProjectRequest::SaveFile {
            project_id: project_id.clone(),
            relative_path: rel_path(std::path::Path::new("save-too-big.txt")),
            text: "y".repeat(6 * 1024 * 1024 + 1),
            mode: ProjectSaveMode::Force,
        }))
        .await
        .unwrap_err();
    let ClientCoreError::Protocol(failure) = save_error else {
        panic!("unexpected oversized save error: {save_error}");
    };
    assert_eq!(failure.code, FailureCode::ResourceLimit);

    let Response::Project(ProjectResponse::Save(ProjectSaveResult::Saved(_))) = client
        .request(Request::Project(ProjectRequest::SaveFile {
            project_id: project_id.clone(),
            relative_path: rel_path(std::path::Path::new("ok.txt")),
            text: "rewritten".to_string(),
            mode: ProjectSaveMode::Force,
        }))
        .await
        .unwrap()
    else {
        panic!("in-limit save failed");
    };
    let Response::Project(ProjectResponse::File(file)) = client
        .request(Request::Project(ProjectRequest::ReadFile {
            project_id: project_id.clone(),
            relative_path: rel_path(std::path::Path::new("ok.txt")),
        }))
        .await
        .unwrap()
    else {
        panic!("in-limit reread failed");
    };
    assert_eq!(file.text, "rewritten");

    client
        .request(Request::Project(ProjectRequest::Close {
            view_id: "test-view".to_string(),
            project_id,
            registration_epoch,
        }))
        .await
        .unwrap();
    drop(client);
    assert_eq!(
        host.lifecycle_request(LifecycleRequest::BeginDrain, false)
            .await,
        LifecycleResponse::Draining
    );
    tokio::time::timeout(Duration::from_secs(5), host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}
