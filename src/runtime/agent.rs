use crate::{config::default_layout::BuiltinAgent, model::layout::PaneKind};

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

    if command_basename(command)
        .and_then(classify_agent_executable)
        .is_some()
    {
        AgentClassification::Agent
    } else {
        AgentClassification::Shell
    }
}

fn classify_agent_executable(executable: &str) -> Option<BuiltinAgent> {
    let basename = executable.rsplit(['/', '\\']).next()?.to_ascii_lowercase();
    let basename = basename
        .strip_suffix(".exe")
        .or_else(|| basename.strip_suffix(".cmd"))
        .or_else(|| basename.strip_suffix(".bat"))
        .or_else(|| basename.strip_suffix(".ps1"))
        .unwrap_or(&basename);
    match basename {
        "codex" => Some(BuiltinAgent::Codex),
        "claude" => Some(BuiltinAgent::Claude),
        "grok" | "groky" => Some(BuiltinAgent::Grok),
        "opencode" => Some(BuiltinAgent::OpenCode),
        "pi" => Some(BuiltinAgent::Pi),
        "omp" => Some(BuiltinAgent::OhMyPi),
        executable if executable.starts_with("codex-") => Some(BuiltinAgent::Codex),
        _ => None,
    }
}

fn command_basename(command: &str) -> Option<&str> {
    let program = command.split_whitespace().next()?;
    program.rsplit(['/', '\\']).next()
}
