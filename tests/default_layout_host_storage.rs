use std::{fs, sync::Arc, time::Duration};
use yttt::{
    config::{
        default_layout::{DefaultLayoutState, DefaultLayoutTemplate},
        paths::AppConfigPaths,
        profile::{
            AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        },
        project_settings::{
            ProjectEditorSettingKey, ProjectEditorSettingValue, ProjectSettingSource,
            load_project_editor_settings_snapshot, project_settings_file,
        },
        settings::EditorSettings,
        storage,
    },
    host_storage::HostStorage,
};
use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, HostPath, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request,
    Response,
    session::ProfileControlRequest,
    workspace::{WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse},
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

// The environment binding is process-wide; keep this real-Host regression in its own test binary.
#[test]
fn missing_default_layout_loads_without_writing_and_updates_through_host_storage() {
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
    let profile = AppProfile::scoped(
        ProfileId::new("default-layout-regression"),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        temporary.path(),
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ProfileDiscovery,
    );
    let bootstrap = yttt_host::HostBootstrap {
        profile_id: ProfileId::new("default-layout-regression"),
        runtime_root,
        state_root: temporary.path().join("state"),
        config_root: config_root.clone(),
        project_config: WorkspaceProjectConfig::Overlay {
            root: HostPath::from_path(&profile.paths().state.join("project-config-overlay"))
                .unwrap(),
        },
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
    let (client, observer, host) = runtime.block_on(async {
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
            ClientCore::connect(
                connector.clone(),
                identity.clone(),
                AuthToken::from_bytes(token),
            )
            .await
            .unwrap(),
        );
        client
            .request(Request::ProfileControl(
                ProfileControlRequest::RequestControl,
            ))
            .await
            .unwrap();
        let mut observer_identity = identity;
        observer_identity.client_instance_id = ClientInstanceId::new("layout-observer");
        observer_identity.session_nonce = yttt_transport::new_session_nonce();
        let observer = Arc::new(
            ClientCore::connect(connector, observer_identity, AuthToken::from_bytes(token))
                .await
                .unwrap(),
        );
        (client, observer, host)
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
        environment.clone(),
        false,
    )))
    .unwrap();
    let paths = AppConfigPaths::from_profile(&profile);
    assert!(!paths.default_layout_file().exists());
    let mut layout = DefaultLayoutState::load(&paths);
    assert!(
        layout.warnings().is_empty(),
        "cold-start layout initialization failed: {:?}",
        layout.warnings()
    );
    assert_eq!(layout.template(), &DefaultLayoutTemplate::builtin());
    assert!(!paths.default_layout_file().exists());
    let mut updated = DefaultLayoutTemplate::builtin();
    updated.tabs[0].title = "Persistent custom shell".into();
    layout.save(updated.clone()).unwrap();
    assert_eq!(DefaultLayoutState::load(&paths).template(), &updated);
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
        DefaultLayoutState::load(&paths).template(),
        &DefaultLayoutTemplate::builtin()
    );

    let project = temporary.path().join("project");
    fs::create_dir_all(&project).unwrap();
    let project_file = project_settings_file(&paths, &project);
    fs::create_dir_all(project_file.parent().unwrap()).unwrap();
    fs::write(&project_file, b"[editor]\ntab_size = 5\n").unwrap();
    storage::read_project_config(
        &project,
        yttt_protocol::workspace::WorkspaceProjectConfigFile::Settings,
        &project_file,
    )
    .unwrap();
    fs::write(&project_file, b"[editor]\ntab_size = 6\n").unwrap();
    assert!(storage::write(&project_file, b"[editor]\ntab_size = 8\n").is_err());
    assert!(storage::remove_file(&project_file).is_err());
    assert_eq!(
        fs::read_to_string(&project_file).unwrap(),
        "[editor]\ntab_size = 6\n"
    );
    storage::read_project_config(
        &project,
        yttt_protocol::workspace::WorkspaceProjectConfigFile::Settings,
        &project_file,
    )
    .unwrap();
    storage::write(&project_file, b"[editor]\ntab_size = 5\n").unwrap();
    let observer_storage = Arc::new(HostStorage::new(
        observer.clone(),
        runtime.handle().clone(),
        config_root.clone(),
        environment,
        true,
    ));
    storage::bind_environment(observer_storage).unwrap();
    let host_defaults = EditorSettings {
        tab_size: 7,
        ..EditorSettings::default()
    };
    let snapshot = load_project_editor_settings_snapshot(&paths, &project, &host_defaults).unwrap();
    let setting = snapshot
        .effective
        .iter()
        .find(|setting| setting.key == ProjectEditorSettingKey::TabSize)
        .unwrap();
    assert_eq!(setting.value, ProjectEditorSettingValue::TabSize(5));
    assert_eq!(setting.source, ProjectSettingSource::Project);
    assert!(storage::write(&project_file, b"[editor]\ntab_size = 9\n").is_err());
    assert_eq!(
        fs::read_to_string(&project_file).unwrap(),
        "[editor]\ntab_size = 5\n"
    );
    assert!(!project.join(".yttt").exists());
    let absent_project = temporary.path().join("absent-project");
    fs::create_dir_all(&absent_project).unwrap();
    let absent_file = project_settings_file(&paths, &absent_project);
    let snapshot =
        load_project_editor_settings_snapshot(&paths, &absent_project, &host_defaults).unwrap();
    let setting = snapshot
        .effective
        .iter()
        .find(|setting| setting.key == ProjectEditorSettingKey::TabSize)
        .unwrap();
    assert_eq!(setting.value, ProjectEditorSettingValue::TabSize(7));
    assert_eq!(setting.source, ProjectSettingSource::Host);
    assert!(!absent_file.parent().unwrap().exists());
    runtime.block_on(observer.shutdown());
    runtime.block_on(client.shutdown());
    host.abort();
}
