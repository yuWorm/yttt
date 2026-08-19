use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use yttt_client_core::ClientEvent;
use yttt_protocol::agent::AgentSnapshotUpdate;

use crate::{host_runtime::DesktopHostRuntime, model::ids::TerminalSessionId};

pub mod installer;

pub const AGENT_HOOK_ENVIRONMENT_VARIABLES: [&str; 3] = [
    "YTTT_AGENT_HOOK_ENDPOINT",
    "YTTT_AGENT_HOOK_TOKEN",
    "YTTT_AGENT_HOOK_SCOPE",
];

#[derive(Default)]
struct PendingAgentSnapshots {
    by_session: HashMap<TerminalSessionId, AgentSnapshotUpdate>,
}

impl PendingAgentSnapshots {
    fn push(&mut self, update: AgentSnapshotUpdate) {
        let cursor = (update.host_epoch, update.scope.generation, update.sequence);
        if self
            .by_session
            .get(&update.terminal_session_id)
            .is_none_or(|current| {
                (
                    current.host_epoch,
                    current.scope.generation,
                    current.sequence,
                ) < cursor
            })
        {
            self.by_session
                .insert(update.terminal_session_id.clone(), update);
        }
    }

    fn drain(&mut self) -> Vec<AgentSnapshotUpdate> {
        std::mem::take(&mut self.by_session).into_values().collect()
    }
}

#[derive(Clone)]
pub struct AgentSnapshotClient {
    pending: Arc<Mutex<PendingAgentSnapshots>>,
}

impl std::fmt::Debug for AgentSnapshotClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentSnapshotClient")
            .finish_non_exhaustive()
    }
}

impl AgentSnapshotClient {
    pub fn new(runtime: Arc<DesktopHostRuntime>) -> Self {
        let source = runtime.events();
        let pending = Arc::new(Mutex::new(PendingAgentSnapshots::default()));
        {
            let mut pending = pending
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            for snapshot in runtime.agent_snapshots() {
                pending.push(snapshot);
            }
        }

        let pending_for_bridge = Arc::downgrade(&pending);
        std::thread::Builder::new()
            .name("yttt-agent-snapshots".to_string())
            .spawn(move || {
                while let Ok(event) = source.recv() {
                    let ClientEvent::AgentSnapshotUpdated(update) = event else {
                        continue;
                    };
                    let Some(pending) = pending_for_bridge.upgrade() else {
                        break;
                    };
                    pending
                        .lock()
                        .unwrap_or_else(|poisoned| poisoned.into_inner())
                        .push(*update);
                }
            })
            .expect("failed to spawn Agent snapshot Host event bridge");
        Self { pending }
    }

    pub fn drain(&self) -> Vec<AgentSnapshotUpdate> {
        self.pending
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .drain()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_agent_core::{
        AgentInstanceId, AgentProcessState, AgentSnapshot, AgentTurnState, ProviderId,
    };

    fn update(
        session_id: &str,
        host_epoch: u64,
        generation: u64,
        sequence: u64,
    ) -> AgentSnapshotUpdate {
        AgentSnapshotUpdate {
            scope: yttt_protocol::agent::AgentHookScope {
                project_id: "project".to_string(),
                tab_id: "tab".to_string(),
                pane_id: "pane".to_string(),
                generation,
            },
            terminal_session_id: TerminalSessionId::new(session_id),
            host_epoch,
            sequence,
            snapshot: AgentSnapshot {
                instance_id: AgentInstanceId::new("agent").unwrap(),
                provider_id: ProviderId::from_static("omp"),
                generation,
                process_state: AgentProcessState::Running,
                turn_state: AgentTurnState::Idle,
                waiting_reason: None,
                waiting_message: None,
                task: None,
                current_action: None,
                last_action_failed: false,
                children: Vec::new(),
                session: None,
                process_exit: None,
                state_started_at: 0,
                updated_at: sequence,
            },
        }
    }

    #[test]
    fn pending_snapshots_keep_only_the_latest_update_per_session() {
        let mut pending = PendingAgentSnapshots::default();
        pending.push(update("a", 1, 1, 2));
        pending.push(update("a", 1, 1, 1));
        pending.push(update("a", 0, 99, 99));
        pending.push(update("a", 1, 2, 1));
        pending.push(update("b", 1, 1, 3));

        let mut updates = pending.drain();
        updates.sort_by(|left, right| {
            left.terminal_session_id
                .as_str()
                .cmp(right.terminal_session_id.as_str())
        });

        assert_eq!(updates.len(), 2);
        assert_eq!(updates[0].terminal_session_id.as_str(), "a");
        assert_eq!(updates[0].scope.generation, 2);
        assert_eq!(updates[0].sequence, 1);
        assert_eq!(updates[1].terminal_session_id.as_str(), "b");
        assert_eq!(updates[1].sequence, 3);
        assert!(pending.drain().is_empty());
    }
}
