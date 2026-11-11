//! A mobile session against a real Host over P2P: pair, connect, observe a
//! terminal without disturbing the desktop, then take control and type.
//! The Host fixture follows `yttt-client-core/tests/p2p_pairing.rs`.

use std::{fs, path::Path, time::Duration};

use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId, ProjectId, TerminalSessionId};
use yttt_mobile_core::session::{MobileSession, PairedHostRecord, RelayChoice, pair};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    p2p::{DevicePreset, PairingCode},
    project::ProjectRequest,
    remote_access::{PairingState, RemoteAccessRequest, RemoteAccessResponse, RemoteAccessStatus},
    terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalSpawnSpec},
    workspace::WorkspaceProjectConfig,
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt as _;

const TOKEN: [u8; 32] = [0x6c; 32];
const PROFILE: &str = "mobile-session";
const SECRET: [u8; 32] = [41; 32];

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
        profile_id: ProfileId::new(PROFILE),
        runtime_root,
        state_root: root.join("state"),
        config_root: root.join("state/config"),
        project_config: WorkspaceProjectConfig::Project,
        auth_token_file,
        ssh_host_keys_file: root.join("ssh-host-keys.toml"),
        credential_namespace: "dev.yttt.ssh.mobile-session".to_string(),
        build: BuildIdentity {
            product_version: "0.5.0".to_string(),
            build_fingerprint: "mobile-session".to_string(),
            resource_compatibility: "mobile-session-v1".to_string(),
        },
        lifetime: yttt_host::HostLifetime::Independent,
    }
}

/// Starts a Host with P2P on and returns its local administrator, which
/// plays the desktop.
async fn start_host(root: &Path) -> ClientCore {
    let bootstrap = bootstrap(root);
    let (listener, connector) = memory_pair();
    let ready = bootstrap.ready_file();
    let build = bootstrap.build.clone();
    tokio::spawn(yttt_host::run_with_remote_work(bootstrap, || async move {
        Ok::<_, yttt_transport::TransportError>((listener, None))
    }));
    tokio::time::timeout(Duration::from_secs(5), async {
        while !ready.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("Host ready timeout");
    let admin = ClientCore::connect(
        connector,
        ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build,
            profile_id: ProfileId::new(PROFILE),
            client_instance_id: ClientInstanceId::new("desktop"),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: yttt_protocol::ConnectionChannel::Session,
        },
        AuthToken::from_bytes(TOKEN),
    )
    .await
    .unwrap();
    remote_access(
        &admin,
        RemoteAccessRequest::SetP2p {
            enabled: true,
            relay_url: None,
        },
    )
    .await;
    admin
}

async fn remote_access(admin: &ClientCore, request: RemoteAccessRequest) -> RemoteAccessResponse {
    match admin.request(Request::RemoteAccess(request)).await.unwrap() {
        Response::RemoteAccess(response) => response,
        other => panic!("unexpected response {other:?}"),
    }
}

async fn status(admin: &ClientCore) -> RemoteAccessStatus {
    match remote_access(admin, RemoteAccessRequest::Status).await {
        RemoteAccessResponse::Status(status) => status,
        other => panic!("unexpected response {other:?}"),
    }
}

/// Pairs the device the way the app does, the desktop user confirming.
async fn pair_device(admin: &ClientCore) -> PairedHostRecord {
    let RemoteAccessResponse::PairingInvitation { code, .. } =
        remote_access(admin, RemoteAccessRequest::StartPairing).await
    else {
        panic!("expected an invitation");
    };
    assert!(PairingCode::decode(&code.0).is_ok());
    let (shown, shown_rx) = tokio::sync::oneshot::channel();
    let text = code.0.clone();
    let pairing = tokio::spawn(async move {
        pair(
            &text,
            "phone",
            SECRET,
            RelayChoice::Off,
            move |short_code| {
                let _ = shown.send(short_code);
            },
        )
        .await
    });
    let (endpoint_id, short_code) = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(PairingState::AwaitingConfirmation {
                endpoint_id,
                short_code,
                ..
            }) = status(admin).await.p2p.pairing
            {
                return (endpoint_id, short_code);
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the device did not ask to pair");
    assert_eq!(shown_rx.await.unwrap(), short_code);
    remote_access(
        admin,
        RemoteAccessRequest::ConfirmPairing {
            endpoint_id,
            preset: DevicePreset::Full,
        },
    )
    .await;
    pairing.await.unwrap().unwrap()
}

/// The desktop opens a project and starts a shell-like terminal in it.
async fn desktop_terminal(admin: &ClientCore, root: &Path) -> TerminalSessionId {
    let project_root = root.join("project");
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("project");
    admin
        .request(Request::Project(ProjectRequest::Register {
            view_id: "desktop-view".to_string(),
            project_id: project_id.clone(),
            root: yttt_protocol::HostPath::from_path(&project_root).unwrap(),
        }))
        .await
        .unwrap();
    let session_id = TerminalSessionId::new("project:tab:pane");
    let Response::TerminalSpawned { .. } = admin
        .request(Request::SpawnTerminal {
            spec: TerminalSpawnSpec {
                session_id: session_id.clone(),
                project_id,
                cwd: yttt_protocol::ProjectRelativePath::root(),
                execution: TerminalExecutionSpec::Command {
                    shell: "/bin/sh".to_string(),
                    program: "/bin/sh".to_string(),
                    args: vec![
                        "-c".to_string(),
                        "printf 'desktop-ready'; IFS= read -r line; printf '\\ngot:%s\\n' \"$line\"; sleep 30"
                            .to_string(),
                    ],
                    return_to_shell: false,
                },
                geometry: TerminalGeometry {
                    cols: 100,
                    rows: 30,
                    cell_width: 8,
                    cell_height: 16,
                },
                geometry_epoch: 1,
                query_palette: Vec::new(),
                palette_revision: 1,
                scrollback_limit: 1_000,
                environment: Vec::new(),
                removed_environment: Vec::new(),
            },
            start_id: "mobile-session-start".to_string(),
            expected_host_epoch: admin.host_epoch().unwrap(),
        })
        .await
        .unwrap()
    else {
        panic!("unexpected spawn response");
    };
    session_id
}

/// Drains until the accumulated repaints contain `needle`.
async fn drain_until(session: &MobileSession, id: &str, needle: &str) -> Vec<u8> {
    let mut seen = Vec::new();
    tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            if let Some(bytes) = session.drain(id) {
                seen.extend_from_slice(&bytes);
                if String::from_utf8_lossy(&seen).contains(needle) {
                    return;
                }
            }
            tokio::time::sleep(Duration::from_millis(16)).await;
        }
    })
    .await
    .unwrap_or_else(|_| {
        panic!(
            "never saw {needle:?} in {:?}",
            String::from_utf8_lossy(&seen)
        )
    });
    seen
}

async fn controller(admin: &ClientCore) -> Option<ClientInstanceId> {
    match admin
        .request(Request::ReadTerminalController)
        .await
        .unwrap()
    {
        Response::TerminalController(controller) => controller.controller,
        other => panic!("unexpected response {other:?}"),
    }
}

async fn geometry(admin: &ClientCore, id: &TerminalSessionId) -> TerminalGeometry {
    let Response::Resources(catalog) = admin.request(Request::ListResources).await.unwrap() else {
        panic!("expected resources");
    };
    catalog
        .terminals
        .into_iter()
        .find(|terminal| &terminal.session_id == id)
        .expect("terminal in catalog")
        .geometry
}

#[tokio::test(flavor = "multi_thread")]
async fn a_phone_observes_without_disturbing_the_desktop_then_takes_control() {
    let temp = tempfile::tempdir().unwrap();
    let admin = start_host(temp.path()).await;
    let record = pair_device(&admin).await;
    assert_eq!(
        PairedHostRecord::from_bytes(&record.to_bytes()).unwrap(),
        record
    );

    let terminal = desktop_terminal(&admin, temp.path()).await;
    let id = terminal.to_string();
    let phone = MobileSession::connect(&record, SECRET, RelayChoice::Off)
        .await
        .unwrap();
    assert!(
        phone
            .terminals()
            .await
            .unwrap()
            .iter()
            .any(|info| info.session_id == id)
    );

    phone.attach(&id).await.unwrap();
    let first = drain_until(&phone, &id, "desktop-ready").await;
    assert!(
        String::from_utf8_lossy(&first).contains("\x1b[2J"),
        "first drain repaints all"
    );
    assert_eq!(phone.drain(&id), None, "nothing changed since");
    assert_eq!(
        controller(&admin).await,
        Some(ClientInstanceId::new("desktop"))
    );
    assert_eq!(
        (
            geometry(&admin, &terminal).await.cols,
            geometry(&admin, &terminal).await.rows
        ),
        (100, 30)
    );
    assert!(!phone.in_control(&id));
    assert!(phone.send_input(&id, b"x".to_vec()).is_err());

    phone.take_control().await.unwrap();
    assert!(phone.in_control(&id));
    assert_eq!(
        controller(&admin).await.map(|id| id.to_string()),
        Some(phone.client_instance_id().to_string())
    );
    phone.send_input(&id, b"hello\r".to_vec()).unwrap();
    let typed = drain_until(&phone, &id, "got:hello").await;
    assert!(
        !String::from_utf8_lossy(&typed).contains("\x1b[2J"),
        "later drains repaint rows only"
    );

    phone.resize(&id, 50, 20).await.unwrap();
    tokio::time::timeout(Duration::from_secs(5), async {
        while geometry(&admin, &terminal).await.cols != 50 {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the PTY took the phone's size");

    let stats = phone.drain_stats();
    assert!(stats.frames > 0 && stats.bytes > 0);
    let first_id = phone.client_instance_id().to_string();
    assert!(first_id.starts_with("mobile-client-"));
    phone.close().await;

    // A restarted app connects again at once with the same record.
    let again = MobileSession::connect(&record, SECRET, RelayChoice::Off)
        .await
        .unwrap();
    assert_ne!(again.client_instance_id(), first_id);
    assert!(
        again
            .terminals()
            .await
            .unwrap()
            .iter()
            .any(|info| info.session_id == id)
    );
    again.close().await;
}
