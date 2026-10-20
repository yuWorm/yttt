use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use sha2::{Digest as _, Sha256};
use yttt_agent_core::AgentSnapshot;
use yttt_core::model::{
    ids::ProjectId,
    workspace::{Workspace, WorkspaceState},
};
use yttt_protocol::workspace::{
    AgentStateRef, MAX_AGENT_STATE_BYTES, MAX_WORKSPACE_AGENT_STATE_BYTES,
};

pub(super) type AgentStateUpload = (AgentStateRef, Arc<Vec<u8>>);
type Address = (ProjectId, String, String);
pub(super) type AgentStateCache = HashMap<Address, CachedAgentState>;

pub(super) struct CachedAgentState {
    snapshot: AgentSnapshot,
    upload: AgentStateUpload,
}

pub(super) fn capture(
    workspace: &Workspace,
    cache: &mut AgentStateCache,
    bounded: bool,
) -> Result<Vec<AgentStateUpload>, String> {
    let mut uploads = Vec::new();
    let mut retained = HashSet::new();
    let mut total = 0u64;
    for project in workspace.opened_projects() {
        for tab in &project.tab_states {
            for pane in &tab.pane_states {
                let Some(snapshot) = pane.agent_snapshot.as_ref() else {
                    continue;
                };
                let address = (project.id.clone(), tab.tab_id.clone(), pane.pane_id.clone());
                retained.insert(address.clone());
                let unchanged = cache.get_mut(&address).is_some_and(|cached| {
                    // Heartbeats are liveness observations, not workspace edits.
                    let observed_at = cached.snapshot.updated_at;
                    cached.snapshot.updated_at = snapshot.updated_at;
                    let unchanged = snapshot == &cached.snapshot;
                    cached.snapshot.updated_at = observed_at;
                    unchanged
                });
                if !unchanged {
                    let content =
                        serde_json::to_vec(snapshot).map_err(|error| error.to_string())?;
                    let reference = AgentStateRef::for_content(
                        project.id.clone(),
                        tab.tab_id.clone(),
                        pane.pane_id.clone(),
                        &content,
                    );
                    cache.insert(
                        address.clone(),
                        CachedAgentState {
                            snapshot: snapshot.clone(),
                            upload: (reference, Arc::new(content)),
                        },
                    );
                }
                let upload = &cache.get(&address).expect("captured Agent state").upload;
                total = total.saturating_add(upload.0.bytes);
                if bounded && upload.0.bytes > MAX_AGENT_STATE_BYTES as u64 {
                    return Err("Agent state exceeds the 16 MiB object limit".into());
                }
                if bounded && total > MAX_WORKSPACE_AGENT_STATE_BYTES as u64 {
                    return Err("Workspace Agent states exceed the 256 MiB object limit".into());
                }
                uploads.push(upload.clone());
            }
        }
    }
    cache.retain(|address, _| retained.contains(address));
    Ok(uploads)
}

pub(super) fn validate_references(
    workspace: &WorkspaceState,
    references: &[AgentStateRef],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for reference in references {
        let address = (&reference.project_id, &reference.tab_id, &reference.pane_id);
        if !seen.insert(address) {
            return Err("Workspace contains duplicate Agent state references".into());
        }
        let pane = workspace
            .opened_projects
            .iter()
            .find(|project| project.id == reference.project_id)
            .and_then(|project| {
                project
                    .tab_states
                    .iter()
                    .find(|tab| tab.tab_id == reference.tab_id)
            })
            .and_then(|tab| {
                tab.pane_states
                    .iter()
                    .find(|pane| pane.pane_id == reference.pane_id)
            })
            .ok_or("Agent state reference belongs to a missing pane")?;
        if pane.agent_snapshot.is_some() {
            return Err("Pane contains both inline and referenced Agent state".into());
        }
    }
    Ok(())
}

pub(super) fn restore(
    workspace: &mut WorkspaceState,
    references: &[AgentStateRef],
    mut read: impl FnMut(&AgentStateRef) -> Result<Arc<Vec<u8>>, String>,
) -> Result<AgentStateCache, String> {
    validate_references(workspace, references)?;
    let mut cache = AgentStateCache::new();
    for reference in references {
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
        pane.agent_snapshot = Some(snapshot.clone());
        cache.insert(
            (
                reference.project_id.clone(),
                reference.tab_id.clone(),
                reference.pane_id.clone(),
            ),
            CachedAgentState {
                snapshot,
                upload: (reference.clone(), content),
            },
        );
    }
    Ok(cache)
}
