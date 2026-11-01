use crate::model::layout::PaneKind;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentClassification {
    Agent,
    Shell,
}

impl AgentClassification {
    pub fn is_agent(self) -> bool {
        self == Self::Agent
    }
}

pub fn classify_agent(kind: Option<PaneKind>, command: &str) -> AgentClassification {
    if kind == Some(PaneKind::Agent)
        || yttt_agent_providers::provider_for_command(command).is_some()
    {
        AgentClassification::Agent
    } else {
        AgentClassification::Shell
    }
}
