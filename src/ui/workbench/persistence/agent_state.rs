//! Agent histories the Host stores beside a workspace layout: the Host owns
//! them, so a Client only reads them back into its panes when it opens the
//! workspace.

use std::{collections::HashSet, sync::Arc};

use sha2::{Digest as _, Sha256};
use yttt_agent_core::AgentSnapshot;
use yttt_client_core::ClientCoreError;
use yttt_core::model::workspace::WorkspaceState;
use yttt_protocol::{
    Response,
    workspace::{AgentStateRef, AgentStateSummary, WorkspaceResponse},
};

/// Agent state bodies by reference, as read from the Host.
pub(super) type AgentStateBodies = Vec<(AgentStateRef, Arc<Vec<u8>>)>;

/// The Agent states a window opened its workspace with.
pub(super) enum OpenedAgentStates {
    /// Whole, with their boards.
    Bodies(AgentStateBodies),
    /// Without their boards, which are read once something shows them.
    Summaries(Vec<AgentStateSummary>),
}

/// Whether `summaries` stand for every one of `references`; a Host that
/// predates summaries sends none.
pub(super) fn summaries_cover(
    references: &[AgentStateRef],
    summaries: &[AgentStateSummary],
) -> bool {
    references.len() == summaries.len()
        && references.iter().all(|reference| {
            summaries
                .iter()
                .any(|summary| summary.reference == *reference)
        })
}

/// Reads the referenced Agent states from the Host. A state the Host has
/// replaced since listing it is left out: the pane gets its live Agent state
/// once the Client is connected.
pub(super) fn read_published(
    references: Vec<AgentStateRef>,
    mut read: impl FnMut(&AgentStateRef) -> Result<Response, ClientCoreError>,
) -> Result<AgentStateBodies, String> {
    let mut states = Vec::with_capacity(references.len());
    for reference in references {
        match read(&reference) {
            Ok(Response::Workspace(WorkspaceResponse::AgentState {
                reference: received,
                content,
            })) if received == reference => states.push((reference, Arc::new(content))),
            Ok(_) => return Err("Host returned an unexpected Agent state body".into()),
            Err(ClientCoreError::Protocol(failure))
                if failure.code == yttt_protocol::FailureCode::NotFound => {}
            Err(error) => return Err(error.to_string()),
        }
    }
    Ok(states)
}

/// Writes each referenced Agent state into its pane. Fails on a duplicate
/// reference, a missing pane, or a body that does not match its reference.
pub(super) fn restore(
    workspace: &mut WorkspaceState,
    references: &[AgentStateRef],
    mut read: impl FnMut(&AgentStateRef) -> Result<Arc<Vec<u8>>, String>,
) -> Result<(), String> {
    restore_with(workspace, references, |reference| {
        decode(reference, &read(reference)?)
    })
}

/// Writes each summary's Agent state, without a board, into its pane.
pub(super) fn restore_summaries(
    workspace: &mut WorkspaceState,
    summaries: &[AgentStateSummary],
) -> Result<(), String> {
    let references = summaries
        .iter()
        .map(|summary| summary.reference.clone())
        .collect::<Vec<_>>();
    restore_with(workspace, &references, |reference| {
        summaries
            .iter()
            .find(|summary| summary.reference == *reference)
            .map(|summary| summary.snapshot.clone())
            .ok_or_else(|| "Agent state summary is missing".into())
    })
}

/// An Agent state body, checked against its reference.
pub(super) fn decode(reference: &AgentStateRef, content: &[u8]) -> Result<AgentSnapshot, String> {
    let digest: [u8; 32] = Sha256::digest(content).into();
    if digest != reference.content_sha256 || content.len() as u64 != reference.bytes {
        return Err("Agent state body does not match its manifest".into());
    }
    serde_json::from_slice(content).map_err(|error| format!("Agent state body is invalid: {error}"))
}

/// Gives a pane's Agent state, opened as a summary, the board of its stored
/// state: whole while the pane has no board, otherwise as retained history
/// behind the board live updates brought meanwhile.
pub(super) fn add_stored_board(current: &mut AgentSnapshot, stored: AgentSnapshot) {
    if current.board == yttt_agent_core::AgentBoardSnapshot::default() {
        current.board = stored.board;
    } else {
        current.board.merge_retained(&stored.board);
    }
}

fn restore_with(
    workspace: &mut WorkspaceState,
    references: &[AgentStateRef],
    mut snapshot_of: impl FnMut(&AgentStateRef) -> Result<AgentSnapshot, String>,
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for reference in references {
        if !seen.insert((&reference.project_id, &reference.tab_id, &reference.pane_id)) {
            return Err("Workspace contains duplicate Agent state references".into());
        }
        let snapshot = snapshot_of(reference)?;
        let pane = workspace
            .opened_projects
            .iter_mut()
            .find(|project| project.id == reference.project_id)
            .and_then(|project| {
                project
                    .tab_states
                    .iter_mut()
                    .find(|tab| tab.tab_id == reference.tab_id)
            })
            .and_then(|tab| {
                tab.pane_states
                    .iter_mut()
                    .find(|pane| pane.pane_id == reference.pane_id)
            })
            .ok_or("Agent state reference belongs to a missing pane")?;
        pane.agent_snapshot = Some(snapshot);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use yttt_protocol::{FailureCode, ProtocolFailure};

    use super::*;

    fn reference(pane: &str, content: &[u8]) -> AgentStateRef {
        AgentStateRef::for_content(
            yttt_core::model::ids::ProjectId::new("project"),
            "tab".into(),
            pane.into(),
            content,
        )
    }

    #[test]
    fn a_state_the_host_replaced_meanwhile_is_left_out() {
        let kept = reference("kept", b"{}");
        let replaced = reference("replaced", b"[]");
        let read = read_published(vec![kept.clone(), replaced.clone()], |wanted| {
            if *wanted == replaced {
                return Err(ClientCoreError::Protocol(ProtocolFailure {
                    code: FailureCode::NotFound,
                    message: "Agent state is not published by this workspace".into(),
                    retryable: false,
                }));
            }
            Ok(Response::Workspace(WorkspaceResponse::AgentState {
                reference: wanted.clone(),
                content: b"{}".to_vec(),
            }))
        })
        .unwrap();

        assert_eq!(read.len(), 1);
        assert_eq!(read[0].0, kept);
    }

    fn agent(messages: &[&str]) -> AgentSnapshot {
        let mut snapshot = yttt_agent_core::AgentReducer::new(
            yttt_agent_core::AgentInstanceId::new("agent").unwrap(),
            yttt_agent_core::ProviderId::from_static("omp"),
            1,
        )
        .snapshot()
        .clone();
        snapshot.board.messages = messages
            .iter()
            .map(|id| yttt_agent_core::AgentBoardMessage {
                id: (*id).into(),
                execution_id: "run".into(),
                role: yttt_agent_core::AgentMessageRole::Assistant,
                text: (*id).into(),
                timestamp: 1,
                truncated: false,
            })
            .collect();
        snapshot
    }

    fn summary(reference: &AgentStateRef) -> AgentStateSummary {
        AgentStateSummary {
            reference: reference.clone(),
            snapshot: agent(&[]),
        }
    }

    #[test]
    fn summaries_stand_for_the_states_only_when_they_cover_every_one() {
        let first = reference("first", b"{}");
        let second = reference("second", b"[]");
        let references = [first.clone(), second.clone()];
        assert!(summaries_cover(
            &references,
            &[summary(&second), summary(&first)]
        ));
        // An older Host sends no summaries.
        assert!(!summaries_cover(&references, &[]));
        assert!(!summaries_cover(
            &references,
            &[summary(&first), summary(&first)]
        ));
        assert!(summaries_cover(&[], &[]));
    }

    #[test]
    fn a_summary_for_a_missing_pane_fails_the_restore() {
        let mut workspace = WorkspaceState {
            opened_projects: Vec::new(),
            selected_project_id: None,
        };
        assert!(restore_summaries(&mut workspace, &[]).is_ok());
        assert!(restore_summaries(&mut workspace, &[summary(&reference("pane", b"{}"))]).is_err());
    }

    #[test]
    fn a_stored_board_fills_an_empty_one_and_trails_a_live_one() {
        let mut opened = agent(&[]);
        add_stored_board(&mut opened, agent(&["old-1", "old-2"]));
        assert_eq!(opened.board, agent(&["old-1", "old-2"]).board);

        let mut live = agent(&["new"]);
        add_stored_board(&mut live, agent(&["old-1", "new"]));
        let ids = live
            .board
            .messages
            .iter()
            .map(|message| message.id.as_str())
            .collect::<Vec<_>>();
        assert_eq!(ids, ["new", "old-1"]);
    }

    #[test]
    fn other_failures_still_fail_the_read() {
        let state = reference("pane", b"{}");
        let read = read_published(vec![state], |_| {
            Err(ClientCoreError::Protocol(ProtocolFailure {
                code: FailureCode::TransportClosed,
                message: "closed".into(),
                retryable: true,
            }))
        });

        assert!(read.is_err());
    }
}
