use std::{fs, path::Path, time::Duration};
use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    remote_access::{
        RemoteAccessRequest, RemoteAccessResponse, RemoteAccessState, RemoteAccessStatus,
    },
    workspace::WorkspaceProjectConfig,
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

const TOKEN: [u8; 32] = [0x5a; 32];

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
        profile_id: ProfileId::new("remote-access-rebind"),
        runtime_root,
        state_root: root.join("state"),
        config_root: root.join("state/config"),
        project_config: WorkspaceProjectConfig::Project,
        auth_token_file,
        ssh_host_keys_file: root.join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.remote-access-rebind".to_string(),
        build: BuildIdentity {
            product_version: "0.2.0".to_string(),
            build_fingerprint: "remote-access-rebind".to_string(),
            resource_compatibility: "remote-access-rebind-v1".to_string(),
        },
        lifetime: yttt_host::HostLifetime::Independent,
    }
}

/// Starts a Host on its own runtime, which stops every Host task when dropped.
fn start_host(root: &Path) -> (tokio::runtime::Runtime, ClientCore) {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let bootstrap = bootstrap(root);
    let client = runtime.block_on(async {
        let (listener, connector) = memory_pair();
        let ready = bootstrap.ready_file();
        tokio::spawn(yttt_host::run_with_remote_work(
            bootstrap.clone(),
            || async move { Ok::<_, yttt_transport::TransportError>((listener, None)) },
        ));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("Host ready timeout");
        ClientCore::connect(
            connector,
            ClientIdentity {
                expected_environment: None,
                credential_generation: 0,
                session_nonce: yttt_transport::new_session_nonce(),
                supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
                build: bootstrap.build.clone(),
                profile_id: bootstrap.profile_id.clone(),
                client_instance_id: ClientInstanceId::new("rebind-admin"),
                host_epoch_hint: None,
                can_force_stop: false,
                channel: yttt_protocol::ConnectionChannel::Session,
            },
            AuthToken::from_bytes(TOKEN),
        )
        .await
        .unwrap()
    });
    (runtime, client)
}

fn status(runtime: &tokio::runtime::Runtime, client: &ClientCore) -> RemoteAccessStatus {
    let Response::RemoteAccess(RemoteAccessResponse::Status(status)) = runtime
        .block_on(client.request(Request::RemoteAccess(RemoteAccessRequest::Status)))
        .unwrap()
    else {
        panic!("status")
    };
    status
}

#[test]
fn an_enabled_listener_is_reopened_once_its_address_frees_up() {
    let temp = tempfile::tempdir().unwrap();
    let address = std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap();

    let (runtime, client) = start_host(temp.path());
    runtime
        .block_on(
            client.request(Request::RemoteAccess(RemoteAccessRequest::SetEnabled {
                enabled: true,
                listen_address: address,
                confirm_non_loopback: false,
            })),
        )
        .unwrap();
    assert_eq!(status(&runtime, &client).bound_address, Some(address));
    drop(client);
    drop(runtime);

    // The address is taken while the Host restarts with remote access enabled.
    let occupied = std::net::TcpListener::bind(address).unwrap();
    let (runtime, client) = start_host(temp.path());
    let failed = status(&runtime, &client);
    assert_eq!(failed.effective, RemoteAccessState::Failed);
    assert!(failed.error.unwrap().contains("retrying"));

    drop(occupied);
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let current = status(&runtime, &client);
        if current.effective == RemoteAccessState::Listening {
            assert_eq!(current.bound_address, Some(address));
            assert_eq!(current.error, None);
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "listener was not reopened: {current:?}"
        );
        std::thread::sleep(Duration::from_millis(100));
    }
}
