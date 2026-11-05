use std::ffi::OsString;
use yttt::{
    cli,
    desktop_shell::{DesktopShellClaim, DesktopShellCommand, DesktopShellRuntime, control_desktop},
    ui::i18n::UiText,
};
use yttt_core::model::layout::PaneRole;
use yttt_protocol::desktop_control::*;

fn parse(words: &[&str]) -> Result<cli::CliInvocation, String> {
    cli::parse(
        &words.iter().map(OsString::from).collect::<Vec<_>>(),
        UiText::english(),
    )
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
            role: PaneRole::Worker,
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
fn cli_grants_the_orchestrator_role_only_when_asked_and_rejects_unknown_roles() {
    let role = |extra: &[&str]| {
        let mut words = vec!["agents", "create", "--project", "p", "--provider", "claude"];
        words.extend_from_slice(extra);
        parse(&words).map(|invocation| match invocation.request.command {
            DesktopControlCommand::CreateAgent { role, .. } => role,
            other => panic!("unexpected command {other:?}"),
        })
    };
    assert_eq!(role(&[]).unwrap(), PaneRole::Worker);
    assert_eq!(
        role(&["--role", "orchestrator"]).unwrap(),
        PaneRole::Orchestrator
    );
    assert!(
        role(&["--role", "lead"])
            .unwrap_err()
            .contains("worker or orchestrator")
    );
    let wire = serde_json::to_value(DesktopControlCommand::CreateAgent {
        provider: "claude".into(),
        args: Vec::new(),
        title: None,
        role: PaneRole::Worker,
    })
    .unwrap();
    assert!(
        wire["create_agent"].get("role").is_none(),
        "requests from older CLIs and newer ones agree on workers: {wire}"
    );
}

fn parse_control(words: &[&str]) -> Result<Option<cli::AgentControlInvocation>, String> {
    cli::parse_agent_control(
        &words.iter().map(OsString::from).collect::<Vec<_>>(),
        UiText::english(),
    )
}

#[test]
fn orchestrator_commands_go_to_the_host_and_others_to_the_desktop() {
    let spawn = parse_control(&[
        "agents",
        "spawn",
        "--provider",
        "codex",
        "--title",
        "Parser",
        "--json",
        "--",
        "Fix $HOME parsing",
    ])
    .unwrap()
    .unwrap();
    assert_eq!(spawn.path, "/control/v1/agents/spawn");
    assert!(spawn.json);
    assert_eq!(
        spawn.body,
        serde_json::json!({"provider": "codex", "args": ["Fix $HOME parsing"], "title": "Parser"})
    );
    let mine = parse_control(&["agents", "list", "--mine"])
        .unwrap()
        .unwrap();
    assert_eq!(mine.path, "/control/v1/agents/list");
    let close = parse_control(&["agents", "close", "--agent", "p:t:agent"])
        .unwrap()
        .unwrap();
    assert_eq!(close.body, serde_json::json!({"agent": "p:t:agent"}));

    assert!(
        parse_control(&["agents", "list", "--project", "p"])
            .unwrap()
            .is_none()
    );
    assert!(parse_control(&["tabs", "close"]).unwrap().is_none());
    assert!(
        parse_control(&["agents", "close"])
            .unwrap_err()
            .contains("--agent")
    );
    assert!(
        parse_control(&["agents", "spawn", "--provider", "codex", "--project", "p"])
            .unwrap_err()
            .contains("--project")
    );
    assert!(parse_control(&["agents", "close", "--agent", "a", "--", "extra"]).is_err());
}

#[test]
fn assigning_an_agent_names_one_orchestrator_or_clears_it() {
    let assign = |extra: &[&str]| {
        let mut words = vec![
            "agents",
            "assign",
            "--project",
            "p",
            "--tab",
            "t",
            "--pane",
            "agent",
        ];
        words.extend_from_slice(extra);
        parse(&words).map(|invocation| invocation.request.command)
    };
    assert_eq!(
        assign(&["--to", "p:lead:lead"]).unwrap(),
        DesktopControlCommand::AssignAgent {
            owner: Some("p:lead:lead".into())
        }
    );
    assert_eq!(
        assign(&["--clear"]).unwrap(),
        DesktopControlCommand::AssignAgent { owner: None }
    );
    assert!(assign(&[]).is_err());
    assert!(assign(&["--to", "x", "--clear"]).is_err());
    assert!(
        parse(&[
            "agents",
            "assign",
            "--project",
            "p",
            "--tab",
            "t",
            "--to",
            "x"
        ])
        .unwrap_err()
        .contains("--pane")
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

#[test]
fn offline_cli_language_prefers_device_and_never_migrates_legacy_settings() {
    use yttt::config::{
        profile::{
            AppProfile, EnvironmentKind, HostConnectPolicy, ProfilePersistence, ProjectConfigPolicy,
        },
        settings::LanguageSetting,
    };
    use yttt::ui::i18n::Locale;

    let root = tempfile::tempdir().unwrap();
    let profile = AppProfile::scoped(
        yttt::model::ids::ProfileId::random(),
        EnvironmentKind::Test,
        ProfilePersistence::Ephemeral,
        root.path(),
        ProjectConfigPolicy::ReadOnly,
        HostConnectPolicy::ProfileDiscovery,
    );
    let config = &profile.paths().config;
    let device = config.join("device/settings.toml");
    let system = UiText::from_language(LanguageSetting::System);
    let (legacy_language, legacy_locale) = match system.locale() {
        Locale::English => ("zh-CN", Locale::Chinese),
        Locale::Chinese => ("en", Locale::English),
    };
    std::fs::create_dir_all(config).unwrap();
    std::fs::write(
        config.join("settings.toml"),
        format!("[general]\nlanguage = \"{legacy_language}\"\n"),
    )
    .unwrap();
    assert_eq!(cli::ui_text_for_profile(&profile).locale(), legacy_locale);
    assert!(
        !device.exists(),
        "reading CLI language must not migrate legacy settings"
    );

    std::fs::create_dir_all(device.parent().unwrap()).unwrap();
    for (language, expected) in [
        ("en", Locale::English),
        ("zh-CN", Locale::Chinese),
        ("system", system.locale()),
    ] {
        std::fs::write(&device, format!("[general]\nlanguage = \"{language}\"\n")).unwrap();
        assert_eq!(cli::ui_text_for_profile(&profile).locale(), expected);
    }

    std::fs::write(&device, "[invalid").unwrap();
    assert_eq!(
        cli::ui_text_for_profile(&profile),
        system,
        "an unreadable Device file must not silently reuse a stale legacy preference"
    );
    assert_eq!(std::fs::read_to_string(&device).unwrap(), "[invalid");
    assert!(
        !profile.paths().data.exists(),
        "offline language lookup must not start a Host"
    );
}
