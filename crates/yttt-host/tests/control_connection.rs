#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt as _, path::Path, time::Duration};

use tempfile::TempDir;
use yttt_core::model::{
    host_workspace::{ProjectPanel, WorkspaceIntent},
    ids::{ClientInstanceId, ProfileId, ProjectId},
    layout::ProjectLayout,
    project::{ProjectDescriptor, ProjectLocation},
};
use yttt_host::{HostBootstrap, HostLifetime, run};
use yttt_protocol::{
    BuildIdentity, ClientRequest, ConnectionChannel, ControlMessage, FailureCode, FrameKind,
    HostBlocker, HostResponse, LIFECYCLE_PROTOCOL_VERSION, LifecycleMessage, LifecycleRequest,
    LifecycleRequestEnvelope, LifecycleResponse, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request,
    Response, ServerEvent, encode_message,
    workspace::{WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse},
};
use yttt_transport::{
    AuthToken, ClientIdentity, MemoryConnector, TransportConnector as _, TransportStream,
    client_handshake, memory_pair, new_session_nonce, receive_control, receive_lifecycle,
    send_control, send_frame, send_lifecycle,
};

struct RunningHost {
    _temp: TempDir,
    bootstrap: HostBootstrap,
    token: [u8; 32],
    connector: MemoryConnector,
    task: tokio::task::JoinHandle<Result<(), yttt_host::HostError>>,
}

impl Drop for RunningHost {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl RunningHost {
    async fn start() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let runtime_root = temp.path().join("runtime");
        fs::create_dir_all(&runtime_root).unwrap();
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        let auth_token_file = runtime_root.join("auth-token");
        let token = [0x6d; 32];
        fs::write(&auth_token_file, token).unwrap();
        fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
        let bootstrap = HostBootstrap {
            profile_id: ProfileId::new("host-control-regression"),
            runtime_root,
            state_root: temp.path().join("state"),
            config_root: temp.path().join("state/config"),
            project_config: WorkspaceProjectConfig::Project,
            auth_token_file,
            ssh_host_keys_file: temp.path().join("ssh-host-keys.toml"),
            credential_namespace: "dev.yttt.host-control-regression".to_string(),
            build: BuildIdentity {
                product_version: "test".to_string(),
                build_fingerprint: "host-control-regression".to_string(),
                resource_compatibility: "host-control-regression".to_string(),
            },
            lifetime: HostLifetime::Independent,
        };
        let (listener, connector) = memory_pair();
        let task = tokio::spawn(run(bootstrap.clone(), || async move {
            Ok::<_, yttt_transport::TransportError>(listener)
        }));
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
            connector,
            task,
        }
    }

    async fn control_client(&self, id: &str) -> TransportStream {
        self.session_client(id, new_session_nonce()).await
    }

    async fn session_client(
        &self,
        id: &str,
        session_nonce: yttt_protocol::Nonce,
    ) -> TransportStream {
        self.client(
            id,
            ConnectionChannel::Session,
            RESOURCE_PROTOCOL_VERSION,
            session_nonce,
        )
        .await
    }

    async fn lifecycle_client(&self, id: &str) -> TransportStream {
        self.client(
            id,
            ConnectionChannel::Lifecycle,
            LIFECYCLE_PROTOCOL_VERSION,
            new_session_nonce(),
        )
        .await
    }

    async fn client(
        &self,
        id: &str,
        channel: ConnectionChannel,
        protocol: u16,
        session_nonce: yttt_protocol::Nonce,
    ) -> TransportStream {
        let mut stream = self.connector.connect().await.unwrap();
        client_handshake(
            &mut stream,
            &ClientIdentity {
                expected_environment: None,
                credential_generation: 0,
                session_nonce,
                supported: ProtocolRange::exact(protocol),
                build: self.bootstrap.build.clone(),
                profile_id: self.bootstrap.profile_id.clone(),
                client_instance_id: ClientInstanceId::new(id),
                host_epoch_hint: None,
                can_force_stop: false,
                channel,
            },
            &AuthToken::from_bytes(self.token),
        )
        .await
        .unwrap();
        stream
    }
}

async fn ping(stream: &mut TransportStream, request_id: u64) -> bool {
    if send_control(
        stream,
        &ControlMessage::Request(ClientRequest::new(
            request_id,
            Request::Ping { sent_millis: 0 },
        )),
    )
    .await
    .is_err()
    {
        return false;
    }
    loop {
        match tokio::time::timeout(Duration::from_secs(5), receive_control(stream)).await {
            Ok(Ok(ControlMessage::Response(response))) if response.request_id == request_id => {
                return matches!(response.result, Ok(Response::Pong { .. }));
            }
            Ok(Ok(_)) => {}
            Ok(Err(_)) => return false,
            Err(_) => panic!("a Session connection neither answered nor closed"),
        }
    }
}

#[tokio::test]
async fn a_session_connection_replaces_its_predecessor_and_refuses_another_session() {
    let host = RunningHost::start().await;
    let nonce = new_session_nonce();
    let mut first = host.session_client("one-connection", nonce).await;
    assert!(ping(&mut first, 1).await);

    let mut second = host.session_client("one-connection", nonce).await;
    assert!(ping(&mut second, 2).await);
    assert!(
        !ping(&mut first, 3).await,
        "the replaced connection must be closed"
    );

    let mut foreign = host
        .session_client("one-connection", new_session_nonce())
        .await;
    assert!(
        !ping(&mut foreign, 4).await,
        "a different authenticated session cannot take over the Client"
    );
    assert!(ping(&mut second, 5).await);
}

async fn control_request(
    stream: &mut TransportStream,
    request_id: u64,
    body: Request,
) -> Result<Response, yttt_protocol::ProtocolFailure> {
    send_control(
        stream,
        &ControlMessage::Request(ClientRequest::new(request_id, body)),
    )
    .await
    .unwrap();
    loop {
        match receive_control(stream).await.unwrap() {
            ControlMessage::Response(response) if response.request_id == request_id => {
                return response.result;
            }
            ControlMessage::Event(_) => {}
            message => panic!("unexpected control message: {message:?}"),
        }
    }
}

async fn lifecycle_request(
    stream: &mut TransportStream,
    request_id: u64,
    body: LifecycleRequest,
) -> LifecycleResponse {
    send_lifecycle(
        stream,
        &LifecycleMessage::Request(LifecycleRequestEnvelope { request_id, body }),
    )
    .await
    .unwrap();
    let LifecycleMessage::Response(response) = receive_lifecycle(stream).await.unwrap() else {
        panic!("lifecycle response")
    };
    assert_eq!(response.request_id, request_id);
    response.result
}

fn open_project_intent(project_id: ProjectId, root: &Path) -> WorkspaceIntent {
    let layout: ProjectLayout = toml::from_str(
        r#"
        [project]
        name = "Saved"

        [[tabs]]
        id = "saved"
        title = "Saved"
        layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }
        "#,
    )
    .unwrap();
    WorkspaceIntent::OpenProject {
        descriptor: ProjectDescriptor::new(project_id, ProjectLocation::local(root)),
        layout,
        panel: ProjectPanel {
            visible: true,
            width: 240.0,
        },
    }
}

#[tokio::test]
async fn forgetting_last_archived_workspace_releases_project_idle_blocker() {
    let mut host = RunningHost::start().await;
    let project_root = host._temp.path().join("saved-project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("saved-project");
    let workspace_id = yttt_protocol::workspace::WorkspaceId::new("saved-workspace").unwrap();

    let mut owner = host.control_client("workspace-owner").await;
    let Response::Workspace(WorkspaceResponse::Applied { .. }) = control_request(
        &mut owner,
        3,
        Request::Workspace(WorkspaceRequest::Apply {
            workspace_id: workspace_id.clone(),
            intent: open_project_intent(project_id.clone(), &project_root),
            expected_revision: None,
        }),
    )
    .await
    .unwrap() else {
        panic!("workspace apply")
    };

    let mut lifecycle = host.lifecycle_client("lifecycle").await;
    // The Host runs the opened project's pane; forgetting the workspace
    // removes both blockers.
    assert_eq!(
        lifecycle_request(&mut lifecycle, 1, LifecycleRequest::StopIfIdle).await,
        LifecycleResponse::Busy {
            blockers: vec![
                HostBlocker::Project(project_id.clone()),
                HostBlocker::RunningTerminal(yttt_core::model::ids::TerminalSessionId::new(
                    "saved-project:saved:shell"
                )),
            ],
        }
    );

    // Starting the pane moved the revision past the intent's.
    let Response::Workspace(WorkspaceResponse::Workspaces(index)) =
        control_request(&mut owner, 4, Request::Workspace(WorkspaceRequest::List))
            .await
            .unwrap()
    else {
        panic!("workspace list")
    };
    let revision = index[0].revision;
    let Response::Workspace(WorkspaceResponse::Registered(_)) = control_request(
        &mut owner,
        5,
        Request::Workspace(WorkspaceRequest::SetRestoreOnStartup {
            workspace_id: workspace_id.clone(),
            expected_revision: revision,
            restore: false,
        }),
    )
    .await
    .unwrap() else {
        panic!("workspace close")
    };
    let Response::Workspace(WorkspaceResponse::Forgotten {
        workspace_id: forgotten,
    }) = control_request(
        &mut owner,
        6,
        Request::Workspace(WorkspaceRequest::Forget {
            workspace_id: workspace_id.clone(),
            expected_revision: revision,
        }),
    )
    .await
    .unwrap()
    else {
        panic!("workspace forget")
    };
    assert_eq!(forgotten, workspace_id);

    // The forgotten pane's terminal ends asynchronously.
    let mut request_id = 1;
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            request_id += 1;
            match lifecycle_request(&mut lifecycle, request_id, LifecycleRequest::StopIfIdle).await
            {
                LifecycleResponse::Stopping => break,
                LifecycleResponse::Busy { blockers } => {
                    assert!(
                        !blockers.contains(&HostBlocker::Project(project_id.clone())),
                        "the forgotten project still blocks: {blockers:?}"
                    );
                }
                response => panic!("unexpected lifecycle response: {response:?}"),
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the forgotten workspace's pane kept the Host busy");
    tokio::time::timeout(Duration::from_secs(5), &mut host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn a_pane_the_host_starts_is_announced_to_watching_clients() {
    let host = RunningHost::start().await;
    let project_root = host._temp.path().join("announced-project");
    fs::create_dir_all(&project_root).unwrap();
    let workspace_id = yttt_protocol::workspace::WorkspaceId::new("announced").unwrap();

    let mut owner = host.control_client("pane-owner").await;
    let mut watcher = host.control_client("pane-watcher").await;
    assert!(ping(&mut watcher, 1).await);

    // The opened project's pane is started by the Host, not by a terminal
    // request, so only the catalog event tells the watcher to attach.
    let Response::Workspace(WorkspaceResponse::Applied { .. }) = control_request(
        &mut owner,
        3,
        Request::Workspace(WorkspaceRequest::Apply {
            workspace_id,
            intent: open_project_intent(ProjectId::new("announced-project"), &project_root),
            expected_revision: None,
        }),
    )
    .await
    .unwrap() else {
        panic!("workspace apply")
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if let ControlMessage::Event(event) = receive_control(&mut watcher).await.unwrap()
                && matches!(event.body, ServerEvent::ResourceCatalogChanged)
            {
                break;
            }
        }
    })
    .await
    .expect("the started pane was not announced");
    let Response::Resources(catalog) = control_request(&mut watcher, 2, Request::ListResources)
        .await
        .unwrap()
    else {
        panic!("resource catalog")
    };
    assert_eq!(
        catalog
            .terminals
            .iter()
            .map(|terminal| terminal.session_id.as_str())
            .collect::<Vec<_>>(),
        ["announced-project:saved:shell"]
    );
}

#[derive(serde::Serialize)]
enum FutureBody {
    SummonDragon { size: u32 },
}

#[derive(serde::Serialize)]
struct FutureRequest {
    request_id: u64,
    body: FutureBody,
}

#[derive(serde::Serialize)]
enum FutureEnvelope {
    Request(FutureRequest),
}

async fn send_future_request(stream: &mut TransportStream, kind: FrameKind, request_id: u64) {
    let message = FutureEnvelope::Request(FutureRequest {
        request_id,
        body: FutureBody::SummonDragon { size: 3 },
    });
    send_frame(stream, &encode_message(kind, &message).unwrap())
        .await
        .unwrap();
}

fn assert_unsupported(response: HostResponse, request_id: u64) {
    assert_eq!(response.request_id, request_id);
    assert!(matches!(
        response.result,
        Err(failure) if failure.code == FailureCode::Unsupported
    ));
}

#[tokio::test]
async fn requests_from_a_newer_client_are_unsupported_without_dropping_the_connection() {
    let host = RunningHost::start().await;

    let mut control = host.control_client("newer-client").await;
    send_future_request(&mut control, FrameKind::Control, 41).await;
    let response = loop {
        match receive_control(&mut control).await.unwrap() {
            ControlMessage::Response(response) => break response,
            ControlMessage::Event(_) => {}
            message => panic!("unexpected control message: {message:?}"),
        }
    };
    assert_unsupported(response, 41);
    assert!(matches!(
        control_request(&mut control, 42, Request::Ping { sent_millis: 1 }).await,
        Ok(Response::Pong { .. })
    ));
}
