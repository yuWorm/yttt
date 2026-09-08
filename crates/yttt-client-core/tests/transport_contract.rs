use std::{fs, time::Duration};

use tempfile::TempDir;
use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId, ProjectId, TerminalSessionId};
use yttt_host::{HostBootstrap, run};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    terminal::{
        AttachTerminal, TerminalExecutionSpec, TerminalGeometry, TerminalLeaseMode,
        TerminalSpawnSpec,
    },
};
use yttt_transport::{
    AuthToken, ClientIdentity, TransportConnector, TransportListener, memory_pair,
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

struct ContractHost<C> {
    _temp: TempDir,
    bootstrap: HostBootstrap,
    token: [u8; 32],
    connector: C,
    task: tokio::task::JoinHandle<Result<(), yttt_host::HostError>>,
}

impl<C> Drop for ContractHost<C> {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn start_host<L, C>(listener: L, connector: C) -> ContractHost<C>
where
    L: TransportListener,
    C: TransportConnector + Clone,
{
    start_host_with_work(listener, connector, None).await
}

async fn start_host_with_work<L: TransportListener, C: TransportConnector + Clone>(
    listener: L,
    connector: C,
    work: Option<yttt_host::RemoteWorkListener>,
) -> ContractHost<C> {
    let temp = tempfile::tempdir().unwrap();
    let runtime_root = temp.path().join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
    let auth_token_file = runtime_root.join("auth-token");
    let token = [0x3c; 32];
    fs::write(&auth_token_file, token).unwrap();
    #[cfg(unix)]
    fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    let bootstrap = HostBootstrap {
        profile_id: ProfileId::new("transport-contract"),
        runtime_root,
        state_root: temp.path().join("state"),
        config_root: temp.path().join("state/config"),
        auth_token_file,
        ssh_host_keys_file: temp.path().join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.transport-contract".to_string(),
        build: BuildIdentity {
            product_version: "0.2.0".to_string(),
            build_fingerprint: "transport-contract-build".to_string(),
            resource_compatibility: "transport-contract-resource-v1".to_string(),
        },
        lifetime: yttt_host::HostLifetime::Independent,
    };
    let task = tokio::spawn(yttt_host::run_with_remote_work(
        bootstrap.clone(),
        || async move { Ok::<_, yttt_transport::TransportError>((listener, work)) },
    ));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !bootstrap.ready_file().exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Host ready timeout");
    ContractHost {
        _temp: temp,
        bootstrap,
        token,
        connector,
        task,
    }
}

async fn connect_client<C: TransportConnector + Clone>(
    host: &ContractHost<C>,
    id: &str,
) -> ClientCore {
    ClientCore::connect(
        host.connector.clone(),
        ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: host.bootstrap.build.clone(),
            profile_id: host.bootstrap.profile_id.clone(),
            client_instance_id: ClientInstanceId::new(id),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: yttt_protocol::ConnectionChannel::Control,
            terminal_session_id: None,
        },
        AuthToken::from_bytes(host.token),
    )
    .await
    .unwrap()
}

fn contract_spawn_spec() -> TerminalSpawnSpec {
    TerminalSpawnSpec {
        session_id: TerminalSessionId::new("contract-session"),
        project_id: ProjectId::new("contract-project"),
        cwd: yttt_protocol::ProjectRelativePath::root(),
        execution: TerminalExecutionSpec::Command {
            shell: "/bin/sh".to_string(),
            program: "/bin/sh".to_string(),
            args: vec![
                "-lc".to_string(),
                "printf contract-ready; sleep 30".to_string(),
            ],
            return_to_shell: false,
        },
        geometry: TerminalGeometry {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        },
        geometry_epoch: 1,
        query_palette: Vec::new(),
        palette_revision: 1,
        scrollback_limit: 1_000,
        environment: Vec::new(),
        removed_environment: Vec::new(),
    }
}

async fn assert_host_contract<C: TransportConnector + Clone>(host: ContractHost<C>) {
    let owner = connect_client(&host, "contract-owner").await;
    owner
        .request(Request::ProfileControl(
            yttt_protocol::session::ProfileControlRequest::RequestControl,
        ))
        .await
        .unwrap();
    let Response::Resources(catalog) = owner.request(Request::ListResources).await.unwrap() else {
        panic!("ListResources must return a catalog");
    };
    assert_eq!(catalog.profile_id, host.bootstrap.profile_id);
    assert!(catalog.terminals.is_empty());

    let spec = contract_spawn_spec();
    let session_id = spec.session_id.clone();
    let spawned = owner.request(Request::SpawnTerminal(spec)).await.unwrap();
    let Response::TerminalSpawned {
        lease,
        session_epoch,
    } = spawned
    else {
        panic!("unexpected spawn response: {spawned:?}");
    };
    assert_eq!(lease.session_id, session_id);
    assert_eq!(lease.mode, TerminalLeaseMode::Interactive);

    let observer = connect_client(&host, "contract-observer").await;
    let attached = observer
        .request(Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: Some(session_epoch),
            after_sequence: None,
            mode: TerminalLeaseMode::Observer,
            query_palette: Vec::new(),
            palette_revision: 1,
            geometry: TerminalGeometry {
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
            },
            geometry_epoch: 1,
        }))
        .await
        .unwrap();
    let Response::TerminalAttached {
        lease: observer_lease,
        checkpoint,
    } = attached
    else {
        panic!("unexpected attach response: {attached:?}");
    };
    assert_eq!(observer_lease.session_id, session_id);
    assert_eq!(observer_lease.mode, TerminalLeaseMode::Observer);
    assert_eq!(checkpoint.viewport.session_id, session_id);

    let Response::Resources(catalog) = observer.request(Request::ListResources).await.unwrap()
    else {
        panic!("observer catalog must be available");
    };
    let placement = catalog
        .terminals
        .iter()
        .find(|placement| placement.session_id == session_id)
        .expect("catalog must include the shared terminal");
    let placement = serde_json::to_value(placement).unwrap();
    assert!(placement.get("tab_id").is_none());
    assert!(placement.get("pane_id").is_none());

    owner.shutdown().await;
    observer.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn memory_transport_satisfies_host_contract() {
    let (listener, connector) = memory_pair();
    let host = start_host(listener, connector).await;
    assert_host_contract(host).await;
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn local_transport_satisfies_host_contract() {
    use yttt_transport_local::{LocalConnector, LocalEndpoint, LocalListener};

    let temp = tempfile::tempdir().unwrap();
    let runtime_root = temp.path().join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
    let auth_token_file = runtime_root.join("auth-token");
    let token = [0x3d; 32];
    fs::write(&auth_token_file, token).unwrap();
    fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    let bootstrap = HostBootstrap {
        profile_id: ProfileId::new("transport-contract-local"),
        runtime_root: runtime_root.clone(),
        state_root: temp.path().join("state"),
        config_root: temp.path().join("state/config"),
        auth_token_file,
        ssh_host_keys_file: temp.path().join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.transport-contract-local".to_string(),
        build: BuildIdentity {
            product_version: "0.2.0".to_string(),
            build_fingerprint: "transport-contract-local-build".to_string(),
            resource_compatibility: "transport-contract-resource-v1".to_string(),
        },
        lifetime: yttt_host::HostLifetime::Independent,
    };
    let endpoint = LocalEndpoint::for_profile(bootstrap.profile_id.clone(), runtime_root);
    let connector = LocalConnector::new(endpoint.clone());
    let task = tokio::spawn(run(bootstrap.clone(), || LocalListener::bind(endpoint)));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !bootstrap.ready_file().exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Host ready timeout");
    let host = ContractHost {
        _temp: temp,
        bootstrap,
        token,
        connector,
        task,
    };
    assert_host_contract(host).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn a_new_session_cannot_inherit_an_existing_actors_journal_or_terminals() {
    let (listener, connector) = memory_pair();
    let host = start_host(listener, connector).await;
    let owner = connect_client(&host, "bound-owner").await;
    owner
        .request(Request::ProfileControl(
            yttt_protocol::session::ProfileControlRequest::RequestControl,
        ))
        .await
        .unwrap();
    owner
        .request(Request::SpawnTerminal(contract_spawn_spec()))
        .await
        .unwrap();
    let identity = ClientIdentity {
        expected_environment: None,
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: host.bootstrap.build.clone(),
        profile_id: host.bootstrap.profile_id.clone(),
        client_instance_id: ClientInstanceId::new("bound-owner"),
        host_epoch_hint: None,
        can_force_stop: false,
        channel: yttt_protocol::ConnectionChannel::Control,
        terminal_session_id: None,
    };
    let impersonation = ClientCore::connect(
        host.connector.clone(),
        identity,
        AuthToken::from_bytes(host.token),
    )
    .await;
    assert!(impersonation.is_err());
    let Response::Resources(resources) = owner.request(Request::ListResources).await.unwrap()
    else {
        panic!("resource catalog");
    };
    assert_eq!(
        resources.terminals[0].session_id,
        TerminalSessionId::new("contract-session")
    );
    assert!(owner.is_controller());
    owner.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn tls_existing_host_authentication_rotation_and_shutdown_preserve_local_resources() {
    use yttt_protocol::{remote_access::*, session::ProfileControlRequest, workspace::*};
    use yttt_transport_tls::TlsConnector;
    let (listener, connector) = memory_pair();
    let host = start_host(listener, connector).await;
    let local = connect_client(&host, "desktop").await;
    local
        .request(Request::ProfileControl(
            ProfileControlRequest::RequestControl,
        ))
        .await
        .unwrap();
    let workspace_id = WorkspaceId::new("desktop-window").unwrap();
    local
        .request(Request::Workspace(WorkspaceRequest::Register {
            workspace_id: workspace_id.clone(),
            name: "Desktop".into(),
        }))
        .await
        .unwrap();
    local
        .request(Request::Workspace(WorkspaceRequest::Commit {
            workspace_id,
            expected_revision: 0,
            operation_id: WorkspaceOperationId::new("initial").unwrap(),
            snapshot: WorkspaceSnapshot::new(serde_json::json!({"unsaved": "original desktop"}))
                .unwrap(),
            drafts: Vec::new(),
        }))
        .await
        .unwrap();
    let spec = contract_spawn_spec();
    let session_id = spec.session_id.clone();
    local.request(Request::SpawnTerminal(spec)).await.unwrap();
    let epoch = local.control_status().unwrap().context.host_epoch;
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = occupied.local_addr().unwrap();
    assert!(
        local
            .request(Request::RemoteAccess(RemoteAccessRequest::SetEnabled {
                enabled: true,
                listen_address: address,
                confirm_non_loopback: false,
            }))
            .await
            .is_err()
    );
    let Response::RemoteAccess(RemoteAccessResponse::Status(status)) = local
        .request(Request::RemoteAccess(RemoteAccessRequest::Status))
        .await
        .unwrap()
    else {
        panic!("status")
    };
    assert!(!status.settings.enabled);
    assert!(status.bound_address.is_none());
    drop(occupied);
    local
        .request(Request::RemoteAccess(RemoteAccessRequest::SetEnabled {
            enabled: true,
            listen_address: address,
            confirm_non_loopback: false,
        }))
        .await
        .unwrap();
    let Response::RemoteAccess(RemoteAccessResponse::ConnectionInfo(info)) = local
        .request(Request::RemoteAccess(
            RemoteAccessRequest::ExportConnectionInfo,
        ))
        .await
        .unwrap()
    else {
        panic!("connection info")
    };
    let tls = TlsConnector::new(address.to_string(), info.certificate_der.clone()).unwrap();
    let identity = ClientIdentity {
        expected_environment: Some(info.environment_id.clone()),
        credential_generation: info.credential_generation,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: host.bootstrap.build.clone(),
        profile_id: info.profile_id.clone(),
        client_instance_id: ClientInstanceId::new("tls-observer"),
        host_epoch_hint: None,
        can_force_stop: false,
        channel: yttt_protocol::ConnectionChannel::Control,
        terminal_session_id: None,
    };
    let untrusted = rcgen::generate_simple_self_signed(vec![TLS_SERVER_NAME.into()]).unwrap();
    assert!(
        TlsConnector::new(address.to_string(), untrusted.cert.der().to_vec())
            .unwrap()
            .connect()
            .await
            .is_err()
    );
    let mut wrong_identity = identity.clone();
    wrong_identity.expected_environment = Some("wrong-environment".into());
    assert!(
        ClientCore::connect(
            tls.clone(),
            wrong_identity,
            AuthToken::from_bytes(info.work_secret)
        )
        .await
        .is_err()
    );
    assert!(
        ClientCore::connect(
            tls.clone(),
            identity.clone(),
            AuthToken::from_bytes([0; 32])
        )
        .await
        .is_err()
    );
    let mut forged_admin = identity.clone();
    forged_admin.channel = yttt_protocol::ConnectionChannel::DesktopOwner;
    let mut stream = tls.connect().await.unwrap();
    assert!(
        yttt_transport::client_handshake(
            &mut stream,
            &forged_admin,
            &AuthToken::from_bytes(info.work_secret)
        )
        .await
        .is_err()
    );
    let remote = ClientCore::connect(
        tls.clone(),
        identity.clone(),
        AuthToken::from_bytes(info.work_secret),
    )
    .await
    .unwrap();
    assert!(
        remote
            .request(Request::RemoteAccess(
                RemoteAccessRequest::ExportConnectionInfo
            ))
            .await
            .is_err()
    );
    assert!(remote.request(Request::ReadDeviceSettings).await.is_err());
    let Response::Workspace(WorkspaceResponse::Workspaces(index)) = remote
        .request(Request::Workspace(WorkspaceRequest::List))
        .await
        .unwrap()
    else {
        panic!("workspaces")
    };
    assert_eq!(index[0].revision, 1);
    remote
        .request(Request::AttachTerminal(AttachTerminal {
            session_id: session_id.clone(),
            known_session_epoch: None,
            after_sequence: None,
            mode: yttt_protocol::terminal::TerminalLeaseMode::Observer,
            geometry: TerminalGeometry {
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
            },
            geometry_epoch: 1,
            query_palette: Vec::new(),
            palette_revision: 1,
        }))
        .await
        .unwrap();
    let mut admitted_clients = Vec::new();
    for index in 1..MAX_REMOTE_CLIENTS {
        let mut extra = identity.clone();
        extra.client_instance_id = ClientInstanceId::new(format!("bounded-{index}"));
        extra.session_nonce = yttt_transport::new_session_nonce();
        admitted_clients.push(
            ClientCore::connect(tls.clone(), extra, AuthToken::from_bytes(info.work_secret))
                .await
                .unwrap(),
        );
    }
    let mut overflow = identity.clone();
    overflow.client_instance_id = ClientInstanceId::new("over-limit");
    overflow.session_nonce = yttt_transport::new_session_nonce();
    assert!(
        ClientCore::connect(
            tls.clone(),
            overflow,
            AuthToken::from_bytes(info.work_secret)
        )
        .await
        .is_err()
    );
    let mut stalled = Vec::new();
    for _ in 0..MAX_PENDING_HANDSHAKES {
        stalled.push(tokio::net::TcpStream::connect(address).await.unwrap());
    }
    tokio::time::timeout(
        Duration::from_secs(1),
        local.request(Request::ListResources),
    )
    .await
    .expect("stalled TLS handshakes must not block local IPC")
    .unwrap();
    local
        .request(Request::RemoteAccess(RemoteAccessRequest::ResetCredentials))
        .await
        .unwrap();
    drop(stalled);
    for client in admitted_clients {
        client.shutdown().await;
    }
    assert!(
        ClientCore::connect(
            tls.clone(),
            identity.clone(),
            AuthToken::from_bytes(info.work_secret)
        )
        .await
        .is_err()
    );
    let Response::RemoteAccess(RemoteAccessResponse::ConnectionInfo(rotated)) = local
        .request(Request::RemoteAccess(
            RemoteAccessRequest::ExportConnectionInfo,
        ))
        .await
        .unwrap()
    else {
        panic!("rotated connection info")
    };
    assert_eq!(
        rotated.credential_generation,
        info.credential_generation + 1
    );
    let mut fresh_identity = identity;
    fresh_identity.credential_generation = rotated.credential_generation;
    fresh_identity.client_instance_id = ClientInstanceId::new("tls-after-rotation");
    fresh_identity.session_nonce = yttt_transport::new_session_nonce();
    let fresh = ClientCore::connect(
        tls.clone(),
        fresh_identity,
        AuthToken::from_bytes(rotated.work_secret),
    )
    .await
    .unwrap();
    fresh.request(Request::ListResources).await.unwrap();
    let settings_path = host
        .bootstrap
        .state_root
        .join("remote-access/settings.json");
    let backup_path = settings_path.with_extension("saved");
    fs::rename(&settings_path, &backup_path).unwrap();
    fs::create_dir(&settings_path).unwrap();
    local
        .request(Request::RemoteAccess(RemoteAccessRequest::SetEnabled {
            enabled: false,
            listen_address: address,
            confirm_non_loopback: false,
        }))
        .await
        .unwrap();
    assert!(tls.connect().await.is_err());
    let Response::RemoteAccess(RemoteAccessResponse::Status(status)) = local
        .request(Request::RemoteAccess(RemoteAccessRequest::Status))
        .await
        .unwrap()
    else {
        panic!("closed status")
    };
    assert_eq!(status.effective, RemoteAccessState::Disabled);
    assert!(status.clients.is_empty());
    assert!(
        status.settings.enabled,
        "failed persistence must retain the last confirmed preference"
    );
    assert!(
        status.error.is_some(),
        "temporary closure must report the unsaved preference"
    );
    fs::remove_dir(&settings_path).unwrap();
    fs::rename(backup_path, settings_path).unwrap();
    assert_eq!(local.control_status().unwrap().context.host_epoch, epoch);
    let Response::Resources(resources) = local.request(Request::ListResources).await.unwrap()
    else {
        panic!("resources")
    };
    assert!(
        resources
            .terminals
            .iter()
            .any(|terminal| terminal.session_id == session_id)
    );
    remote.shutdown().await;
    fresh.shutdown().await;
    local.shutdown().await;
}

#[tokio::test(flavor = "multi_thread")]
async fn ssh_work_credential_cannot_administer_host_and_survives_tcp_disable() {
    use yttt_protocol::{remote_access::RemoteAccessRequest, session::ProfileControlRequest};
    let (admin_listener, admin_connector) = memory_pair();
    let (work_listener, work_connector) = memory_pair();
    let work_token = [0x93; 32];
    let host = start_host_with_work(
        admin_listener,
        admin_connector,
        Some(yttt_host::RemoteWorkListener::new(
            work_listener,
            AuthToken::from_bytes(work_token),
        )),
    )
    .await;
    let identity = ClientIdentity {
        expected_environment: None,
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: host.bootstrap.build.clone(),
        profile_id: host.bootstrap.profile_id.clone(),
        client_instance_id: ClientInstanceId::new("ssh-work"),
        host_epoch_hint: None,
        can_force_stop: false,
        channel: yttt_protocol::ConnectionChannel::Control,
        terminal_session_id: None,
    };
    assert!(
        ClientCore::connect(
            host.connector.clone(),
            identity.clone(),
            AuthToken::from_bytes(work_token)
        )
        .await
        .is_err()
    );
    assert!(
        ClientCore::connect(
            work_connector.clone(),
            identity.clone(),
            AuthToken::from_bytes(host.token)
        )
        .await
        .is_err()
    );
    let mut forged_owner = identity.clone();
    forged_owner.channel = yttt_protocol::ConnectionChannel::DesktopOwner;
    let mut stream = work_connector.connect().await.unwrap();
    assert!(
        yttt_transport::client_handshake(
            &mut stream,
            &forged_owner,
            &AuthToken::from_bytes(work_token)
        )
        .await
        .is_err()
    );
    let work = ClientCore::connect(work_connector, identity, AuthToken::from_bytes(work_token))
        .await
        .unwrap();
    work.request(Request::ProfileControl(
        ProfileControlRequest::RequestControl,
    ))
    .await
    .unwrap();
    assert!(work.request(Request::ReadDeviceSettings).await.is_err());
    assert!(
        work.request(Request::RemoteAccess(RemoteAccessRequest::Status))
            .await
            .is_err()
    );
    let admin = connect_client(&host, "local-admin").await;
    admin
        .request(Request::RemoteAccess(RemoteAccessRequest::SetEnabled {
            enabled: false,
            listen_address: "127.0.0.1:43123".parse().unwrap(),
            confirm_non_loopback: false,
        }))
        .await
        .unwrap();
    let Response::ProfileControl(status) = work
        .request(Request::ProfileControl(ProfileControlRequest::Status))
        .await
        .unwrap()
    else {
        panic!("control status")
    };
    assert_eq!(status.owner.as_ref(), Some(work.client_id()));
    let Response::Resources(catalog) = work.request(Request::ListResources).await.unwrap() else {
        panic!("SSH work connection must remain usable")
    };
    assert_eq!(catalog.profile_id, host.bootstrap.profile_id);
    work.shutdown().await;
    admin.shutdown().await;
}
