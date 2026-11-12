//! Spike: a throwaway Host for trying the mobile link end to end without the
//! desktop app or a real profile. It runs `yttt-host` on a temporary profile,
//! turns P2P on (without a relay unless given one), prints pairing codes and
//! confirms every device that asks, and opens a project "Spike" in the active
//! workspace with a clean zsh (`spike:shell:1`), a demo that prints colors and
//! CJK text (`spike:demo:1`), and two fake Agents that keep cycling through
//! working, waiting for approval and finishing, so the home screen has
//! something to show (`spike:review:codex`, `spike:build:codex`). The fakes
//! post Codex hook events with `curl`.
//!
//! ```text
//! cargo run -p yttt-mobile-core --example spike_host [-- --relay http://<LAN IP>:3341] [--name A]
//! ```
//!
//! The current pairing code is also written to
//! `target/mobile-spike-host/pairing-code`. Everything is deleted on exit.
//!
//! `--name A` runs one of several spike Hosts side by side: its pairing code
//! goes to `target/mobile-spike-host-A/` and its project is "Spike A".

use std::{fs, path::Path, time::Duration};

use yttt_client_core::ClientCore;
use yttt_core::model::{
    host_workspace::{ProjectPanel, WorkspaceIntent},
    ids::{ClientInstanceId, ProfileId, ProjectId},
    layout::ProjectLayout,
    project::{ProjectDescriptor, ProjectLocation},
};
use yttt_protocol::{
    BuildIdentity, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    p2p::DevicePreset,
    remote_access::{PairingState, RemoteAccessRequest, RemoteAccessResponse},
    workspace::{WorkspaceId, WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse},
};
use yttt_transport::{AuthToken, ClientIdentity, memory_pair};

const TOKEN: [u8; 32] = [0x7d; 32];
const PROFILE: &str = "mobile-spike-host";

const DEMO: &str = r#"printf '\033[1;32mspike host\033[0m  \033[33m黄色\033[0m \033[38;5;208m208\033[0m \033[38;2;80;160;255mrgb\033[0m\n'
printf '中文宽字符：终端镜像 ✓  emoji 😀\n'
i=0; while true; do i=$((i+1)); printf '\r\033[2Ktick %d  %s' "$i" "$(date +%T)"; sleep 1; done"#;

/// A fake Agent: `sh agent.sh <session> <task> <seconds per step>` posts the
/// hook events a Codex session would, over and over.
const AGENT: &str = r#"post() {
  curl -s -o /dev/null -X POST "$YTTT_AGENT_HOOK_ENDPOINT/hook/codex" \
    -H "x-yttt-agent-hook-token: $YTTT_AGENT_HOOK_TOKEN" \
    -H "x-yttt-agent-hook-scope: $YTTT_AGENT_HOOK_SCOPE" \
    --data "{\"event\":\"$1\",\"payload\":$2}"
}
session="$1"; task="$2"; step="$3"
post SessionStart "{\"session_id\":\"$session\"}"
while true; do
  echo "> $task"
  post UserPromptSubmit "{\"session_id\":\"$session\",\"prompt\":\"$task\"}"
  sleep "$step"
  echo "running npm test"
  post PreToolUse "{\"session_id\":\"$session\",\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"npm test\"}}"
  sleep "$step"
  post PostToolUse "{\"session_id\":\"$session\",\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"npm test\"}}"
  echo "waiting for approval: npm install"
  post PermissionRequest "{\"session_id\":\"$session\",\"tool_name\":\"Bash\",\"tool_input\":{\"command\":\"npm install\"}}"
  sleep $((step * 2))
  echo "done"
  post Stop "{\"session_id\":\"$session\"}"
  sleep "$step"
done
"#;

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

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() {
    // `--relay <url>`: the Host uses this relay and names it in pairing codes.
    // Give an address both the Host and the phone reach, such as the Mac's
    // LAN address for an emulator.
    let args = std::env::args().collect::<Vec<_>>();
    let relay_url = args
        .iter()
        .position(|arg| arg == "--relay")
        .and_then(|at| args.get(at + 1).cloned());
    if let Some(url) = &relay_url {
        println!("relay: {url}");
    }
    let name = args
        .iter()
        .position(|arg| arg == "--name")
        .and_then(|at| args.get(at + 1).cloned());
    let (state_dir, project_name) = match &name {
        Some(name) => (format!("mobile-spike-host-{name}"), format!("Spike {name}")),
        None => ("mobile-spike-host".to_owned(), "Spike".to_owned()),
    };
    let state = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(state_dir);
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
            relay_url: relay_url.clone(),
        },
    )
    .await;

    let project = root.path().join("project");
    fs::create_dir_all(&project).unwrap();
    fs::write(project.join("demo.sh"), DEMO).unwrap();
    fs::write(project.join("agent.sh"), AGENT).unwrap();
    // Something to browse and edit in the right drawer.
    fs::write(
        project.join("README.md"),
        "# Spike\n\nA throwaway project the mobile apps open.\n",
    )
    .unwrap();
    fs::create_dir_all(project.join("src")).unwrap();
    fs::write(
        project.join("src/app.js"),
        "export function greet(name) {\n  return `hello ${name}`;\n}\n",
    )
    .unwrap();
    println!("project files: {}", project.display());
    // The Host starts each eager tab's pane, as for a project the desktop opens.
    let layout: ProjectLayout = toml::from_str(
        &r#"
        [project]
        name = "{project}"

        [[tabs]]
        id = "shell"
        title = "Shell"
        startup = "eager"
        layout = { type = "pane", id = "1", title = "zsh", command = "zsh -f -i" }

        [[tabs]]
        id = "demo"
        title = "Demo"
        startup = "eager"
        layout = { type = "pane", id = "1", title = "Demo", command = "sh demo.sh" }

        [[tabs]]
        id = "server"
        title = "Server"
        startup = "eager"

        [tabs.layout]
        type = "split"
        direction = "horizontal"
        ratio = 0.5
        left = { type = "pane", id = "app", title = "App", command = "sh demo.sh" }
        right = { type = "pane", id = "logs", title = "Logs", command = "zsh -f -i" }

        [[tabs]]
        id = "review"
        title = "Code review"
        startup = "eager"
        layout = { type = "pane", id = "codex", title = "Codex", command = "sh agent.sh review 'Review the login page' 5" }

        [[tabs]]
        id = "build"
        title = "Build"
        startup = "eager"
        layout = { type = "pane", id = "codex", title = "Codex", command = "sh agent.sh build 'Fix the failing build' 8" }
        "#
        .replace("{project}", &project_name),
    )
    .unwrap();
    let workspace_id = WorkspaceId::new("spike").unwrap();
    let response = admin
        .request(Request::Workspace(WorkspaceRequest::Apply {
            workspace_id: workspace_id.clone(),
            intent: WorkspaceIntent::OpenProject {
                descriptor: ProjectDescriptor::new(
                    ProjectId::new("spike"),
                    ProjectLocation::local(&project),
                ),
                layout,
                panel: ProjectPanel {
                    visible: true,
                    width: 240.0,
                },
            },
            expected_revision: None,
        }))
        .await
        .unwrap();
    assert!(
        matches!(
            response,
            Response::Workspace(WorkspaceResponse::Applied { .. })
        ),
        "{response:?}"
    );
    // As the desktop window showing it would, which the phone opens.
    admin
        .request(Request::Workspace(WorkspaceRequest::SetActive {
            workspace_id,
        }))
        .await
        .unwrap();
    println!(
        "project Spike: spike:shell:1, spike:demo:1, spike:server:app + spike:server:logs (split), \
         spike:review:codex, spike:build:codex"
    );

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
                // A new invitation closes this one; give the device time to
                // hear that it was confirmed, which over a distant relay can
                // take a while.
                tokio::time::sleep(Duration::from_secs(5)).await;
            }
            None => new_invitation(&admin, &code_file).await,
            _ => {}
        }
    }
    admin.shutdown().await;
    let _ = fs::remove_dir_all(&state);
}
