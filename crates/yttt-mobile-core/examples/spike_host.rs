//! Spike: a throwaway Host for trying the mobile link end to end without the
//! desktop app or a real profile. It runs `yttt-host` on a temporary profile,
//! turns P2P on without a relay, prints pairing codes and confirms every device
//! that asks, and starts two terminals: a clean zsh and a demo that prints
//! colors and CJK text.
//!
//! ```text
//! cargo run -p yttt-mobile-core --example spike_host
//! ```
//!
//! The current pairing code is also written to
//! `target/mobile-spike-host/pairing-code`. Everything is deleted on exit.

use std::{fs, path::Path, time::Duration};

use yttt_client_core::ClientCore;
use yttt_core::model::ids::{ClientInstanceId, ProfileId, ProjectId, TerminalSessionId};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    p2p::DevicePreset,
    project::ProjectRequest,
    remote_access::{PairingState, RemoteAccessRequest, RemoteAccessResponse},
    terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalSpawnSpec},
    workspace::WorkspaceProjectConfig,
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

const TOKEN: [u8; 32] = [0x7d; 32];
const PROFILE: &str = "mobile-spike-host";

const DEMO: &str = r#"printf '\033[1;32mspike host\033[0m  \033[33m黄色\033[0m \033[38;5;208m208\033[0m \033[38;2;80;160;255mrgb\033[0m\n'
printf '中文宽字符：终端镜像 ✓  emoji 😀\n'
i=0; while true; do i=$((i+1)); printf '\r\033[2Ktick %d  %s' "$i" "$(date +%T)"; sleep 1; done"#;

fn bootstrap(root: &Path) -> yttt_host::HostBootstrap {
    let runtime_root = root.join("runtime");
    fs::create_dir_all(&runtime_root).unwrap();
    let auth_token_file = runtime_root.join("auth-token");
    fs::write(&auth_token_file, TOKEN).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
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
        credential_namespace: "dev.yttt.ssh.mobile-spike-host".to_string(),
        build: BuildIdentity {
            product_version: env!("CARGO_PKG_VERSION").into(),
            build_fingerprint: "mobile-spike-host".into(),
            resource_compatibility: "yttt-resource-v1".into(),
        },
        lifetime: yttt_host::HostLifetime::Independent,
    }
}

async fn remote_access(admin: &ClientCore, request: RemoteAccessRequest) -> RemoteAccessResponse {
    match admin.request(Request::RemoteAccess(request)).await.unwrap() {
        Response::RemoteAccess(response) => response,
        other => panic!("unexpected response {other:?}"),
    }
}

async fn new_invitation(admin: &ClientCore, code_file: &Path) {
    let RemoteAccessResponse::PairingInvitation { code, .. } =
        remote_access(admin, RemoteAccessRequest::StartPairing).await
    else {
        panic!("expected an invitation");
    };
    fs::write(code_file, &code.0).unwrap();
    println!("\npairing code:\n{}\n", code.0);
}

async fn spawn(admin: &ClientCore, id: &str, program: &str, args: &[&str]) {
    let response = admin
        .request(Request::SpawnTerminal {
            spec: TerminalSpawnSpec {
                session_id: TerminalSessionId::new(id),
                project_id: ProjectId::new("spike"),
                cwd: yttt_protocol::ProjectRelativePath::root(),
                execution: TerminalExecutionSpec::Command {
                    shell: "/bin/zsh".into(),
                    program: program.into(),
                    args: args.iter().map(|arg| arg.to_string()).collect(),
                    return_to_shell: false,
                },
                geometry: TerminalGeometry {
                    cols: 120,
                    rows: 36,
                    cell_width: 8,
                    cell_height: 16,
                },
                geometry_epoch: 1,
                query_palette: Vec::new(),
                palette_revision: 1,
                scrollback_limit: 5_000,
                environment: vec![("TERM".into(), "xterm-256color".into())],
                removed_environment: Vec::new(),
            },
            start_id: format!("spike-{id}"),
            expected_host_epoch: admin.host_epoch().unwrap(),
        })
        .await
        .unwrap();
    assert!(
        matches!(response, Response::TerminalSpawned { .. }),
        "{response:?}"
    );
    println!("terminal {id}: {program} {}", args.join(" "));
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    let state = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/mobile-spike-host");
    let _ = fs::remove_dir_all(&state);
    fs::create_dir_all(&state).unwrap();
    let root = tempfile::tempdir().unwrap();
    let bootstrap = bootstrap(root.path());
    let (listener, connector) = memory_pair();
    let ready = bootstrap.ready_file();
    let build = bootstrap.build.clone();
    tokio::spawn(yttt_host::run_with_remote_work(bootstrap, || async move {
        Ok::<_, yttt_transport::TransportError>((listener, None))
    }));
    while !ready.exists() {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let admin = ClientCore::connect(
        connector,
        ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build,
            profile_id: ProfileId::new(PROFILE),
            client_instance_id: ClientInstanceId::new("spike-desktop"),
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

    let project = root.path().join("project");
    fs::create_dir_all(&project).unwrap();
    admin
        .request(Request::Project(ProjectRequest::Register {
            view_id: "spike".into(),
            project_id: ProjectId::new("spike"),
            root: yttt_protocol::HostPath::from_path(&project).unwrap(),
        }))
        .await
        .unwrap();
    spawn(&admin, "spike:shell:1", "/bin/zsh", &["-f", "-i"]).await;
    spawn(&admin, "spike:demo:1", "/bin/sh", &["-c", DEMO]).await;

    let code_file = state.join("pairing-code");
    new_invitation(&admin, &code_file).await;
    println!("Confirms every device that pairs. Ctrl-C to stop.");
    let mut stop = std::pin::pin!(tokio::signal::ctrl_c());
    loop {
        tokio::select! {
            _ = &mut stop => break,
            _ = tokio::time::sleep(Duration::from_millis(200)) => {}
        }
        let RemoteAccessResponse::Status(status) =
            remote_access(&admin, RemoteAccessRequest::Status).await
        else {
            continue;
        };
        match status.p2p.pairing {
            Some(PairingState::AwaitingConfirmation {
                endpoint_id,
                short_code,
                ..
            }) => {
                println!("confirming device {endpoint_id:?} (code {short_code:06})");
                remote_access(
                    &admin,
                    RemoteAccessRequest::ConfirmPairing {
                        endpoint_id,
                        preset: DevicePreset::Full,
                    },
                )
                .await;
            }
            None => new_invitation(&admin, &code_file).await,
            _ => {}
        }
    }
    admin.shutdown().await;
    let _ = fs::remove_dir_all(&state);
}
