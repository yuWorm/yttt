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

use yttt_client_core::{ClientCore, ClientCoreError, ClientEvent, ConnectionState, TerminalMirror};
use yttt_core::model::ids::{
    ClientInstanceId, PaneId, ProfileId, ProjectId, TabId, TerminalSessionId,
};
use yttt_host::{HostBootstrap, diagnostics::HostDiagnosticsSnapshot, run};
use yttt_protocol::{
    BuildIdentity, ClientRequest, ControlMessage, FailureCode, LIFECYCLE_PROTOCOL_VERSION,
    LifecycleMessage, LifecycleRequest, LifecycleRequestEnvelope, LifecycleResponse,
    LifecycleResponseEnvelope, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    agent::AgentSnapshotCursor,
    project::{
        PlatformPath, ProjectFileState, ProjectRequest, ProjectResponse, ProjectSaveMode,
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
    AuthToken, ClientIdentity, client_handshake, connect, receive_control, receive_lifecycle,
    send_control, send_lifecycle,
};

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
            build: BuildIdentity {
                product_version: "0.2.0".to_string(),
                build_fingerprint: "integration-build".to_string(),
                resource_compatibility: "integration-resource-v1".to_string(),
            },
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
        .unwrap()
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
    let mut stream = connect(&host.bootstrap.endpoint()).await.unwrap();
    client_handshake(
        &mut stream,
        &ClientIdentity {
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
    let mut stream = connect(&bootstrap.endpoint()).await.unwrap();
    client_handshake(
        &mut stream,
        &ClientIdentity {
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
    let mut stream = connect(&host.bootstrap.endpoint()).await.unwrap();
    client_handshake(
        &mut stream,
        &ClientIdentity {
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

async fn raw_request(
    stream: &mut yttt_transport_local::LocalStream,
    request_id: u64,
    body: Request,
) -> Result<Response, yttt_protocol::ProtocolFailure> {
    send_control(
        stream,
        &ControlMessage::Request(ClientRequest { request_id, body }),
    )
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
        tab_id: TabId::new("tab"),
        pane_id: PaneId::new("pane"),
        cwd: std::env::temp_dir().to_string_lossy().into_owned(),
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
    assert_eq!(
        raw_request(
            &mut control,
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
            mirror.apply(update);
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
    spec.pane_id = PaneId::new("exited-reconnect");
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
    spec.pane_id = PaneId::new("lifecycle-blocker");
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
async fn idle_exit_waits_thirty_seconds_and_new_clients_cancel_the_countdown() {
    let host = RunningHost::start().await;
    let first = host.client("idle-countdown-first").await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::task::yield_now().await;
    assert!(!host.task.is_finished());

    first.shutdown().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(29)).await;
    tokio::task::yield_now().await;
    assert!(!host.task.is_finished());

    tokio::time::resume();
    let second = host.client("idle-countdown-second").await;
    tokio::time::pause();
    tokio::time::advance(Duration::from_secs(31)).await;
    tokio::task::yield_now().await;
    assert!(!host.task.is_finished());

    second.shutdown().await;
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(29)).await;
    tokio::task::yield_now().await;
    assert!(!host.task.is_finished());
    tokio::time::advance(Duration::from_secs(2)).await;
    tokio::time::timeout(Duration::from_secs(1), host.task)
        .await
        .expect("idle Host shutdown timeout")
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
    conflicting.cwd = std::env::temp_dir()
        .join("different-address")
        .to_string_lossy()
        .into_owned();
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

    assert_eq!(
        client
            .request(Request::TerminalInput(TerminalInput {
                session_id: session_id.clone(),
                context: mutation_context(&client, session_epoch, lease.lease_epoch, 1, 1),
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
    spec.pane_id = PaneId::new("shared-terminal");
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

    let mut owner_events = owner.subscribe_events();
    let lease_response = observer
        .request(Request::AcquireTerminalLease {
            session_id: session_id.clone(),
            mode: TerminalLeaseMode::Interactive,
        })
        .await
        .unwrap();
    let Response::TerminalLease(new_owner_lease) = lease_response else {
        panic!("unexpected acquire lease response: {lease_response:?}");
    };
    assert_eq!(new_owner_lease.owner.as_str(), "viewport-observer");
    assert_eq!(new_owner_lease.mode, TerminalLeaseMode::Interactive);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let event = owner_events.recv().await.unwrap();
            if matches!(
                &event,
                ClientEvent::Server(yttt_protocol::HostEvent {
                    body: yttt_protocol::ServerEvent::TerminalLeaseRevoked {
                        session_id,
                        previous_owner,
                    },
                    ..
                }) if *session_id == TerminalSessionId::new("shared-terminal")
                    && previous_owner.as_str() == "viewport-owner"
            ) {
                break;
            }
        }
    })
    .await
    .expect("lease revocation event timeout");
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
    spec.pane_id = PaneId::new("slow-observer");
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
    let observer_resize = raw_request(
        &mut slow,
        2,
        Request::ResizeTerminal(ResizeTerminal {
            session_id: session_id.clone(),
            context: mutation_context(&owner, session_epoch, observer_lease.lease_epoch, 100, 1),
            geometry: observer_geometry,
        }),
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
    spec.tab_id = TabId::new("tab");
    spec.pane_id = PaneId::new("pane");
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
    assert_eq!(update.scope.tab_id, "tab");
    assert_eq!(update.scope.pane_id, "pane");
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
            root: "/remote".to_string(),
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
            root: home.clone(),
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

    let repository = format!("{home}/yttt-smoke/repo");
    let Response::RemoteCommand(initialized) = client
        .request(Request::RemoteCommand(RemoteCommandRequest {
            project_id: project_id.clone(),
            program: "git".to_string(),
            args: vec![
                "init".to_string(),
                "--quiet".to_string(),
                repository.clone(),
            ],
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
            program: "git".to_string(),
            args: vec![
                "-C".to_string(),
                repository,
                "status".to_string(),
                "--porcelain".to_string(),
            ],
        }))
        .await
        .unwrap()
    else {
        panic!("unexpected remote git status response");
    };
    assert_eq!(status.exit_status, 0, "{status:?}");
    assert_eq!(String::from_utf8(status.stdout).unwrap(), "?? note.txt\n");

    let session_id = TerminalSessionId::new("ssh-product-terminal");
    let mut spec = spawn_spec();
    spec.session_id = session_id.clone();
    spec.project_id = ProjectId::new("ssh-product-project");
    spec.cwd = home;
    spec.execution = TerminalExecutionSpec::Ssh {
        connection_id: "ssh-smoke".to_string(),
        execution: RemoteTerminalExecutionSpec::Command {
            program: "/bin/sh".to_string(),
            args: vec![
                "-lc".to_string(),
                "printf 'HOST_SSH_TERMINAL_OK\\n'; sleep 30".to_string(),
            ],
        },
    };
    assert!(matches!(
        client.request(Request::SpawnTerminal(spec)).await.unwrap(),
        Response::TerminalSpawned { .. }
    ));
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
    let diagnostics_path = host.bootstrap.runtime_root.join("host-diagnostics.jsonl");
    let diagnostics = tokio::time::timeout(Duration::from_secs(5), async {
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
