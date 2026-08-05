use serde::{Deserialize, Serialize};

use crate::{
    AgentAction, AgentSessionMetadata, AgentTask, ChildAgentDescriptor, ChildAgentUpdate,
    TurnOutcome, WaitingReason,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AgentEventKind {
    SessionStarted {
        metadata: AgentSessionMetadata,
    },
    SessionUpdated {
        metadata: AgentSessionMetadata,
    },
    TurnStarted {
        task: Option<AgentTask>,
    },
    Working,
    ActionStarted {
        action: AgentAction,
    },
    ActionFinished {
        action_id: Option<String>,
        failed: bool,
    },
    Waiting {
        reason: WaitingReason,
        message: Option<String>,
    },
    TurnFinished {
        outcome: TurnOutcome,
    },
    ChildStarted {
        child: ChildAgentDescriptor,
    },
    ChildUpdated {
        child_id: String,
        update: ChildAgentUpdate,
    },
    ChildFinished {
        child_id: String,
        outcome: TurnOutcome,
    },
}
