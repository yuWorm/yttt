use yttt::config::default_layout::BuiltinAgent;
use yttt::model::layout::PaneKind;
use yttt::runtime::agent::{
    AgentProcessRecord, classify_agent, classify_agent_process, detect_agent_processes_by_root,
};
use yttt::runtime::notification::{
    AgentTransitionNotificationInput, ExitNotificationInput, NoopSystemNotifier, NotificationEvent,
    NotificationKind, SystemNotifier, maybe_notify_system, notification_for_agent_transition,
    notification_for_exit,
};
use yttt::runtime::terminal::{
    ExitReason, FakeTerminalRuntime, ProcessStatus, TerminalRuntime, TerminalSpawnRequest,
};
use yttt_agent_core::AgentViewState;

fn process_record(
    pid: u32,
    parent_pid: Option<u32>,
    agent: Option<BuiltinAgent>,
    blocks_descendant_agent_discovery: bool,
) -> AgentProcessRecord {
    AgentProcessRecord {
        pid,
        parent_pid,
        agent,
        blocks_descendant_agent_discovery,
    }
}

#[test]
fn fake_runtime_marks_process_running_then_exited() {
    let mut runtime = FakeTerminalRuntime::default();

    let pane = runtime
        .spawn(TerminalSpawnRequest::for_shell("server", "sh", "echo ok"))
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
    let request = TerminalSpawnRequest::for_shell("server", "sh", "pwd").cwd("/tmp/yttt");

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
fn all_onboarding_agent_commands_are_agents() {
    for command in ["codex", "claude", "opencode", "pi", "omp"] {
        assert!(
            classify_agent(None, command).is_agent(),
            "{command} should be classified as an agent"
        );
    }
    assert!(classify_agent(None, "claude --dangerously-skip-permissions").is_agent());
    assert!(classify_agent(None, "/usr/local/bin/codex").is_agent());
    assert!(classify_agent(None, r"C:\tools\opencode.exe").is_agent());
}

#[test]
fn interpreter_backed_agent_processes_are_identified_by_script_path() {
    let cases = [
        (
            "node",
            "/opt/lib/node_modules/@openai/codex/bin/codex.js",
            BuiltinAgent::Codex,
        ),
        (
            "node",
            "/opt/lib/node_modules/@anthropic-ai/claude-code/cli.js",
            BuiltinAgent::Claude,
        ),
        (
            "bun",
            "/opt/lib/node_modules/opencode-ai/bin/opencode",
            BuiltinAgent::OpenCode,
        ),
        (
            "node",
            "/opt/lib/node_modules/@mariozechner/pi-coding-agent/dist/cli.js",
            BuiltinAgent::Pi,
        ),
        (
            "bun",
            "/opt/lib/node_modules/@oh-my-pi/pi-coding-agent/src/cli.ts",
            BuiltinAgent::OhMyPi,
        ),
    ];
    for (interpreter, script, expected) in cases {
        assert_eq!(
            classify_agent_process(interpreter, &[interpreter, script]),
            Some(expected)
        );
    }
    assert_eq!(
        classify_agent_process("node", &["node", "/tmp/runner.js", "ask codex for help"]),
        None
    );
}

#[test]
fn shell_pane_process_tree_detects_the_nearest_agent_descendant() {
    let processes = [
        process_record(10, None, None, false),
        process_record(11, Some(10), Some(BuiltinAgent::Codex), false),
        process_record(12, Some(11), Some(BuiltinAgent::Claude), false),
        process_record(20, None, None, false),
    ];

    let detected = detect_agent_processes_by_root(&[10, 20], &processes);

    assert_eq!(detected.get(&10), Some(&BuiltinAgent::Codex));
    assert!(!detected.contains_key(&20));
}

#[test]
fn nested_yttt_process_blocks_its_agent_descendants_from_the_outer_pane() {
    let processes = [
        process_record(30, None, None, false),
        process_record(31, Some(30), None, false),
        process_record(32, Some(31), None, true),
        process_record(33, Some(32), None, false),
        process_record(34, Some(33), Some(BuiltinAgent::OhMyPi), false),
    ];

    let detected = detect_agent_processes_by_root(&[30], &processes);

    assert!(!detected.contains_key(&30));
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
#[test]
fn agent_state_transitions_emit_attention_and_completion_notifications_once() {
    let waiting =
        notification_for_agent_transition(agent_transition_input(None, AgentViewState::Waiting))
            .unwrap();
    assert_eq!(waiting.kind, NotificationKind::AgentWaiting);
    assert_eq!(waiting.title(), "Codex needs attention");
    assert_eq!(waiting.context(), "yttt › Agent › Codex");

    assert!(
        notification_for_agent_transition(agent_transition_input(
            Some(AgentViewState::Waiting),
            AgentViewState::Waiting,
        ))
        .is_none()
    );
    assert!(
        notification_for_agent_transition(agent_transition_input(
            Some(AgentViewState::Waiting),
            AgentViewState::Working,
        ))
        .is_none()
    );

    let completed = notification_for_agent_transition(agent_transition_input(
        Some(AgentViewState::Working),
        AgentViewState::Completed,
    ))
    .unwrap();
    assert_eq!(completed.kind, NotificationKind::AgentCompleted);
    assert_eq!(
        completed.summary.as_deref(),
        Some("Refine agent notifications")
    );

    let failed = notification_for_agent_transition(agent_transition_input(
        Some(AgentViewState::Working),
        AgentViewState::Failed,
    ))
    .unwrap();
    assert_eq!(failed.kind, NotificationKind::AgentFailed);
    assert!(
        notification_for_agent_transition(agent_transition_input(
            Some(AgentViewState::Failed),
            AgentViewState::Completed,
        ))
        .is_none()
    );
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
        summary: None,
    }
}

fn agent_transition_input(
    previous_state: Option<AgentViewState>,
    state: AgentViewState,
) -> AgentTransitionNotificationInput {
    AgentTransitionNotificationInput {
        previous_state,
        state,
        project_id: "/tmp/yttt".to_string(),
        tab_id: "agent".to_string(),
        pane_id: "codex".to_string(),
        project_title: "yttt".to_string(),
        tab_title: "Agent".to_string(),
        pane_title: "Codex".to_string(),
        summary: Some("Refine agent notifications".to_string()),
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
