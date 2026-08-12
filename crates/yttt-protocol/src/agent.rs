use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHookScope {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub generation: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHookEnvironment {
    pub variables: BTreeMap<String, String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHookEvent {
    pub scope: AgentHookScope,
    pub source: String,
    pub event: String,
    pub payload_json: Vec<u8>,
}
