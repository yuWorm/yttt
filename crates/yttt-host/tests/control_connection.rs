#![cfg(unix)]

use std::{fs, os::unix::fs::PermissionsExt as _, path::Path, time::Duration};

use tempfile::TempDir;
use yttt_core::model::{
    ids::{ClientInstanceId, ProfileId, ProjectId},
    layout::ProjectLayout,
    project::{ProjectDescriptor, ProjectLocation},
    workspace::Workspace,
};
use yttt_host::{HostBootstrap, HostLifetime, run};
use yttt_protocol::{
    BuildIdentity, ClientRequest, ConnectionChannel, ControlMessage, FailureCode, FrameKind,
    HostBlocker, HostResponse, LIFECYCLE_PROTOCOL_VERSION, LifecycleMessage, LifecycleRequest,
    LifecycleRequestEnvelope, LifecycleResponse, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request,
    Response, ServerEvent, encode_message,
    session::{ControlContext, ControlStatus, ProfileControlRequest, TransferPhase},
    workspace::{
        WorkspaceOperationId, WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse,
        WorkspaceSnapshot,
    },
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
    context: Option<ControlContext>,
) -> Result<Response, yttt_protocol::ProtocolFailure> {
    let mut request = ClientRequest::new(request_id, body);
    request.control = context;
    send_control(stream, &ControlMessage::Request(request))
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

async fn profile_request(
    stream: &mut TransportStream,
    request_id: u64,
    request: ProfileControlRequest,
) -> ControlStatus {
    let Response::ProfileControl(status) =
        control_request(stream, request_id, Request::ProfileControl(request), None)
            .await
            .unwrap()
    else {
        panic!("profile control response")
    };
    status
}

async fn next_profile_event(stream: &mut TransportStream) -> ControlStatus {
    loop {
        if let ControlMessage::Event(event) = receive_control(stream).await.unwrap()
            && let ServerEvent::ProfileControl(status) = event.body
        {
            return status;
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

fn workspace_snapshot_with_project(project_id: ProjectId, root: &Path) -> WorkspaceSnapshot {
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
    let mut workspace = Workspace::new();
    workspace
        .open_project(
            ProjectDescriptor::new(project_id, ProjectLocation::local(root)),
            layout,
        )
        .unwrap();
    WorkspaceSnapshot::new(serde_json::json!({ "workspace": workspace.persisted_state() })).unwrap()
}

#[tokio::test]
async fn reacquire_if_unowned_is_atomic_and_refuses_competing_control() {
    let host = RunningHost::start().await;
    let mut alice = host.control_client("alice").await;
    let initial = profile_request(&mut alice, 1, ProfileControlRequest::Status).await;
    assert!(initial.owner.is_none());
    assert!(initial.transfer.is_none());

    let acquired = profile_request(
        &mut alice,
        2,
        ProfileControlRequest::ReacquireIfUnowned {
            context: initial.context,
        },
    )
    .await;
    assert_eq!(acquired.owner, Some(ClientInstanceId::new("alice")));
    assert!(acquired.transfer.is_none());
    assert_ne!(
        acquired.context.control_epoch,
        initial.context.control_epoch
    );

    let mut bob = host.control_client("bob").await;
    let stale = profile_request(
        &mut bob,
        3,
        ProfileControlRequest::ReacquireIfUnowned {
            context: initial.context,
        },
    )
    .await;
    assert_eq!(stale, acquired);

    let occupied = profile_request(
        &mut bob,
        4,
        ProfileControlRequest::ReacquireIfUnowned {
            context: acquired.context,
        },
    )
    .await;
    assert_eq!(occupied, acquired);

    let pending = profile_request(&mut bob, 5, ProfileControlRequest::RequestControl).await;
    assert_eq!(
        pending.transfer.as_ref().map(|transfer| transfer.phase),
        Some(TransferPhase::Preparing)
    );
    drop(alice);

    let unowned_pending = tokio::time::timeout(Duration::from_secs(5), async {
        let mut request_id = 6;
        loop {
            let status = profile_request(&mut bob, request_id, ProfileControlRequest::Status).await;
            request_id += 1;
            if status.owner.is_none() {
                return status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("owner disconnect");
    assert_eq!(
        unowned_pending
            .transfer
            .as_ref()
            .map(|transfer| transfer.phase),
        Some(TransferPhase::ForceConfirmationRequired)
    );

    let mut carol = host.control_client("carol").await;
    let while_transferring = profile_request(
        &mut carol,
        20,
        ProfileControlRequest::ReacquireIfUnowned {
            context: unowned_pending.context,
        },
    )
    .await;
    assert_eq!(while_transferring, unowned_pending);
}

#[tokio::test]
async fn preparation_deadline_emits_force_confirmation_without_status_polling() {
    let host = RunningHost::start().await;
    let mut owner = host.control_client("owner").await;
    let initial = profile_request(&mut owner, 1, ProfileControlRequest::Status).await;
    let owner_status = profile_request(
        &mut owner,
        2,
        ProfileControlRequest::ReacquireIfUnowned {
            context: initial.context,
        },
    )
    .await;

    let mut events = host.control_client("observer").await;
    assert_eq!(
        next_profile_event(&mut events).await.owner,
        Some(ClientInstanceId::new("owner"))
    );

    let mut requester = host.control_client("requester").await;
    let preparing = profile_request(&mut requester, 3, ProfileControlRequest::RequestControl).await;
    assert_eq!(preparing.owner, owner_status.owner);
    assert_eq!(
        preparing.transfer.as_ref().map(|transfer| transfer.phase),
        Some(TransferPhase::Preparing)
    );
    let transfer_id = preparing
        .transfer
        .as_ref()
        .expect("preparing transfer")
        .id
        .clone();

    let expired = tokio::time::timeout(Duration::from_secs(7), async {
        loop {
            let status = next_profile_event(&mut events).await;
            if status.transfer.as_ref().map(|transfer| transfer.phase)
                == Some(TransferPhase::ForceConfirmationRequired)
            {
                return status;
            }
        }
    })
    .await
    .expect("preparation deadline event");
    assert_eq!(expired.owner, owner_status.owner);
    let automatic = control_request(
        &mut owner,
        4,
        Request::ProfileControl(ProfileControlRequest::Ready {
            transfer_id,
            revisions: Vec::new(),
        }),
        None,
    )
    .await
    .unwrap_err();
    assert_eq!(automatic.code, FailureCode::Conflict);
}

#[tokio::test]
async fn canceled_transfer_deadline_does_not_mutate_its_replacement() {
    let host = RunningHost::start().await;
    let mut owner = host.control_client("owner").await;
    let initial = profile_request(&mut owner, 1, ProfileControlRequest::Status).await;
    let owner_status = profile_request(
        &mut owner,
        2,
        ProfileControlRequest::ReacquireIfUnowned {
            context: initial.context,
        },
    )
    .await;

    let mut first = host.control_client("first").await;
    let preparing = profile_request(&mut first, 3, ProfileControlRequest::RequestControl).await;
    let first_transfer = preparing.transfer.expect("first transfer");
    tokio::time::sleep(Duration::from_millis(4_500)).await;
    let canceled = profile_request(
        &mut first,
        4,
        ProfileControlRequest::Cancel {
            transfer_id: first_transfer.id,
        },
    )
    .await;
    assert!(canceled.transfer.is_none());

    let mut replacement = host.control_client("replacement").await;
    let replacement_status =
        profile_request(&mut replacement, 5, ProfileControlRequest::RequestControl).await;
    let replacement_transfer = replacement_status.transfer.expect("replacement transfer");
    tokio::time::sleep(Duration::from_secs(1)).await;

    let status = profile_request(&mut replacement, 6, ProfileControlRequest::Status).await;
    assert_eq!(status.owner, owner_status.owner);
    assert_eq!(
        status.transfer.as_ref().map(|transfer| &transfer.id),
        Some(&replacement_transfer.id)
    );
    assert_eq!(
        status.transfer.as_ref().map(|transfer| transfer.phase),
        Some(TransferPhase::Preparing)
    );
}

#[tokio::test]
async fn forgetting_last_archived_workspace_releases_project_idle_blocker() {
    let mut host = RunningHost::start().await;
    let project_root = host._temp.path().join("saved-project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("saved-project");
    let workspace_id = yttt_protocol::workspace::WorkspaceId::new("saved-workspace").unwrap();

    let mut owner = host.control_client("workspace-owner").await;
    let initial = profile_request(&mut owner, 1, ProfileControlRequest::Status).await;
    let owner_status = profile_request(
        &mut owner,
        2,
        ProfileControlRequest::ReacquireIfUnowned {
            context: initial.context,
        },
    )
    .await;
    let snapshot = workspace_snapshot_with_project(project_id.clone(), &project_root);
    let Response::Workspace(WorkspaceResponse::Committed { revision, .. }) = control_request(
        &mut owner,
        3,
        Request::Workspace(WorkspaceRequest::Commit {
            workspace_id: workspace_id.clone(),
            expected_revision: 0,
            operation_id: WorkspaceOperationId::new("saved-project").unwrap(),
            snapshot,
            drafts: Vec::new(),
        }),
        Some(owner_status.context),
    )
    .await
    .unwrap() else {
        panic!("workspace commit")
    };

    let mut lifecycle = host.lifecycle_client("lifecycle").await;
    assert_eq!(
        lifecycle_request(&mut lifecycle, 1, LifecycleRequest::StopIfIdle).await,
        LifecycleResponse::Busy {
            blockers: vec![HostBlocker::Project(project_id.clone())],
        }
    );

    let Response::Workspace(WorkspaceResponse::Registered(_)) = control_request(
        &mut owner,
        4,
        Request::Workspace(WorkspaceRequest::SetRestoreOnStartup {
            workspace_id: workspace_id.clone(),
            expected_revision: revision,
            restore: false,
        }),
        Some(owner_status.context),
    )
    .await
    .unwrap() else {
        panic!("workspace close")
    };
    let Response::Workspace(WorkspaceResponse::Forgotten {
        workspace_id: forgotten,
    }) = control_request(
        &mut owner,
        5,
        Request::Workspace(WorkspaceRequest::Forget {
            workspace_id: workspace_id.clone(),
            expected_revision: revision,
        }),
        Some(owner_status.context),
    )
    .await
    .unwrap()
    else {
        panic!("workspace forget")
    };
    assert_eq!(forgotten, workspace_id);

    assert_eq!(
        lifecycle_request(&mut lifecycle, 2, LifecycleRequest::StopIfIdle).await,
        LifecycleResponse::Stopping
    );
    tokio::time::timeout(Duration::from_secs(5), &mut host.task)
        .await
        .expect("Host shutdown timeout")
        .unwrap()
        .unwrap();
}

#[derive(serde::Serialize)]
enum FutureBody {
    SummonDragon { size: u32 },
}

#[derive(serde::Serialize)]
struct FutureRequest {
    request_id: u64,
    control: Option<()>,
    body: FutureBody,
}

#[derive(serde::Serialize)]
enum FutureEnvelope {
    Request(FutureRequest),
}

async fn send_future_request(stream: &mut TransportStream, kind: FrameKind, request_id: u64) {
    let message = FutureEnvelope::Request(FutureRequest {
        request_id,
        control: None,
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
        control_request(&mut control, 42, Request::Ping { sent_millis: 1 }, None).await,
        Ok(Response::Pong { .. })
    ));
}
