use std::{
    collections::{HashMap, HashSet, VecDeque},
    ffi::OsStr,
};

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentProcessRecord {
    pub pid: u32,
    pub parent_pid: Option<u32>,
    pub agent: Option<BuiltinAgent>,
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

pub fn classify_agent_process<N, S>(process_name: N, command: &[S]) -> Option<BuiltinAgent>
where
    N: AsRef<OsStr>,
    S: AsRef<OsStr>,
{
    let process_name = process_name.as_ref().to_string_lossy();
    if let Some(agent) = classify_agent_executable(&process_name) {
        return Some(agent);
    }

    let executable = command.first()?.as_ref().to_string_lossy();
    if let Some(agent) = classify_agent_executable(&executable) {
        return Some(agent);
    }
    if !is_script_interpreter(&executable) {
        return None;
    }

    command
        .iter()
        .skip(1)
        .map(|argument| argument.as_ref().to_string_lossy())
        .find(|argument| !argument.starts_with('-'))
        .and_then(|script| {
            classify_agent_executable(&script).or_else(|| classify_agent_script_path(&script))
        })
}

pub fn detect_agent_processes_by_root(
    root_pids: &[u32],
    processes: &[AgentProcessRecord],
) -> HashMap<u32, BuiltinAgent> {
    let mut by_pid = HashMap::with_capacity(processes.len());
    let mut children_by_parent = HashMap::<u32, Vec<u32>>::new();
    for process in processes {
        by_pid.insert(process.pid, *process);
        if let Some(parent_pid) = process.parent_pid {
            children_by_parent
                .entry(parent_pid)
                .or_default()
                .push(process.pid);
        }
    }

    root_pids
        .iter()
        .filter_map(|root_pid| {
            nearest_agent_process(*root_pid, &by_pid, &children_by_parent)
                .map(|agent| (*root_pid, agent))
        })
        .collect()
}

fn nearest_agent_process(
    root_pid: u32,
    by_pid: &HashMap<u32, AgentProcessRecord>,
    children_by_parent: &HashMap<u32, Vec<u32>>,
) -> Option<BuiltinAgent> {
    let mut pending = VecDeque::from([root_pid]);
    let mut visited = HashSet::new();
    while let Some(pid) = pending.pop_front() {
        if !visited.insert(pid) {
            continue;
        }
        if let Some(agent) = by_pid.get(&pid).and_then(|process| process.agent) {
            return Some(agent);
        }
        if let Some(children) = children_by_parent.get(&pid) {
            pending.extend(children);
        }
    }
    None
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
        "opencode" => Some(BuiltinAgent::OpenCode),
        "pi" => Some(BuiltinAgent::Pi),
        "omp" => Some(BuiltinAgent::OhMyPi),
        executable if executable.starts_with("codex-") => Some(BuiltinAgent::Codex),
        _ => None,
    }
}

fn is_script_interpreter(executable: &str) -> bool {
    matches!(
        executable
            .rsplit(['/', '\\'])
            .next()
            .unwrap_or(executable)
            .to_ascii_lowercase()
            .as_str(),
        "node" | "node.exe" | "bun" | "bun.exe" | "deno" | "deno.exe"
    )
}

fn classify_agent_script_path(script: &str) -> Option<BuiltinAgent> {
    let normalized = script.replace('\\', "/").to_ascii_lowercase();
    if normalized.contains("/@oh-my-pi/pi-coding-agent/") {
        return Some(BuiltinAgent::OhMyPi);
    }
    if normalized.contains("/@mariozechner/pi-coding-agent/") {
        return Some(BuiltinAgent::Pi);
    }
    if normalized.contains("/@openai/codex/") || normalized.ends_with("/codex/bin/codex.js") {
        return Some(BuiltinAgent::Codex);
    }
    if normalized.contains("/@anthropic-ai/claude-code/") {
        return Some(BuiltinAgent::Claude);
    }
    if normalized.contains("/opencode-ai/") || normalized.contains("/opencode/bin/") {
        return Some(BuiltinAgent::OpenCode);
    }
    None
}

fn command_basename(command: &str) -> Option<&str> {
    let program = command.split_whitespace().next()?;
    program.rsplit(['/', '\\']).next()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_agent_interpreter_shims_by_entrypoint_name() {
        assert_eq!(
            classify_agent_process("bun", &["bun", "/Users/test/.bun/bin/omp"]),
            Some(BuiltinAgent::OhMyPi)
        );
        assert_eq!(
            classify_agent_process("node", &["node", "/usr/local/bin/claude"]),
            Some(BuiltinAgent::Claude)
        );
    }

    #[test]
    fn classifies_known_package_entrypoints_behind_interpreters() {
        assert_eq!(
            classify_agent_process(
                "bun",
                &[
                    "bun",
                    "/tmp/node_modules/@oh-my-pi/pi-coding-agent/dist/cli.js",
                ],
            ),
            Some(BuiltinAgent::OhMyPi)
        );
        assert_eq!(
            classify_agent_process(
                "node",
                &["node", "/tmp/node_modules/@openai/codex/bin/codex.js"],
            ),
            Some(BuiltinAgent::Codex)
        );
    }
}
