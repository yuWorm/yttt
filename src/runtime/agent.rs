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
    if kind == Some(PaneKind::Agent) {
        return AgentClassification::Agent;
    }

    match command_basename(command) {
        Some("codex" | "claude") => AgentClassification::Agent,
        _ => AgentClassification::Shell,
    }
}

fn command_basename(command: &str) -> Option<&str> {
    let program = command.split_whitespace().next()?;
    program.rsplit('/').next()
}
