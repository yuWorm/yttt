use std::{fs, sync::Arc, time::Duration};
use yttt::{config::storage::ConfigStorage, host_storage::HostStorage};
use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, ProjectRelativePath, ProtocolRange,
    RESOURCE_PROTOCOL_VERSION, Request, Response,
    workspace::{WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse},
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

fn identity(id: &str, build: &BuildIdentity) -> ClientIdentity {
    ClientIdentity {
        expected_environment: None,
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: build.clone(),
        profile_id: ProfileId::new("remote-config-cache"),
        client_instance_id: ClientInstanceId::new(id),
        host_epoch_hint: None,
        can_force_stop: false,
        channel: ConnectionChannel::Session,
    }
}

/// Waits until `read` returns `expected`, as a background refresh lands.
fn eventually(storage: &HostStorage, path: &std::path::Path, expected: &[u8]) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while storage.read(path).ok().as_deref() != Some(expected) {
        assert!(
            std::time::Instant::now() < deadline,
            "cached configuration never refreshed to {:?}",
            String::from_utf8_lossy(expected)
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn remote_configuration_reads_are_served_from_a_refreshing_cache() {
    let temporary = tempfile::tempdir().unwrap();
    let runtime_root = temporary.path().join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    let token = [0x41; 32];
    let auth_token_file = runtime_root.join("auth-token");
    fs::write(&auth_token_file, token).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let config_root = temporary.path().join("config");
    fs::create_dir_all(&config_root).unwrap();
    let settings = config_root.join("settings.toml");
    fs::write(&settings, b"# first\n").unwrap();
    let build = BuildIdentity {
        product_version: "0.2.0".into(),
        build_fingerprint: "remote-config-cache".into(),
        resource_compatibility: "yttt-resource-v1".into(),
    };
    let bootstrap = yttt_host::HostBootstrap {
        profile_id: ProfileId::new("remote-config-cache"),
        runtime_root,
        state_root: temporary.path().join("state"),
        config_root: config_root.clone(),
        project_config: WorkspaceProjectConfig::Overlay {
            root: yttt_protocol::HostPath::from_path(&temporary.path().join("overlay")).unwrap(),
        },
        auth_token_file,
        ssh_host_keys_file: config_root.join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.remote-config-cache".into(),
        build: build.clone(),
        lifetime: yttt_host::HostLifetime::Independent,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (client, other, host) = runtime.block_on(async {
        let (listener, connector) = memory_pair();
        let ready = bootstrap.ready_file();
        let host = tokio::spawn(yttt_host::run_with_remote_work(bootstrap, || async move {
            Ok::<_, yttt_transport::TransportError>((listener, None))
        }));
        tokio::time::timeout(Duration::from_secs(5), async {
            while !ready.exists() {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let client = ClientCore::connect(
            connector.clone(),
            identity("cache-client", &build),
            AuthToken::from_bytes(token),
        )
        .await
        .unwrap();
        let other = ClientCore::connect(
            connector,
            identity("other-client", &build),
            AuthToken::from_bytes(token),
        )
        .await
        .unwrap();
        (Arc::new(client), other, host)
    });
    let Response::Workspace(WorkspaceResponse::Environment(environment)) = runtime
        .block_on(client.request(Request::Workspace(WorkspaceRequest::Environment)))
        .unwrap()
    else {
        panic!("environment")
    };
    let storage = HostStorage::new(
        client.clone(),
        runtime.handle().clone(),
        config_root.clone(),
        environment,
        true,
    );
    assert_eq!(storage.read(&settings).unwrap(), b"# first\n");

    // Another Client's write is announced and refreshes the cache.
    let current = runtime
        .block_on(
            other.request(Request::Workspace(WorkspaceRequest::ReadConfig {
                relative_path: ProjectRelativePath::from_path(std::path::Path::new(
                    "settings.toml",
                ))
                .unwrap(),
            })),
        )
        .unwrap();
    let Response::Workspace(WorkspaceResponse::Config(Some(current))) = current else {
        panic!("settings");
    };
    runtime
        .block_on(
            other.request(Request::Workspace(WorkspaceRequest::WriteConfig {
                relative_path: current.relative_path,
                expected_revision: Some(current.revision),
                bytes: b"# second\n".to_vec(),
            })),
        )
        .unwrap();
    eventually(&storage, &settings, b"# second\n");

    // An edit on the Host's disk is picked up by the periodic refresh.
    fs::write(&settings, b"# third\n").unwrap();
    std::thread::sleep(Duration::from_millis(2_100));
    eventually(&storage, &settings, b"# third\n");

    // A local write after a refresh uses the newest revision.
    storage.write(&settings, b"# fourth\n").unwrap();
    assert_eq!(fs::read(&settings).unwrap(), b"# fourth\n");
    assert_eq!(storage.read(&settings).unwrap(), b"# fourth\n");

    // Without a connection, cached reads still answer at once.
    runtime.block_on(client.shutdown());
    let started = std::time::Instant::now();
    assert_eq!(storage.read(&settings).unwrap(), b"# fourth\n");
    assert!(started.elapsed() < Duration::from_millis(100));

    runtime.block_on(other.shutdown());
    host.abort();
}
