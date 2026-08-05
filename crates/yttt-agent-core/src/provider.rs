use serde_json::Value;

use crate::{AgentEventKind, ProviderId};

#[derive(Clone, Debug)]
pub struct ProviderDescriptor {
    pub id: ProviderId,
    pub display_name: &'static str,
}

#[derive(Clone, Debug)]
pub struct ProviderHookEvent<'a> {
    pub name: &'a str,
    pub payload: &'a Value,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProviderResumeCommand {
    pub program: &'static str,
    pub arguments: Vec<String>,
}

pub trait AgentProvider: Send + Sync + 'static {
    fn descriptor(&self) -> ProviderDescriptor;

    fn matches_command(&self, command: &str) -> bool;

    fn resume_command(
        &self,
        _session: &crate::AgentSessionMetadata,
    ) -> Option<ProviderResumeCommand> {
        None
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError>;
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProviderError {
    #[error("unsupported provider event: {0}")]
    UnsupportedEvent(String),
    #[error("invalid provider payload: {0}")]
    InvalidPayload(String),
}
