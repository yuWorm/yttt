use std::sync::Arc;

use crate::{
    config::default_layout::BuiltinAgent, model::ids::ProjectId,
    runtime::agent_sessions::AgentSession,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct AgentSessionScanKey {
    pub(in super::super) project_id: ProjectId,
    pub(in super::super) agent: BuiltinAgent,
}

#[derive(Default)]
pub(in super::super) struct AgentSessionsControllerState {
    pub(in super::super) generation: u64,
    pub(in super::super) pending_scan: bool,
    pub(in super::super) loading: bool,
    pub(in super::super) key: Option<AgentSessionScanKey>,
    pub(in super::super) sessions: Arc<Vec<AgentSession>>,
    pub(in super::super) error: Option<String>,
}

impl AgentSessionsControllerState {
    pub(in super::super) fn clear(&mut self) {
        self.generation = self.generation.wrapping_add(1);
        self.pending_scan = false;
        self.loading = false;
        self.key = None;
        self.sessions = Arc::new(Vec::new());
        self.error = None;
    }
}
