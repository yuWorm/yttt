use std::{ffi::OsString, process::Command};

use tempfile::tempdir;
use yttt::{
    config::profile::{
        AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    },
    host_launcher::{HostLauncher, ProcessRole, process_role},
    model::ids::{ClientInstanceId, ProfileId},
};
use yttt_client_core::{ClientCore, ConnectionState};
use yttt_protocol::{Request, Response};

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

#[tokio::test(flavor = "multi_thread")]
async fn host_role_starts_headless_enforces_single_instance_and_stops_cleanly() {
    let temp = tempdir().unwrap();
    let profile = isolated_profile(temp.path());
    let executable = env!("CARGO_BIN_EXE_yttt");
    let launcher = HostLauncher::new(profile.clone(), executable);
    let (first, second) = tokio::join!(launcher.launch_or_attach(), launcher.launch_or_attach(),);
    let mut processes = vec![first.unwrap(), second.unwrap()];
    assert_ne!(processes[0].spawned(), processes[1].spawned());
    let owner = usize::from(!processes[0].spawned());
    let process = processes.swap_remove(owner);
    drop(processes);
    assert!(process.spawned());
    assert!(process.child_id().is_some());

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
        .arg("--build-id")
        .arg(env!("CARGO_PKG_VERSION"))
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

#[tokio::test(flavor = "multi_thread")]
async fn client_reconnects_after_launcher_recovers_a_crashed_host() {
    let temp = tempdir().unwrap();
    let profile = isolated_profile(temp.path());
    let launcher = HostLauncher::new(profile, env!("CARGO_BIN_EXE_yttt"));
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

    client.shutdown();
    recovered.drain_and_stop().await.unwrap();
}
