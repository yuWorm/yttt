use yttt::model::layout::PaneKind;
use yttt::runtime::agent::classify_agent;
use yttt::runtime::notification::{
    notification_for_exit, ExitNotificationInput, NotificationKind,
};
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

#[test]
fn agent_exit_code_zero_emits_completed_notification() {
    let event = notification_for_exit(exit_input(true, Some(0), ExitReason::Completed)).unwrap();

    assert_eq!(event.kind, NotificationKind::AgentCompleted);
    assert_eq!(event.project_title, "yttt");
    assert_eq!(event.tab_title, "Agent");
    assert_eq!(event.pane_title, "Codex");
}

#[test]
fn agent_non_zero_exit_emits_failed_notification() {
    let event = notification_for_exit(exit_input(true, Some(1), ExitReason::Failed)).unwrap();

    assert_eq!(event.kind, NotificationKind::AgentFailed);
}

#[test]
fn user_killed_agent_exit_emits_no_notification() {
    let event = notification_for_exit(exit_input(true, None, ExitReason::KilledByUser));

    assert!(event.is_none());
}

#[test]
fn normal_shell_exit_emits_no_agent_notification() {
    let event = notification_for_exit(exit_input(false, Some(0), ExitReason::Completed));

    assert!(event.is_none());
}

fn exit_input(
    is_agent: bool,
    exit_code: Option<i32>,
    exit_reason: ExitReason,
) -> ExitNotificationInput {
    ExitNotificationInput {
        is_agent,
        notify_on_exit: true,
        exit_code,
        exit_reason,
        project_title: "yttt".to_string(),
        tab_title: "Agent".to_string(),
        pane_title: "Codex".to_string(),
    }
}
