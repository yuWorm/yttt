use std::ffi::OsString;
use yttt::{
    cli,
    desktop_shell::{DesktopShellClaim, DesktopShellCommand, DesktopShellRuntime, control_desktop},
};
use yttt_protocol::desktop_control::*;

fn parse(words: &[&str]) -> Result<cli::CliInvocation, String> {
    cli::parse(&words.iter().map(OsString::from).collect::<Vec<_>>())
}

#[test]
fn cli_requires_explicit_targets_and_rejects_misspelled_options() {
    assert!(
        parse(&["panes", "send", "--text", "echo hello"])
            .unwrap_err()
            .contains("--pane")
    );
    assert!(
        parse(&["tabs", "create", "--project", "p", "--comand", "pwd"])
            .unwrap_err()
            .contains("--comand")
    );
    assert!(
        parse(&[
            "tabs",
            "close",
            "--project",
            "p",
            "--tab",
            "t",
            "--pane",
            "p"
        ])
        .is_err()
    );
    assert!(parse(&["panes", "list", "--pane", "shell"]).is_err());
    assert!(
        parse(&[
            "panes",
            "resize",
            "--project",
            "p",
            "--tab",
            "t",
            "--pane",
            "x",
            "--direction",
            "left",
            "--percent",
            "0"
        ])
        .is_err()
    );
}

#[test]
fn cli_preserves_provider_arguments_and_terminal_text_without_shell_expansion() {
    let invocation = parse(&[
        "agents",
        "create",
        "--project",
        "p",
        "--provider",
        "codex",
        "--json",
        "--",
        "Review $HOME and `x`",
        "--remote-client",
    ])
    .unwrap();
    assert!(invocation.json);
    assert_eq!(
        invocation.request.command,
        DesktopControlCommand::CreateAgent {
            provider: "codex".into(),
            args: vec!["Review $HOME and `x`".into(), "--remote-client".into()],
            title: None,
        }
    );
    let invocation = parse(&[
        "agents",
        "send",
        "--project",
        "p",
        "--tab",
        "t",
        "--pane",
        "a",
        "--text",
        "first\n第二行",
    ])
    .unwrap();
    assert_eq!(
        invocation.request.command,
        DesktopControlCommand::Send {
            text: "first\n第二行".into(),
            enter: true,
            raw: false,
            agent_only: true,
        }
    );
}

#[test]
fn cli_bounds_prompt_files_and_rejects_ambiguous_input_sources() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("prompt.txt");
    std::fs::write(&file, "x".repeat(MAX_CONTROL_TEXT_BYTES + 1)).unwrap();
    assert!(
        parse(&[
            "agents",
            "send",
            "--project",
            "p",
            "--tab",
            "t",
            "--pane",
            "a",
            "--prompt-file",
            file.to_str().unwrap()
        ])
        .is_err()
    );
    assert!(parse(&["agents", "send", "--text", "hello", "--stdin"]).is_err());
}

#[test]
fn desktop_control_waits_for_execution_reply_and_preserves_errors() {
    use yttt::config::profile::{
        AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
    };
    let root = tempfile::tempdir().unwrap();
    let profile = AppProfile::scoped(
        yttt::model::ids::ProfileId::random(),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        root.path(),
        ProjectConfigPolicy::Overlay,
        HostConnectPolicy::ProfileDiscovery,
    );
    let owner = match DesktopShellRuntime::claim_or_forward(&profile, DesktopShellCommand::Activate)
        .unwrap()
    {
        DesktopShellClaim::Owner(owner) => owner,
        _ => panic!("expected endpoint owner"),
    };
    let calls = owner.controls();
    let handler = std::thread::spawn(move || {
        let call = calls
            .recv_timeout(std::time::Duration::from_secs(3))
            .unwrap();
        assert_eq!(call.request.command, DesktopControlCommand::Projects);
        call.reply
            .send(Err(DesktopControlError::new(
                DesktopControlErrorCode::PermissionDenied,
                "observer",
            )))
            .unwrap();
    });
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let result = runtime
        .block_on(control_desktop(
            &profile,
            DesktopControlRequest {
                window: None,
                project: None,
                tab: None,
                pane: None,
                command: DesktopControlCommand::Projects,
            },
        ))
        .unwrap();
    assert_eq!(
        result.unwrap_err().code,
        DesktopControlErrorCode::PermissionDenied
    );
    handler.join().unwrap();
}
