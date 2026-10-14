#![forbid(unsafe_code)]

mod board;
mod event;
mod model;
mod provider;
mod reducer;

pub use board::{
    AgentAnswerSource, AgentBoardCommand, AgentBoardEvent, AgentBoardMessage, AgentBoardSnapshot,
    AgentBoardTask, AgentExecutionRecord, AgentExecutionTelemetry, AgentMessageRole,
    AgentPlanDelta, AgentPlanItem, AgentPlanStatus, AgentQuestion, AgentQuestionAnswer,
    AgentQuestionOption, AgentQuestionRecord, AgentQuestionRequest, AgentQuestionState,
};
pub use event::AgentEventKind;
pub use model::{
    AgentAction, AgentExitReason, AgentIdError, AgentInstanceId, AgentProcessExit,
    AgentProcessState, AgentSessionMetadata, AgentTask, AgentTaskSource, AgentTurnState,
    AgentViewState, ChildAgentDescriptor, ChildAgentSnapshot, ChildAgentUpdate, ProviderId,
    ProviderIdError, TurnOutcome, WaitingReason,
};
pub use provider::{
    AgentProvider, ProviderDescriptor, ProviderError, ProviderHookEvent, ProviderResumeCommand,
};
pub use reducer::{AGENT_ACTIVITY_STALE_AFTER_MILLIS, AgentReducer, AgentSnapshot};

#[cfg(test)]
mod tests {
    use super::*;

    fn reducer() -> AgentReducer {
        AgentReducer::new(
            AgentInstanceId::new("agent-1").unwrap(),
            ProviderId::new("omp").unwrap(),
            1,
        )
    }

    #[test]
    fn process_and_session_metadata_do_not_claim_an_idle_turn() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Stale);
        reducer.apply(
            1,
            AgentEventKind::SessionStarted {
                metadata: AgentSessionMetadata {
                    session_id: Some("root".into()),
                    ..Default::default()
                },
            },
            4,
        );
        assert_eq!(reducer.snapshot().turn_state, AgentTurnState::Unknown);
        reducer.apply(1, AgentEventKind::Working, 5);
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Working);
        reducer.apply(
            1,
            AgentEventKind::SessionStarted {
                metadata: AgentSessionMetadata {
                    session_id: Some("resumed".into()),
                    ..Default::default()
                },
            },
            6,
        );
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Stale);
    }

    #[test]
    fn reducer_tracks_turn_action_wait_and_completion() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        assert!(reducer.process_started(1, 3));
        assert!(reducer.apply(
            1,
            AgentEventKind::TurnStarted {
                task: AgentTask::new("Implement agent status", AgentTaskSource::UserPromptHook),
            },
            4,
        ));
        assert!(
            reducer.apply(
                1,
                AgentEventKind::ActionStarted {
                    action: AgentAction::new(
                        Some("tool-1".to_string()),
                        "edit",
                        Some("src/main.rs".to_string()),
                    )
                    .unwrap(),
                },
                5,
            )
        );
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Working);
        assert_eq!(
            reducer.snapshot().secondary_text().as_deref(),
            Some("edit: src/main.rs")
        );

        assert!(reducer.apply(
            1,
            AgentEventKind::Waiting {
                reason: WaitingReason::Approval,
                message: Some("Approve edit".to_string()),
            },
            6,
        ));
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Waiting);

        assert!(reducer.apply(1, AgentEventKind::Working, 7));
        assert!(reducer.apply(
            1,
            AgentEventKind::TurnFinished {
                outcome: TurnOutcome::Completed,
            },
            8,
        ));
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Completed);
        assert_eq!(reducer.snapshot().primary_text(), "Implement agent status");
    }

    #[test]
    fn reducer_rejects_stale_generations() {
        let mut reducer = reducer();
        reducer.process_starting(2, 2);
        assert!(!reducer.process_started(1, 3));
        assert!(!reducer.apply(1, AgentEventKind::Working, 4));
        assert_eq!(
            reducer.snapshot().process_state,
            AgentProcessState::Starting
        );
    }

    #[test]
    fn process_exit_is_authoritative_fallback() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(1, AgentEventKind::Working, 4);
        assert!(reducer.process_exited(
            1,
            AgentProcessExit {
                code: None,
                reason: AgentExitReason::KilledByUser,
            },
            5,
        ));
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Interrupted);
    }

    #[test]
    fn session_end_marks_the_agent_process_exited() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(1, AgentEventKind::Working, 4);

        assert!(reducer.apply(1, AgentEventKind::SessionEnded, 5));
        assert_eq!(reducer.snapshot().process_state, AgentProcessState::Exited);
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Completed);
    }
    #[test]
    fn reducer_tracks_child_agent_lifecycle() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: "child-1".to_string(),
                    name: Some("Reviewer".to_string()),
                    task: AgentTask::new("Review event mapping", AgentTaskSource::External),
                },
            },
            4,
        ));
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildUpdated {
                child_id: "child-1".to_string(),
                update: ChildAgentUpdate {
                    task: None,
                    current_action: AgentAction::new(None, "Read", Some("event.rs".to_string())),
                    clear_current_action: false,
                    turn_state: Some(AgentTurnState::Working),
                },
            },
            5,
        ));
        let child = &reducer.snapshot().children[0];
        assert_eq!(child.name.as_deref(), Some("Reviewer"));
        assert_eq!(child.primary_text(), "Review event mapping");
        assert_eq!(child.turn_state, AgentTurnState::Working);
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildUpdated {
                child_id: "child-1".to_string(),
                update: ChildAgentUpdate {
                    clear_current_action: true,
                    ..Default::default()
                },
            },
            6,
        ));
        assert!(reducer.snapshot().children[0].current_action.is_none());
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildFinished {
                child_id: "child-1".to_string(),
                outcome: TurnOutcome::Completed,
            },
            6,
        ));
        assert!(reducer.snapshot().children.is_empty());

        assert!(reducer.apply(
            1,
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: "child-2".to_string(),
                    name: None,
                    task: None,
                },
            },
            7,
        ));
        assert!(reducer.apply(
            1,
            AgentEventKind::TurnFinished {
                outcome: TurnOutcome::Completed,
            },
            8,
        ));
        assert_eq!(reducer.snapshot().children.len(), 1);
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Working);

        assert!(reducer.apply(
            1,
            AgentEventKind::ChildFinished {
                child_id: "child-2".to_string(),
                outcome: TurnOutcome::Completed,
            },
            9,
        ));
        assert!(reducer.snapshot().children.is_empty());
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Completed);
    }

    #[test]
    fn child_board_activity_requires_a_known_child_and_cannot_change_root_action() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        let root_action =
            AgentAction::new(Some("root-tool".into()), "Edit", Some("src/lib.rs".into())).unwrap();
        assert!(reducer.apply(
            1,
            AgentEventKind::ActionStarted {
                action: root_action.clone(),
            },
            4,
        ));
        let child_execution = AgentExecutionRecord {
            id: "root:child:child-1".into(),
            parent_id: Some("root".into()),
            name: "Explore".into(),
            task: None,
            telemetry: AgentExecutionTelemetry::default(),
            current_action: Some("Read src/board.rs".into()),
            state: AgentTurnState::Working,
            started_at: 4,
            updated_at: 4,
            ended_at: None,
        };
        assert!(!reducer.apply(
            1,
            AgentEventKind::ChildBoard {
                child_id: "child-1".into(),
                event: AgentBoardEvent::Execution {
                    execution: child_execution.clone(),
                },
            },
            5,
        ));
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: "child-1".into(),
                    name: Some("Explore".into()),
                    task: None,
                },
            },
            6,
        ));
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildBoard {
                child_id: "child-1".into(),
                event: AgentBoardEvent::Execution {
                    execution: child_execution,
                },
            },
            7,
        ));
        assert_eq!(reducer.snapshot().current_action, Some(root_action));
        assert_eq!(reducer.snapshot().waiting_reason, None);
        assert_eq!(
            reducer.snapshot().board.executions[0]
                .current_action
                .as_deref(),
            Some("Read src/board.rs")
        );
    }

    #[test]
    fn retained_child_telemetry_arrives_after_stop_without_reviving_the_execution() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: "child-1".into(),
                    name: Some("Explore".into()),
                    task: None,
                },
            },
            4,
        ));
        let execution = AgentExecutionRecord {
            id: "claude:session:prompt:child:child-1".into(),
            parent_id: Some("claude:session:prompt".into()),
            name: "Explore".into(),
            task: None,
            telemetry: AgentExecutionTelemetry::default(),
            current_action: None,
            state: AgentTurnState::Working,
            started_at: 4,
            updated_at: 4,
            ended_at: None,
        };
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildBoard {
                child_id: "child-1".into(),
                event: AgentBoardEvent::Execution {
                    execution: execution.clone(),
                },
            },
            5,
        ));
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildBoard {
                child_id: "child-1".into(),
                event: AgentBoardEvent::Execution {
                    execution: AgentExecutionRecord {
                        state: AgentTurnState::Completed,
                        ended_at: Some(6),
                        ..execution
                    },
                },
            },
            6,
        ));
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildFinished {
                child_id: "child-1".into(),
                outcome: TurnOutcome::Completed,
            },
            7,
        ));
        assert!(reducer.apply(
            1,
            AgentEventKind::Board {
                event: AgentBoardEvent::ExecutionTelemetry {
                    execution_id: "claude:session:prompt:child:child-1".into(),
                    telemetry: AgentExecutionTelemetry {
                        tokens: Some(17),
                        input_tokens: Some(11),
                        ..Default::default()
                    },
                },
            },
            8,
        ));
        let retained = &reducer.snapshot().board.executions[0];
        assert_eq!(retained.name, "Explore");
        assert_eq!(retained.state, AgentTurnState::Completed);
        assert_eq!(retained.ended_at, Some(6));
        assert_eq!(retained.telemetry.tokens, Some(17));
        assert_eq!(retained.telemetry.input_tokens, Some(11));
    }

    #[test]
    fn stale_activity_becomes_unknown_without_losing_history() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(
            1,
            AgentEventKind::TurnStarted {
                task: AgentTask::new("Long-running task", AgentTaskSource::UserPromptHook),
            },
            4,
        );
        reducer.apply(
            1,
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: "child-stale".to_string(),
                    name: Some("Reviewer".to_string()),
                    task: None,
                },
            },
            5,
        );

        let mut snapshot = reducer.snapshot().clone();
        assert!(!snapshot.decay_stale_activity(105, 100));
        assert!(snapshot.decay_stale_activity(106, 100));
        assert_eq!(snapshot.view_state(), AgentViewState::Stale);
        assert_eq!(snapshot.children[0].turn_state, AgentTurnState::Unknown);
        assert_eq!(snapshot.primary_text(), "Long-running task");
        assert!(!snapshot.decay_stale_activity(107, 100));
    }

    #[test]
    fn live_activity_preserves_actions_and_approvals_but_idle_can_expire() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(
            1,
            AgentEventKind::ActionStarted {
                action: AgentAction {
                    id: Some("long-tool".into()),
                    name: "bash".into(),
                    detail: None,
                },
            },
            4,
        );
        reducer.apply(1, AgentEventKind::Activity { is_idle: false }, 100);
        assert!(!reducer.decay_stale_activity(104, 100));
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Working);
        assert_eq!(
            reducer
                .snapshot()
                .current_action
                .as_ref()
                .unwrap()
                .id
                .as_deref(),
            Some("long-tool")
        );
        reducer.apply(
            1,
            AgentEventKind::Waiting {
                reason: WaitingReason::Approval,
                message: Some("Approve deployment".into()),
            },
            105,
        );
        reducer.apply(1, AgentEventKind::Activity { is_idle: false }, 200);
        assert!(!reducer.decay_stale_activity(205, 100));
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Waiting);
        assert_eq!(
            reducer.snapshot().waiting_message.as_deref(),
            Some("Approve deployment")
        );
        reducer.apply(1, AgentEventKind::Activity { is_idle: true }, 206);
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Idle);
        assert!(reducer.snapshot().current_action.is_none());
        assert!(!reducer.decay_stale_activity(306, 100));
        assert!(reducer.decay_stale_activity(307, 100));
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Stale);
    }

    #[test]
    fn root_activity_does_not_keep_a_lost_child_alive() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(1, AgentEventKind::Activity { is_idle: true }, 4);
        reducer.apply(
            1,
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: "lost".into(),
                    name: None,
                    task: None,
                },
            },
            5,
        );
        reducer.apply(1, AgentEventKind::Activity { is_idle: true }, 100);
        assert!(!reducer.decay_stale_activity(105, 100));
        assert!(reducer.decay_stale_activity(106, 100));
        assert_eq!(
            reducer.snapshot().children[0].turn_state,
            AgentTurnState::Unknown
        );
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Stale);
        reducer.apply(
            1,
            AgentEventKind::ChildFinished {
                child_id: "lost".into(),
                outcome: TurnOutcome::Completed,
            },
            107,
        );
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Idle);
    }

    #[test]
    fn stale_lead_is_not_revived_by_a_child_finishing() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(1, AgentEventKind::Working, 4);
        reducer.apply(
            1,
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: "child".to_string(),
                    name: None,
                    task: None,
                },
            },
            5,
        );
        assert!(reducer.decay_stale_activity(106, 100));
        reducer.apply(
            1,
            AgentEventKind::ChildFinished {
                child_id: "child".to_string(),
                outcome: TurnOutcome::Completed,
            },
            107,
        );
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Stale);
        reducer.apply(1, AgentEventKind::Working, 108);
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Working);
    }

    #[test]
    fn stale_approval_is_unknown_until_an_authoritative_event_arrives() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(
            1,
            AgentEventKind::Waiting {
                reason: WaitingReason::Approval,
                message: Some("Approve deployment".to_string()),
            },
            4,
        );
        assert!(reducer.decay_stale_activity(105, 100));
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Stale);
        reducer.apply(
            1,
            AgentEventKind::TurnFinished {
                outcome: TurnOutcome::Completed,
            },
            106,
        );
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Completed);
        assert!(!reducer.decay_stale_activity(207, 100));
    }

    #[test]
    fn stale_child_keeps_finished_lead_unknown_until_child_finishes() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(
            1,
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: "child".to_string(),
                    name: None,
                    task: None,
                },
            },
            4,
        );
        reducer.apply(
            1,
            AgentEventKind::TurnFinished {
                outcome: TurnOutcome::Completed,
            },
            5,
        );
        assert!(reducer.decay_stale_activity(106, 100));
        reducer.apply(
            1,
            AgentEventKind::ChildUpdated {
                child_id: "child".to_string(),
                update: ChildAgentUpdate {
                    task: AgentTask::new("Still awaiting result", AgentTaskSource::External),
                    current_action: None,
                    clear_current_action: false,
                    turn_state: None,
                },
            },
            107,
        );
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Stale);
        reducer.apply(
            1,
            AgentEventKind::ChildFinished {
                child_id: "child".to_string(),
                outcome: TurnOutcome::Completed,
            },
            108,
        );
        assert_eq!(reducer.snapshot().view_state(), AgentViewState::Completed);
    }

    #[test]
    fn reducer_generates_updates_and_restores_session_titles() {
        let mut reducer = reducer();
        reducer.process_starting(1, 2);
        reducer.process_started(1, 3);
        reducer.apply(
            1,
            AgentEventKind::SessionStarted {
                metadata: AgentSessionMetadata {
                    session_id: Some("session-1".to_string()),
                    model: Some("model-1".to_string()),
                    ..Default::default()
                },
            },
            4,
        );
        reducer.apply(
            1,
            AgentEventKind::TurnStarted {
                task: AgentTask::new(
                    "  Refactor\n the authentication middleware  ",
                    AgentTaskSource::UserPromptHook,
                ),
            },
            5,
        );
        assert_eq!(
            reducer
                .snapshot()
                .session
                .as_ref()
                .and_then(|session| session.title.as_deref()),
            Some("Refactor the authentication middleware")
        );
        reducer.apply(
            1,
            AgentEventKind::SessionUpdated {
                metadata: AgentSessionMetadata {
                    title: Some("Authentication cleanup".to_string()),
                    ..Default::default()
                },
            },
            6,
        );
        assert_eq!(reducer.snapshot().primary_text(), "Authentication cleanup");

        let restored = reducer.snapshot().clone();
        let mut resumed = AgentReducer::from_restored(
            AgentInstanceId::new("agent-2").unwrap(),
            ProviderId::new("omp").unwrap(),
            &restored,
            7,
        );
        resumed.process_starting(1, 7);
        assert_eq!(resumed.snapshot().primary_text(), "Authentication cleanup");
        assert_eq!(
            resumed
                .snapshot()
                .session
                .as_ref()
                .and_then(|session| session.session_id.as_deref()),
            Some("session-1")
        );

        resumed.apply(
            1,
            AgentEventKind::SessionStarted {
                metadata: AgentSessionMetadata {
                    session_id: Some("session-2".to_string()),
                    ..Default::default()
                },
            },
            8,
        );
        assert!(resumed.snapshot().task.is_none());
        assert_eq!(resumed.snapshot().primary_text(), "omp");
    }
}
