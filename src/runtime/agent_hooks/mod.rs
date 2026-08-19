use std::{
    collections::VecDeque,
    sync::{Arc, Mutex},
};

use yttt_client_core::ClientEvent;
use yttt_protocol::agent::AgentSnapshotUpdate;

use crate::host_runtime::DesktopHostRuntime;

pub mod installer;

pub const AGENT_HOOK_ENVIRONMENT_VARIABLES: [&str; 3] = [
    "YTTT_AGENT_HOOK_ENDPOINT",
    "YTTT_AGENT_HOOK_TOKEN",
    "YTTT_AGENT_HOOK_SCOPE",
];

#[derive(Clone)]
pub struct AgentSnapshotClient {
    initial: Arc<Mutex<VecDeque<AgentSnapshotUpdate>>>,
    events: flume::Receiver<ClientEvent>,
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
        let events = runtime.events();
        let initial = runtime.agent_snapshots().into();
        Self {
            initial: Arc::new(Mutex::new(initial)),
            events,
        }
    }

    pub async fn recv(&self) -> Option<AgentSnapshotUpdate> {
        if let Some(update) = self
            .initial
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .pop_front()
        {
            return Some(update);
        }
        loop {
            match self.events.recv_async().await.ok()? {
                ClientEvent::AgentSnapshotUpdated(update) => return Some(*update),
                _ => continue,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ids::TerminalSessionId;
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
    fn snapshot_client_yields_initial_state_before_live_events() {
        let initial = update("initial", 1, 1, 2);
        let live = update("live", 1, 1, 3);
        let (sender, events) = flume::unbounded();
        let client = AgentSnapshotClient {
            initial: Arc::new(Mutex::new(VecDeque::from([initial]))),
            events,
        };
        sender
            .send(ClientEvent::AgentSnapshotUpdated(Box::new(live)))
            .unwrap();
        drop(sender);

        futures_lite::future::block_on(async {
            assert_eq!(
                client.recv().await.unwrap().terminal_session_id.as_str(),
                "initial"
            );
            assert_eq!(
                client.recv().await.unwrap().terminal_session_id.as_str(),
                "live"
            );
            assert!(client.recv().await.is_none());
        });
    }
}
