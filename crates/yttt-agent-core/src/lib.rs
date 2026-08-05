#![forbid(unsafe_code)]

mod event;
mod model;
mod provider;
mod reducer;

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
pub use reducer::{AgentReducer, AgentSnapshot};

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
                    turn_state: Some(AgentTurnState::Working),
                },
            },
            5,
        ));
        assert!(reducer.apply(
            1,
            AgentEventKind::ChildFinished {
                child_id: "child-1".to_string(),
                outcome: TurnOutcome::Completed,
            },
            6,
        ));

        let child = &reducer.snapshot().children[0];
        assert_eq!(child.name.as_deref(), Some("Reviewer"));
        assert_eq!(child.primary_text(), "Review event mapping");
        assert_eq!(child.turn_state, AgentTurnState::Completed);
        assert!(child.current_action.is_none());
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
