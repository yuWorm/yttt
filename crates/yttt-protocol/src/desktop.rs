use serde::{Deserialize, Serialize};
use yttt_core::model::ids::ProfileId;

use crate::project::PlatformPath;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopShellRequest {
    Activate,
    OpenWindow { project_paths: Vec<PlatformPath> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopShellRequestEnvelope {
    pub protocol_version: u16,
    pub profile_id: ProfileId,
    pub request_id: u64,
    pub body: DesktopShellRequest,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopShellRejectReason {
    VersionMismatch { supported: u16 },
    ProfileMismatch,
    InvalidRequest,
    Busy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopShellResponse {
    Accepted,
    Rejected(DesktopShellRejectReason),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DesktopShellResponseEnvelope {
    pub request_id: u64,
    pub result: DesktopShellResponse,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DesktopShellMessage {
    Request(DesktopShellRequestEnvelope),
    Response(DesktopShellResponseEnvelope),
}
