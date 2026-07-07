use yttt::model::layout::PaneKind;
use yttt::runtime::agent::classify_agent;
use yttt::runtime::terminal::{
    ExitReason, FakeTerminalRuntime, ProcessStatus, TerminalRuntime,
};

#[test]
fn fake_runtime_marks_process_running_then_exited() {
    let mut runtime = FakeTerminalRuntime::default();

    let pane = runtime.spawn("server", "echo ok").unwrap();
    assert_eq!(runtime.status(pane), Some(ProcessStatus::Running));

    runtime.exit(pane, 0, ExitReason::Completed);

    assert_eq!(
        runtime.status(pane),
        Some(ProcessStatus::Exited { code: Some(0) })
    );
}

#[test]
fn explicit_agent_kind_is_agent() {
    assert!(classify_agent(Some(PaneKind::Agent), "anything").is_agent());
}

#[test]
fn codex_and_claude_commands_are_agents() {
    assert!(classify_agent(None, "codex").is_agent());
    assert!(classify_agent(None, "claude --dangerously-skip-permissions").is_agent());
    assert!(classify_agent(None, "/usr/local/bin/codex").is_agent());
}

#[test]
fn normal_shell_command_is_not_agent() {
    assert!(!classify_agent(None, "npm run dev").is_agent());
}
