use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AgentInstanceId(String);

impl AgentInstanceId {
    pub fn random() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn new(value: impl Into<String>) -> Result<Self, AgentIdError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(AgentIdError::Empty);
        }
        if value.len() > 128 {
            return Err(AgentIdError::TooLong);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for AgentInstanceId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum AgentIdError {
    #[error("agent id cannot be empty")]
    Empty,
    #[error("agent id exceeds 128 bytes")]
    TooLong,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProviderId(String);

impl ProviderId {
    pub fn new(value: impl Into<String>) -> Result<Self, ProviderIdError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        {
            return Err(ProviderIdError(value));
        }
        Ok(Self(value))
    }

    pub fn from_static(value: &'static str) -> Self {
        Self::new(value).expect("static provider ids must be valid")
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ProviderId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
#[error("invalid provider id: {0}")]
pub struct ProviderIdError(String);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentProcessState {
    Registered,
    Starting,
    Running,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTurnState {
    Idle,
    Working,
    Waiting,
    Completed,
    Failed,
    Interrupted,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentViewState {
    Starting,
    Idle,
    Working,
    Waiting,
    Completed,
    Failed,
    Interrupted,
    Stale,
}

impl AgentViewState {
    pub const fn label(self) -> &'static str {
        match self {
            Self::Starting => "agent starting",
            Self::Idle => "agent idle",
            Self::Working => "agent working",
            Self::Waiting => "agent waiting",
            Self::Completed => "agent completed",
            Self::Failed => "agent failed",
            Self::Interrupted => "agent interrupted",
            Self::Stale => "agent stale",
        }
    }

    pub const fn priority(self) -> u8 {
        match self {
            Self::Idle | Self::Stale => 0,
            Self::Completed | Self::Interrupted => 1,
            Self::Starting | Self::Working => 2,
            Self::Failed => 3,
            Self::Waiting => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaitingReason {
    Approval,
    UserInput,
    External,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentExitReason {
    Completed,
    Failed,
    KilledByUser,
    Disconnected,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentProcessExit {
    pub code: Option<i32>,
    pub reason: AgentExitReason,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TurnOutcome {
    Completed,
    Failed,
    Interrupted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentTask {
    pub title: String,
    pub source: AgentTaskSource,
}

impl AgentTask {
    pub fn new(title: impl Into<String>, source: AgentTaskSource) -> Option<Self> {
        let title = bounded_text(title.into(), 8 * 1024);
        (!title.is_empty()).then_some(Self { title, source })
    }

    pub fn single_line_title(&self) -> String {
        bounded_text(
            self.title.split_whitespace().collect::<Vec<_>>().join(" "),
            160,
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentTaskSource {
    UserPromptHook,
    InitialLaunchPrompt,
    WorktreeLaunch,
    External,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAction {
    pub id: Option<String>,
    pub name: String,
    pub detail: Option<String>,
}

impl AgentAction {
    pub fn new(
        id: Option<String>,
        name: impl Into<String>,
        detail: Option<String>,
    ) -> Option<Self> {
        let name = bounded_text(name.into(), 128);
        (!name.is_empty()).then_some(Self {
            id: id.map(|value| bounded_text(value, 256)),
            name,
            detail: detail
                .map(|value| bounded_text(value, 4 * 1024))
                .filter(|value| !value.is_empty()),
        })
    }

    pub fn label(&self) -> String {
        match self.detail.as_deref() {
            Some(detail) => format!("{}: {detail}", self.name),
            None => self.name.clone(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildAgentDescriptor {
    pub id: String,
    pub name: Option<String>,
    pub task: Option<AgentTask>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildAgentUpdate {
    pub task: Option<AgentTask>,
    pub current_action: Option<AgentAction>,
    pub turn_state: Option<AgentTurnState>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChildAgentSnapshot {
    pub id: String,
    pub name: Option<String>,
    pub task: Option<AgentTask>,
    pub current_action: Option<AgentAction>,
    pub turn_state: AgentTurnState,
    pub started_at: u64,
    pub updated_at: u64,
}

impl ChildAgentSnapshot {
    pub fn primary_text(&self) -> String {
        self.task
            .as_ref()
            .map(AgentTask::single_line_title)
            .or_else(|| self.name.clone())
            .unwrap_or_else(|| self.id.clone())
    }

    pub fn secondary_text(&self) -> Option<String> {
        self.current_action.as_ref().map(AgentAction::label)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionMetadata {
    #[serde(default)]
    pub session_id: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub transcript_path: Option<String>,
}

pub(crate) fn bounded_text(value: String, max_bytes: usize) -> String {
    let value = value.trim();
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value[..end].trim_end().to_string()
}
