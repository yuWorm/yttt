use std::{fs, sync::Arc, time::Duration};
use yttt::{
    config::{
        default_layout::{DefaultLayoutState, DefaultLayoutTemplate},
        paths::AppConfigPaths,
        storage,
    },
    host_storage::HostStorage,
};
use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    session::ProfileControlRequest,
    workspace::{WorkspaceRequest, WorkspaceResponse},
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

// The environment binding is process-wide; keep this real-Host regression in its own test binary.
#[test]
fn missing_default_layout_is_created_and_updated_through_host_storage() {
    let temporary = tempfile::tempdir().unwrap();
    let runtime_root = temporary.path().join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    let token = [0x39; 32];
    let auth_token_file = runtime_root.join("auth-token");
    fs::write(&auth_token_file, token).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&runtime_root, fs::Permissions::from_mode(0o700)).unwrap();
        fs::set_permissions(&auth_token_file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let config_root = temporary.path().join("config");
    let bootstrap = yttt_host::HostBootstrap {
        profile_id: ProfileId::new("default-layout-regression"),
        runtime_root,
        state_root: temporary.path().join("state"),
        config_root: config_root.clone(),
        auth_token_file,
        ssh_host_keys_file: config_root.join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.default-layout-regression".into(),
        build: BuildIdentity {
            product_version: "0.2.0".into(),
            build_fingerprint: "default-layout-regression".into(),
            resource_compatibility: "yttt-resource-v1".into(),
        },
        lifetime: yttt_host::HostLifetime::Independent,
    };
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap();
    let (client, host) = runtime.block_on(async {
        let (listener, connector) = memory_pair();
        let ready = bootstrap.ready_file();
        let identity = ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: bootstrap.build.clone(),
            profile_id: bootstrap.profile_id.clone(),
            client_instance_id: ClientInstanceId::new("layout-client"),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: ConnectionChannel::Control,
            terminal_session_id: None,
        };
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
        let client = Arc::new(
            ClientCore::connect(connector, identity, AuthToken::from_bytes(token))
                .await
                .unwrap(),
        );
        client
            .request(Request::ProfileControl(
                ProfileControlRequest::RequestControl,
            ))
            .await
            .unwrap();
        (client, host)
    });
    let Response::Workspace(WorkspaceResponse::Environment(environment)) = runtime
        .block_on(client.request(Request::Workspace(WorkspaceRequest::Environment)))
        .unwrap()
    else {
        panic!("environment")
    };
    storage::bind_environment(Arc::new(HostStorage::new(
        client.clone(),
        runtime.handle().clone(),
        config_root.clone(),
        environment,
        false,
    )))
    .unwrap();
    let paths = AppConfigPaths::from_config_dir(&config_root);
    assert!(!paths.default_layout_file().exists());
    let mut layout = DefaultLayoutState::load_or_create(&paths);
    assert!(
        layout.warnings().is_empty(),
        "cold-start layout initialization failed: {:?}",
        layout.warnings()
    );
    let saved: DefaultLayoutTemplate =
        toml::from_str(&fs::read_to_string(paths.default_layout_file()).unwrap()).unwrap();
    assert_eq!(saved, DefaultLayoutTemplate::builtin());
    let mut updated = saved;
    updated.tabs[0].title = "Persistent custom shell".into();
    layout.save(updated.clone()).unwrap();
    assert_eq!(
        DefaultLayoutState::load_or_create(&paths).template(),
        &updated
    );
    let confirmed = layout.clone();
    let mut external = updated;
    external.tabs[0].title = "Changed by another editor".into();
    fs::write(
        paths.default_layout_file(),
        toml::to_string_pretty(&external).unwrap(),
    )
    .unwrap();
    assert!(
        layout.reset().is_err(),
        "a stale writer must not overwrite a newer layout"
    );
    assert_eq!(
        layout, confirmed,
        "failed reset must preserve the confirmed UI state"
    );
    let disk: DefaultLayoutTemplate =
        toml::from_str(&fs::read_to_string(paths.default_layout_file()).unwrap()).unwrap();
    assert_eq!(disk, external);
    layout.reload().unwrap();
    assert_eq!(layout.template(), &external);
    layout.reset().unwrap();
    assert_eq!(
        DefaultLayoutState::load_or_create(&paths).template(),
        &DefaultLayoutTemplate::builtin()
    );
    runtime.block_on(client.shutdown());
    host.abort();
}
