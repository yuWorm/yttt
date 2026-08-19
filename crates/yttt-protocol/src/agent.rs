use serde::{Deserialize, Serialize};
use yttt_agent_core::AgentSnapshot;
use yttt_core::model::ids::TerminalSessionId;

pub const AGENT_HOOK_DELIVERY_PROTOCOL: u16 = 1;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHookDelivery<T> {
    pub protocol: u16,
    pub stream_id: String,
    pub sequence: u64,
    pub event: String,
    pub payload: T,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentHookAcknowledgement {
    pub accepted_sequence: u64,
    pub next_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentHookScope {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSnapshotCursor {
    pub scope: AgentHookScope,
    pub host_epoch: u64,
    pub sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSnapshotUpdate {
    pub scope: AgentHookScope,
    pub terminal_session_id: TerminalSessionId,
    pub host_epoch: u64,
    pub sequence: u64,
    pub snapshot: AgentSnapshot,
}
