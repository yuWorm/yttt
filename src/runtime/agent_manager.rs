use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use yttt_agent_core::{AgentSessionMetadata, ProviderResumeCommand};
use yttt_agent_providers::builtin_providers;
use yttt_agent_runtime::AgentRuntime;
use yttt_protocol::agent::AgentSnapshotUpdate;

use crate::runtime::agent_hooks::AgentSnapshotClient;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentPaneAddress {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
}

impl AgentPaneAddress {
    pub fn new(project_id: &str, tab_id: &str, pane_id: &str) -> Self {
        Self {
            project_id: project_id.to_string(),
            tab_id: tab_id.to_string(),
            pane_id: pane_id.to_string(),
        }
    }
}

/// The Client side of Agent tracking. The Host launches and resumes Agents
/// and keeps their snapshots in the workspace layout; this Client orders the
/// Host's snapshot events per pane and builds resume commands for sessions
/// the user picks.
pub struct AgentManager {
    runtime: AgentRuntime,
    snapshot_client: Option<AgentSnapshotClient>,
    host_snapshot_sequences: HashMap<AgentPaneAddress, (u64, u64, u64)>,
}

impl Default for AgentManager {
    fn default() -> Self {
        let mut runtime = AgentRuntime::default();
        for provider in builtin_providers() {
            runtime.register_provider(provider);
        }
        Self {
            runtime,
            snapshot_client: None,
            host_snapshot_sequences: HashMap::new(),
        }
    }
}

impl AgentManager {
    pub fn resume_command(
        &self,
        provider_id: &str,
        session: &AgentSessionMetadata,
    ) -> Option<ProviderResumeCommand> {
        self.runtime.resume_command(provider_id, session)
    }

    pub fn forget_project(&mut self, project_id: &str) {
        self.host_snapshot_sequences
            .retain(|address, _| address.project_id != project_id);
    }

    pub fn forget_tabs(&mut self, project_id: &str, tab_ids: &[String]) {
        self.host_snapshot_sequences.retain(|address, _| {
            address.project_id != project_id || !tab_ids.contains(&address.tab_id)
        });
    }

    pub fn forget_pane(&mut self, address: &AgentPaneAddress) {
        self.host_snapshot_sequences.remove(address);
    }

    /// Forgets the event order of a previous Host run.
    pub fn reset_host_sequences(&mut self) {
        self.host_snapshot_sequences.clear();
    }

    pub fn set_snapshot_client(&mut self, client: Option<AgentSnapshotClient>) {
        self.snapshot_client = client;
        self.host_snapshot_sequences.clear();
    }

    pub fn snapshot_client(&self) -> Option<AgentSnapshotClient> {
        self.snapshot_client.clone()
    }

    /// Whether `update` is newer than every snapshot event already applied
    /// to the pane; records it as the latest when it is.
    pub fn accept_host_snapshot(
        &mut self,
        address: &AgentPaneAddress,
        update: &AgentSnapshotUpdate,
    ) -> bool {
        let cursor = (update.host_epoch, update.scope.generation, update.sequence);
        if self
            .host_snapshot_sequences
            .get(address)
            .is_some_and(|current| *current >= cursor)
        {
            return false;
        }
        self.host_snapshot_sequences.insert(address.clone(), cursor);
        true
    }
}

#[cfg(test)]
mod tests {
    use yttt_agent_core::{AgentInstanceId, AgentReducer, ProviderId};
    use yttt_core::model::ids::TerminalSessionId;
    use yttt_protocol::agent::AgentHookScope;

    use super::*;

    #[test]
    fn accepts_host_snapshots_in_epoch_generation_sequence_order() {
        let mut manager = AgentManager::default();
        let address = AgentPaneAddress::new("project", "tab", "pane");
        let update = |host_epoch, generation, sequence| AgentSnapshotUpdate {
            scope: AgentHookScope {
                project_id: "project".to_string(),
                tab_id: "tab".to_string(),
                pane_id: "pane".to_string(),
                generation,
            },
            terminal_session_id: TerminalSessionId::new("project:tab:pane"),
            host_epoch,
            sequence,
            snapshot: AgentReducer::new(
                AgentInstanceId::new("agent").unwrap(),
                ProviderId::new("codex").unwrap(),
                1,
            )
            .snapshot()
            .clone(),
        };

        assert!(manager.accept_host_snapshot(&address, &update(4, 3, 2)));
        assert!(!manager.accept_host_snapshot(&address, &update(4, 3, 2)));
        assert!(!manager.accept_host_snapshot(&address, &update(4, 3, 1)));
        // A restarted process starts its sequence over.
        assert!(manager.accept_host_snapshot(&address, &update(4, 4, 1)));
        assert!(!manager.accept_host_snapshot(&address, &update(3, 9, 9)));
        assert!(manager.accept_host_snapshot(&address, &update(5, 1, 1)));

        manager.forget_pane(&address);
        assert!(manager.accept_host_snapshot(&address, &update(4, 3, 2)));
    }
}
