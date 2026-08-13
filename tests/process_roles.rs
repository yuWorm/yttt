use std::{ffi::OsString, fs, process::Command, time::Duration};

use tempfile::tempdir;
use yttt::{
    config::profile::{
        AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    },
    host_launcher::{HostLaunchError, HostLauncher, ProcessRole, process_role, run_host_process},
    model::ids::{ClientInstanceId, ProfileId, ProjectId},
};
use yttt_client_core::{ClientCore, ConnectionState};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, LIFECYCLE_PROTOCOL_VERSION, ProtocolRange,
    RESOURCE_PROTOCOL_VERSION, Request, Response,
    project::{PlatformPath, ProjectRequest, ProjectResponse},
};
use yttt_transport_local::{AuthToken, ClientIdentity};

fn isolated_profile(root: &std::path::Path) -> AppProfile {
    AppProfile::scoped(
        ProfileId::new("process-role-test"),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        root.join("profile"),
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ExplicitEndpoint(root.join("profile/runtime/host.sock")),
    )
}
fn installer_profile(root: &std::path::Path) -> AppProfile {
    AppProfile::scoped(
        ProfileId::new("performance"),
        EnvironmentKind::Development,
        ProfilePersistence::Ephemeral,
        root,
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ProfileDiscovery,
    )
}
fn platform_path(path: &std::path::Path) -> PlatformPath {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;
        PlatformPath::Unix(path.as_os_str().as_bytes().to_vec())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;
        PlatformPath::Windows(path.as_os_str().encode_wide().collect())
    }
}
fn test_executable() -> std::path::PathBuf {
    std::env::var_os("YTTT_TEST_EXECUTABLE")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(env!("CARGO_BIN_EXE_yttt")))
}

async fn start_incompatible_host(
    profile: &AppProfile,
    token: [u8; 32],
) -> tokio::task::JoinHandle<Result<(), HostLaunchError>> {
    fs::create_dir_all(&profile.paths().runtime).unwrap();
    fs::create_dir_all(&profile.paths().logs).unwrap();
    let token_file = profile.paths().runtime.join("host-auth-token");
    fs::write(&token_file, token).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(&token_file, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let host = tokio::spawn(run_host_process([
        OsString::from("yttt"),
        OsString::from("--process-role=host"),
        OsString::from("--profile-id"),
        OsString::from(profile.id().as_str()),
        OsString::from("--runtime-root"),
        profile.paths().runtime.clone().into_os_string(),
        OsString::from("--auth-token-file"),
        token_file.into_os_string(),
        OsString::from("--ssh-host-keys-file"),
        profile
            .paths()
            .config
            .join("ssh-host-keys.toml")
            .into_os_string(),
        OsString::from("--credential-namespace"),
        OsString::from(profile.credential_namespace()),
        OsString::from("--product-version"),
        OsString::from("0.1.0"),
        OsString::from("--build-fingerprint"),
        OsString::from("previous-build"),
        OsString::from("--resource-compatibility"),
        OsString::from("previous-resource"),
    ]));
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        assert!(
            !host.is_finished(),
            "incompatible Host exited before readiness"
        );
        if profile.paths().runtime.join("host-ready.json").exists() {
            return host;
        }
        if tokio::time::Instant::now() >= deadline {
            host.abort();
            let _ = host.await;
            panic!("incompatible Host readiness timeout");
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[test]
fn process_role_defaults_to_desktop_and_recognizes_host() {
    assert_eq!(process_role([OsString::from("yttt")]), ProcessRole::Desktop);
    assert_eq!(
        process_role([
            OsString::from("yttt"),
            OsString::from("--process-role=host"),
        ]),
        ProcessRole::Host
    );
}

#[test]
fn desktop_host_cli_starts_reports_reuses_and_stops_the_profile_host() {
    let temporary = tempdir().unwrap();
    let profile_root = temporary.path().join("profile");
    let profile = installer_profile(&profile_root);
    let executable = test_executable();
    let invoke = |argument: &str| {
        Command::new(&executable)
            .arg(argument)
            .env("YTTT_PROFILE_ROOT", &profile_root)
            .output()
            .unwrap()
    };

    let started = invoke("--start-host");
    assert!(started.status.success(), "{started:?}");
    assert_eq!(
        String::from_utf8_lossy(&started.stdout).trim(),
        "Host started"
    );

    let status = invoke("--host-status");
    assert!(status.status.success(), "{status:?}");
    let status_text = String::from_utf8_lossy(&status.stdout);
    assert!(status_text.contains("Host Running"));
    assert!(status_text.contains("0 terminals"));

    let reused = invoke("--start-host");
    assert!(reused.status.success(), "{reused:?}");
    assert_eq!(
        String::from_utf8_lossy(&reused.stdout).trim(),
        "Host already running"
    );

    let stopped = invoke("--stop-host");
    assert!(stopped.status.success(), "{stopped:?}");
    assert_eq!(
        String::from_utf8_lossy(&stopped.stdout).trim(),
        "Host stopped"
    );
    assert!(!profile.paths().runtime.join("host-ready.json").exists());
    assert!(!profile.paths().runtime.join("host.pid").exists());

    let stopped_status = invoke("--host-status");
    assert!(stopped_status.status.success(), "{stopped_status:?}");
    assert_eq!(
        String::from_utf8_lossy(&stopped_status.stdout).trim(),
        "Host stopped"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn installer_preflight_stops_idle_host_and_preserves_busy_host() {
    let temporary = tempdir().unwrap();
    let profile_root = temporary.path().join("profile");
    let profile = installer_profile(&profile_root);
    let executable = test_executable();
    let launcher = HostLauncher::new(profile.clone(), executable.clone());

    let idle_host = launcher.launch_or_attach().await.unwrap();
    assert!(idle_host.spawned());
    let idle_report = temporary.path().join("idle-preflight.txt");
    let idle_status = Command::new(&executable)
        .arg("--installer-host-preflight")
        .arg(&idle_report)
        .env("YTTT_PROFILE_ROOT", &profile_root)
        .status()
        .unwrap();
    assert!(idle_status.success());
    assert_eq!(fs::read_to_string(&idle_report).unwrap(), "stopped\n");
    drop(idle_host);

    let busy_host = launcher.launch_or_attach().await.unwrap();
    assert!(busy_host.spawned());
    let mut client = busy_host.connect().await.unwrap();
    let project_root = temporary.path().join("installer-blocker");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("installer-blocker");
    let Response::Project(ProjectResponse::Registered {
        registration_epoch, ..
    }) = client
        .request(Request::Project(ProjectRequest::Register {
            project_id: project_id.clone(),
            root: platform_path(&project_root),
        }))
        .await
        .unwrap()
    else {
        panic!("Host did not register installer blocker project");
    };

    let busy_report = temporary.path().join("busy-preflight.txt");
    let busy_status = Command::new(&executable)
        .arg("--installer-host-preflight")
        .arg(&busy_report)
        .env("YTTT_PROFILE_ROOT", &profile_root)
        .status()
        .unwrap();
    assert_eq!(busy_status.code(), Some(2));
    let busy_detail = fs::read_to_string(&busy_report).unwrap();
    assert!(busy_detail.contains("busy:"));
    assert!(busy_detail.contains(project_id.as_str()));
    assert!(profile.paths().runtime.join("host-ready.json").exists());

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
    drop(client);
    busy_host.drain_and_stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn host_role_starts_headless_enforces_single_instance_and_stops_cleanly() {
    let temp = tempdir().unwrap();
    let profile = isolated_profile(temp.path());
    let executable = test_executable();
    let launcher = HostLauncher::new(profile.clone(), executable.clone());
    let (first, second) = tokio::join!(launcher.launch_or_attach(), launcher.launch_or_attach(),);
    let mut processes = vec![first.unwrap(), second.unwrap()];
    assert_ne!(processes[0].spawned(), processes[1].spawned());
    let owner = usize::from(!processes[0].spawned());
    let process = processes.swap_remove(owner);
    drop(processes);
    assert!(process.spawned());
    assert!(process.child_id().is_some());
    let ready =
        yttt_host::read_ready_metadata(&profile.paths().runtime.join("host-ready.json")).unwrap();
    assert_eq!(ready.profile_id, *profile.id());
    assert_eq!(ready.build, *launcher.build_identity());
    assert_eq!(ready.resource_protocol, RESOURCE_PROTOCOL_VERSION);
    assert_eq!(ready.lifecycle_protocol, LIFECYCLE_PROTOCOL_VERSION);

    let mut client = process.connect().await.unwrap();
    assert!(matches!(
        client
            .request(Request::Ping { sent_millis: 41 })
            .await
            .unwrap(),
        Response::Pong {
            sent_millis: 41,
            ..
        }
    ));
    match client.request(Request::ListResources).await.unwrap() {
        Response::Resources(catalog) => {
            assert_eq!(catalog.profile_id, *profile.id());
            assert!(catalog.terminals.is_empty());
        }
        response => panic!("unexpected response: {response:?}"),
    }

    let attached = launcher.launch_or_attach().await.unwrap();
    assert!(!attached.spawned());
    drop(attached);

    let duplicate = Command::new(executable)
        .arg("--process-role=host")
        .arg("--profile-id")
        .arg(profile.id().as_str())
        .arg("--runtime-root")
        .arg(&profile.paths().runtime)
        .arg("--auth-token-file")
        .arg(profile.paths().runtime.join("host-auth-token"))
        .arg("--ssh-host-keys-file")
        .arg(profile.paths().config.join("ssh-host-keys.toml"))
        .arg("--credential-namespace")
        .arg(profile.credential_namespace())
        .arg("--product-version")
        .arg(env!("CARGO_PKG_VERSION"))
        .arg("--build-fingerprint")
        .arg(launcher.build_identity().build_fingerprint.as_str())
        .arg("--resource-compatibility")
        .arg(launcher.build_identity().resource_compatibility.as_str())
        .status()
        .unwrap();
    assert!(!duplicate.success());
    drop(process);
    let survivor = launcher.launch_or_attach().await.unwrap();
    assert!(!survivor.spawned());
    survivor.drain_and_stop().await.unwrap();

    assert!(!profile.paths().runtime.join("host-ready.json").exists());
    assert!(!profile.paths().runtime.join("host.pid").exists());
}

#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn unreachable_live_host_lock_prevents_duplicate_spawn() {
    let temp = tempdir().unwrap();
    let profile = isolated_profile(temp.path());
    let launcher = HostLauncher::new(profile.clone(), test_executable());
    let mut process = launcher.launch_or_attach().await.unwrap();
    let pid = process.child_id().unwrap();
    fs::remove_file(launcher.endpoint().unix_path()).unwrap();

    let Err(error) = launcher.launch_or_attach().await else {
        panic!("an unreachable live Host must not be replaced");
    };
    assert!(matches!(
        error,
        HostLaunchError::UnreachableLiveHost {
            pid: Some(actual_pid)
        } if actual_pid == pid
    ));
    assert!(yttt_host::profile_lock_is_held(&profile.paths().runtime).unwrap());

    process.force_stop().unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn incompatible_busy_host_is_preserved_then_idle_host_is_safely_replaced() {
    let temp = tempdir().unwrap();
    let profile = isolated_profile(temp.path());
    let token = [73; 32];
    let old_host = start_incompatible_host(&profile, token).await;
    let launcher = HostLauncher::new(profile.clone(), test_executable());
    let client = ClientCore::connect(
        launcher.endpoint(),
        ClientIdentity {
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: BuildIdentity {
                product_version: "0.1.0".to_string(),
                build_fingerprint: "previous-build".to_string(),
                resource_compatibility: "previous-resource".to_string(),
            },
            profile_id: profile.id().clone(),
            client_instance_id: ClientInstanceId::new("previous-build-client"),
            host_epoch_hint: None,
            can_force_stop: true,
            channel: ConnectionChannel::Control,
            terminal_session_id: None,
        },
        AuthToken::from_bytes(token),
    )
    .await
    .unwrap();
    let project_root = temp.path().join("busy-project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("upgrade-blocker");
    let Response::Project(ProjectResponse::Registered {
        registration_epoch, ..
    }) = client
        .request(Request::Project(ProjectRequest::Register {
            project_id: project_id.clone(),
            root: platform_path(&project_root),
        }))
        .await
        .unwrap()
    else {
        panic!("incompatible Host did not register the blocker project");
    };

    match launcher.launch_or_attach().await {
        Err(HostLaunchError::HostBusy(blockers)) => {
            assert_eq!(
                blockers,
                vec![yttt_protocol::HostBlocker::Project(project_id.clone())]
            );
        }
        _ => panic!("incompatible busy Host was not preserved"),
    }
    assert!(!old_host.is_finished());

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
    client.shutdown().await;
    let replacement = launcher.launch_or_attach().await.unwrap();
    assert!(replacement.spawned());
    tokio::time::timeout(Duration::from_secs(5), old_host)
        .await
        .expect("incompatible Host did not exit after StopIfIdle")
        .unwrap()
        .unwrap();
    replacement.drain_and_stop().await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn client_reconnects_after_launcher_recovers_a_crashed_host() {
    let temp = tempdir().unwrap();
    let profile = isolated_profile(temp.path());
    let launcher = HostLauncher::new(profile, test_executable());
    let mut process = launcher.launch_or_attach().await.unwrap();
    let (endpoint, identity, token) = launcher
        .client_core_config(ClientInstanceId::new("recovery-client"))
        .unwrap();
    let client = ClientCore::connect(endpoint, identity, token)
        .await
        .unwrap();
    let old_epoch = match client.state() {
        ConnectionState::Ready { host_epoch, .. } => host_epoch,
        state => panic!("unexpected initial state: {state:?}"),
    };

    process.force_stop().unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if matches!(
                client.state(),
                ConnectionState::Reconnecting { .. } | ConnectionState::Connecting
            ) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("client did not observe the crashed Host");
    let recovered = launcher.launch_or_attach().await.unwrap();
    assert!(recovered.spawned());
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            if matches!(
                client.state(),
                ConnectionState::Ready { host_epoch, .. } if host_epoch != old_epoch
            ) {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("client did not reconnect to the recovered Host");

    client.shutdown().await;
    recovered.drain_and_stop().await.unwrap();
}
