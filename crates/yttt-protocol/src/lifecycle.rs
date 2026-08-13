use serde::{Deserialize, Serialize};

use crate::{BuildIdentity, HostBlocker};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HostLifecycleState {
    Running,
    Draining,
    ForceStopping,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HostLifecycleStatus {
    pub lifecycle_protocol: u16,
    pub resource_protocol: u16,
    pub build: BuildIdentity,
    pub state: HostLifecycleState,
    pub terminal_count: u32,
    pub client_count: u32,
    pub project_count: u32,
    pub ssh_connection_count: u32,
    pub agent_count: u32,
    pub blockers: Vec<HostBlocker>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleRequest {
    Probe,
    Status,
    StopIfIdle,
    BeginDrain,
    ForceStop,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleRequestEnvelope {
    pub request_id: u64,
    pub body: LifecycleRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleResponse {
    Pong,
    Status(HostLifecycleStatus),
    Stopping,
    Busy { blockers: Vec<HostBlocker> },
    Draining,
    PermissionDenied,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LifecycleResponseEnvelope {
    pub request_id: u64,
    pub result: LifecycleResponse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LifecycleMessage {
    Request(LifecycleRequestEnvelope),
    Response(LifecycleResponseEnvelope),
}
