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
    workspace::{AgentStateRef, WorkspaceResponse},
};

/// Agent state bodies by reference, as read from the Host.
pub(super) type AgentStateBodies = Vec<(AgentStateRef, Arc<Vec<u8>>)>;

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
    let mut seen = HashSet::new();
    for reference in references {
        if !seen.insert((&reference.project_id, &reference.tab_id, &reference.pane_id)) {
            return Err("Workspace contains duplicate Agent state references".into());
        }
        let content = read(reference)?;
        let digest: [u8; 32] = Sha256::digest(content.as_slice()).into();
        if digest != reference.content_sha256 || content.len() as u64 != reference.bytes {
            return Err("Agent state body does not match its manifest".into());
        }
        let snapshot: AgentSnapshot = serde_json::from_slice(&content)
            .map_err(|error| format!("Agent state body is invalid: {error}"))?;
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
