use serde::{Deserialize, Serialize};
use yttt_core::model::ids::ClientInstanceId;

use crate::workspace::WorkspaceId;

/// Captured when a mutation is admitted by the Client, not when it leaves a queue.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlContext {
    pub host_epoch: u64,
    pub control_epoch: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceRevision {
    pub workspace_id: WorkspaceId,
    pub revision: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransferPhase {
    Preparing,
    ForceConfirmationRequired,
    Fencing,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlTransfer {
    pub id: String,
    pub requester: ClientInstanceId,
    pub previous_owner: Option<ClientInstanceId>,
    pub phase: TransferPhase,
    pub deadline_millis: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlStatus {
    pub context: ControlContext,
    pub revision: u64,
    pub owner: Option<ClientInstanceId>,
    pub transfer: Option<ControlTransfer>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProfileControlRequest {
    Status,
    /// Starts automatic publication by the previous Client; never grants on timeout.
    RequestControl,
    Ready {
        transfer_id: String,
        revisions: Vec<WorkspaceRevision>,
    },
    Cancel {
        transfer_id: String,
    },
    /// A separate user confirmation after the preparation deadline.
    ConfirmForce {
        transfer_id: String,
    },
    Release,
}
