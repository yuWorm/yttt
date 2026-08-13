use std::sync::Arc;

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
    events: Arc<flume::Receiver<AgentSnapshotUpdate>>,
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
        let (sender, events) = flume::bounded(256);
        for snapshot in runtime.agent_snapshots() {
            let _ = sender.try_send(snapshot);
        }
        std::thread::Builder::new()
            .name("yttt-agent-snapshots".to_string())
            .spawn(move || {
                while let Ok(event) = source.recv() {
                    let ClientEvent::AgentSnapshotUpdated(update) = event else {
                        continue;
                    };
                    if sender.try_send(*update).is_err() && sender.is_disconnected() {
                        break;
                    }
                }
            })
            .expect("failed to spawn Agent snapshot Host event bridge");
        Self {
            events: Arc::new(events),
        }
    }

    pub fn drain(&self) -> Vec<AgentSnapshotUpdate> {
        self.events.try_iter().take(256).collect()
    }
}
