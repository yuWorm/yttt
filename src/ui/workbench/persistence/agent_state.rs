//! Agent histories the Host stores beside a workspace layout: the Host owns
//! them, so a Client only reads them back into its panes when it opens the
//! workspace.

use std::{collections::HashSet, sync::Arc};

use sha2::{Digest as _, Sha256};
use yttt_agent_core::AgentSnapshot;
use yttt_core::model::workspace::WorkspaceState;
use yttt_protocol::workspace::AgentStateRef;

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
