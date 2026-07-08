use yttt::model::layout::PaneKind;
use yttt::runtime::agent::classify_agent;
use yttt::runtime::notification::{
    ExitNotificationInput, NoopSystemNotifier, NotificationEvent, NotificationKind, SystemNotifier,
    maybe_notify_system, notification_for_exit,
};
use yttt::runtime::terminal::{
    ExitReason, FakeTerminalRuntime, ProcessStatus, TerminalRuntime, TerminalSpawnRequest,
};

#[test]
fn fake_runtime_marks_process_running_then_exited() {
    let mut runtime = FakeTerminalRuntime::default();

    let pane = runtime
        .spawn(TerminalSpawnRequest::for_shell("server", "echo ok"))
        .unwrap();
    assert_eq!(runtime.status(pane), Some(ProcessStatus::Running));

    runtime.exit(pane, 0, ExitReason::Completed);

    assert_eq!(
        runtime.status(pane),
        Some(ProcessStatus::Exited { code: Some(0) })
    );
}

#[test]
fn fake_runtime_records_spawn_cwd() {
    let mut runtime = FakeTerminalRuntime::default();
    let request = TerminalSpawnRequest::for_shell("server", "pwd").cwd("/tmp/yttt");

    let pane = runtime.spawn(request).unwrap();

    assert_eq!(
        runtime.spawn_cwd(pane).unwrap().to_string_lossy(),
        "/tmp/yttt"
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
    assert_eq!(event.project_id, "/tmp/yttt");
    assert_eq!(event.tab_id, "agent");
    assert_eq!(event.pane_id, "codex");
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

#[test]
fn noop_system_notifier_accepts_notification_events() {
    let notifier = NoopSystemNotifier;

    notifier.notify(&notification_event()).unwrap();
}

#[test]
fn system_notification_is_sent_only_when_enabled() {
    let notifier = CountingNotifier::default();
    let event = notification_event();

    assert!(!maybe_notify_system(&notifier, false, &event).unwrap());
    assert_eq!(notifier.count.get(), 0);

    assert!(maybe_notify_system(&notifier, true, &event).unwrap());
    assert_eq!(notifier.count.get(), 1);
}

#[derive(Default)]
struct CountingNotifier {
    count: std::cell::Cell<usize>,
}

impl SystemNotifier for CountingNotifier {
    fn notify(&self, _event: &NotificationEvent) -> anyhow::Result<()> {
        self.count.set(self.count.get() + 1);
        Ok(())
    }
}

fn notification_event() -> NotificationEvent {
    NotificationEvent {
        kind: NotificationKind::AgentCompleted,
        project_id: "/tmp/yttt".to_string(),
        tab_id: "agent".to_string(),
        pane_id: "codex".to_string(),
        project_title: "yttt".to_string(),
        tab_title: "Agent".to_string(),
        pane_title: "Codex".to_string(),
    }
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
        project_id: "/tmp/yttt".to_string(),
        tab_id: "agent".to_string(),
        pane_id: "codex".to_string(),
        project_title: "yttt".to_string(),
        tab_title: "Agent".to_string(),
        pane_title: "Codex".to_string(),
    }
}
