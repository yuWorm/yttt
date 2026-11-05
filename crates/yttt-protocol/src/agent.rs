use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::{Deserialize, Serialize};
pub use yttt_agent_core::ProjectTokenUsage;
use yttt_agent_core::{
    AgentBoardCommand, AgentInstanceId, AgentProcessState, AgentSnapshot, AgentTurnState,
    WaitingReason,
};
use yttt_core::model::{
    ids::TerminalSessionId,
    layout::{PaneController, PaneRole},
};

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

pub const AGENT_HOOK_ENDPOINT_ENV: &str = "YTTT_AGENT_HOOK_ENDPOINT";
pub const AGENT_HOOK_TOKEN_ENV: &str = "YTTT_AGENT_HOOK_TOKEN";
pub const AGENT_HOOK_SCOPE_ENV: &str = "YTTT_AGENT_HOOK_SCOPE";
/// The pane's own encoded scope, readable by anything running in it.
pub const AGENT_SELF_ENV: &str = "YTTT_SELF";
/// Present only in panes a person made orchestrators.
pub const AGENT_CONTROL_TOKEN_ENV: &str = "YTTT_AGENT_CONTROL_TOKEN";

/// Every variable the Host injects into a terminal. Inherited values are
/// removed before launch so a child cannot pass its identity on.
pub const HOST_TERMINAL_IDENTITY_ENVIRONMENT: [&str; 5] = [
    AGENT_HOOK_ENDPOINT_ENV,
    AGENT_HOOK_TOKEN_ENV,
    AGENT_HOOK_SCOPE_ENV,
    AGENT_SELF_ENV,
    AGENT_CONTROL_TOKEN_ENV,
];

pub const AGENT_CONTROL_TOKEN_HEADER: &str = "x-yttt-agent-control-token";
pub const AGENT_CONTROL_SCOPE_HEADER: &str = "x-yttt-agent-control-scope";
pub const AGENT_CONTROL_WHOAMI_PATH: &str = "/control/v1/whoami";
pub const AGENT_CONTROL_SPAWN_PATH: &str = "/control/v1/agents/spawn";
pub const AGENT_CONTROL_LIST_PATH: &str = "/control/v1/agents/list";
pub const AGENT_CONTROL_CLOSE_PATH: &str = "/control/v1/agents/close";
pub const AGENT_CONTROL_SEND_PATH: &str = "/control/v1/agents/send";
pub const AGENT_CONTROL_WAIT_PATH: &str = "/control/v1/agents/wait";
pub const AGENT_CONTROL_RESULT_PATH: &str = "/control/v1/agents/result";
pub const AGENT_CONTROL_INTERRUPT_PATH: &str = "/control/v1/agents/interrupt";
pub const AGENT_CONTROL_RELEASE_PATH: &str = "/control/v1/agents/release";
/// The most messages held for one worker.
pub const MAX_AGENT_QUEUED_MESSAGES: usize = 32;
/// The longest a single wait or interrupt confirmation may block.
pub const MAX_AGENT_WAIT_SECONDS: u32 = 600;
/// The most text an orchestrator may send in one request.
pub const MAX_AGENT_SEND_BYTES: usize = 64 * 1024;
/// The most of a final reply a result returns.
pub const MAX_AGENT_RESULT_BYTES: usize = 64 * 1024;

/// Requests that may block until an Agent changes state.
pub fn agent_control_blocks(path: &str) -> bool {
    matches!(path, AGENT_CONTROL_WAIT_PATH | AGENT_CONTROL_INTERRUPT_PATH)
}
/// Providers an orchestrator may start; each runs the command of the same name.
pub const AGENT_CONTROL_PROVIDERS: [&str; 7] =
    ["codex", "claude", "grok", "groky", "opencode", "pi", "omp"];

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentHookScope {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub generation: u64,
}

impl AgentHookScope {
    /// The form carried in `YTTT_AGENT_HOOK_SCOPE`, `YTTT_SELF` and the
    /// scope headers; tokens are computed over exactly these bytes.
    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(serde_json::to_vec(self).expect("Agent hook scope must serialize"))
    }

    /// Decodes and checks the shape only; it proves nothing about who sent it.
    pub fn decode(encoded: &str) -> Option<Self> {
        if encoded.len() > 4096 {
            return None;
        }
        let scope: Self = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(encoded).ok()?).ok()?;
        (scope.generation != 0
            && !scope.project_id.trim().is_empty()
            && !scope.tab_id.trim().is_empty()
            && !scope.pane_id.trim().is_empty())
        .then_some(scope)
    }
}

/// What the Host knows about the pane presenting a control credential.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentControlIdentity {
    pub project_id: String,
    pub terminal_session_id: TerminalSessionId,
    pub generation: u64,
    pub role: PaneRole,
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

/// Fences an interaction to the exact Host, terminal generation and agent instance shown.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentCommandRequest {
    pub scope: AgentHookScope,
    pub host_epoch: u64,
    pub instance_id: AgentInstanceId,
    pub command: AgentBoardCommand,
}

/// Starts a worker in a new tab of the orchestrator's project.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSpawnRequest {
    pub provider: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// Names one worker by its terminal.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTargetRequest {
    pub agent: TerminalSessionId,
}

/// A worker as an orchestrator sees it, without the Agent board history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlledAgent {
    /// The handle later requests use: the worker's terminal session.
    pub agent: TerminalSessionId,
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub title: String,
    pub command: String,
    /// `idle`, `running` or `exited`: the pane's terminal, not the Agent.
    pub pane_state: String,
    /// Whether the orchestrator or a person drives the worker now.
    #[serde(default)]
    pub controller: PaneController,
    /// Messages the Host holds until the worker can take them.
    #[serde(default)]
    pub queued: usize,
    /// Absent until the Agent's hooks report.
    pub agent_state: Option<ControlledAgentState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlledAgentState {
    pub provider: String,
    pub process: AgentProcessState,
    pub turn: AgentTurnState,
    /// The number of the current or last turn; see `AgentSnapshot::turn_seq`.
    pub turn_seq: u64,
    pub waiting: Option<WaitingReason>,
    pub task: Option<String>,
    pub updated_at: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentControlResponse {
    Spawned {
        agent: Box<ControlledAgent>,
    },
    Agents {
        agents: Vec<ControlledAgent>,
    },
    Closed {
        agent: TerminalSessionId,
    },
    /// The Host wrote the input. The turn it starts will be numbered above
    /// `turn_seq_before`; wait with `after_turn` set to it.
    Sent {
        agent: TerminalSessionId,
        turn_seq_before: u64,
        /// The Host holds the text until the worker is ready and driven by
        /// the orchestrator; its turn number is not known yet.
        #[serde(default)]
        queued: bool,
    },
    Released {
        agent: TerminalSessionId,
    },
    Waited {
        reason: AgentWaitReason,
        /// The Agent that ended the wait; absent after a timeout.
        agent: Option<TerminalSessionId>,
        agents: Vec<ControlledAgent>,
    },
    Result {
        result: Box<AgentTurnResult>,
    },
    Interrupted {
        agent: TerminalSessionId,
        /// Whether the Agent reported the interruption before the deadline.
        confirmed: bool,
    },
}

/// Writes text into a worker's prompt and presses Enter.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSendRequest {
    pub agent: TerminalSessionId,
    pub text: String,
    /// Hold the text while the worker is busy or a person drives it,
    /// instead of refusing.
    #[serde(default)]
    pub queue: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWaitCondition {
    /// A turn ended: completed, failed or interrupted.
    Finished,
    /// The Agent is waiting for an approval or an answer.
    Waiting,
    /// The Agent can take a new instruction.
    Ready,
    /// Anything about the Agent's process or turn changed.
    Change,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWaitRequest {
    pub agents: Vec<TerminalSessionId>,
    pub until: AgentWaitCondition,
    /// Only turns numbered above this count, for `finished` and `ready`.
    #[serde(default)]
    pub after_turn: Option<u64>,
    pub timeout_seconds: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentWaitReason {
    Matched,
    /// The Agent's process or pane ended; no condition can hold any more.
    Exited,
    /// A person took over, handed back or paused the worker.
    ControlChanged,
    TimedOut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentInterruptRequest {
    pub agent: TerminalSessionId,
    #[serde(default)]
    pub timeout_seconds: Option<u32>,
}

/// A worker's last reply, read from its provider's own transcript.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTurnResult {
    pub agent: TerminalSessionId,
    pub turn_seq: u64,
    pub turn: AgentTurnState,
    pub text: Option<String>,
    pub truncated: bool,
    pub source: AgentResultSource,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentResultSource {
    Transcript,
    /// The provider reported no transcript, or it held no reply yet.
    Unavailable,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentControlErrorCode {
    InvalidRequest,
    /// The caller may not act on that target.
    Forbidden,
    NotFound,
    /// The Agent is not in a state that accepts this; inspect it and retry.
    NotReady,
    /// A person drives the worker or paused it; wait for `control_changed`.
    HumanControl,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentControlError {
    pub code: AgentControlErrorCode,
    pub message: String,
}

impl AgentControlError {
    pub fn new(code: AgentControlErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}
