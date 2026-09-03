use std::{
    borrow::Cow,
    collections::{HashMap, HashSet, VecDeque},
    ffi::{OsStr, OsString},
    sync::Arc,
    time::Duration,
};

use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use tokio::sync::watch;
use yttt_agent_core::{AgentProvider, ProviderId};
use yttt_agent_providers::{
    CLAUDE_PROVIDER_ID, CODEX_PROVIDER_ID, OMP_PROVIDER_ID, OPENCODE_PROVIDER_ID, PI_PROVIDER_ID,
    builtin_providers,
};
use yttt_core::model::ids::TerminalSessionId;

use crate::{agent_hooks::HostAgentHookRuntime, runtime::HostRuntime};

const SCAN_INTERVAL: Duration = Duration::from_millis(500);

pub(crate) async fn run(
    runtime: Arc<HostRuntime>,
    agent_hooks: Arc<HostAgentHookRuntime>,
    mut stop: watch::Receiver<bool>,
) {
    if !sysinfo::IS_SUPPORTED_SYSTEM {
        return;
    }

    let mut scanner = AgentProcessScanner::new();
    let mut interval = tokio::time::interval(SCAN_INTERVAL);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            _ = interval.tick() => {}
            changed = stop.changed() => {
                if changed.is_err() || *stop.borrow() {
                    break;
                }
                continue;
            }
        }

        let roots = runtime.local_process_roots();
        let scan = tokio::task::spawn_blocking(move || {
            let detected = scanner.scan(&roots);
            (scanner, roots, detected)
        })
        .await;
        let Ok((next_scanner, scanned_roots, mut detected)) = scan else {
            break;
        };
        scanner = next_scanner;
        let current_roots = runtime.local_process_roots();
        detected.retain(|session_id, _| {
            current_roots.iter().any(|current| {
                scanned_roots
                    .iter()
                    .any(|scanned| scanned == current && &current.0 == session_id)
            })
        });
        agent_hooks.reconcile_process_scan(&current_roots, &detected);
    }
}

struct AgentProcessScanner {
    system: System,
    providers: Vec<Arc<dyn AgentProvider>>,
}

impl AgentProcessScanner {
    fn new() -> Self {
        Self {
            system: System::new(),
            providers: builtin_providers(),
        }
    }

    fn scan(
        &mut self,
        roots: &[(TerminalSessionId, u32)],
    ) -> HashMap<TerminalSessionId, DetectedAgentProcess> {
        if roots.is_empty() {
            return HashMap::new();
        }

        self.system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            ProcessRefreshKind::new().with_cmd(UpdateKind::Always),
        );
        let processes = self
            .system
            .processes()
            .iter()
            .map(|(pid, process)| AgentProcessRecord {
                pid: pid.as_u32(),
                parent_pid: process.parent().map(|parent| parent.as_u32()),
                provider_id: classify_agent_process(&self.providers, process.name(), process.cmd()),
                blocks_descendant_discovery: blocks_agent_process_discovery(
                    process.name(),
                    process.cmd(),
                ),
            })
            .collect::<Vec<_>>();
        let root_pids = roots.iter().map(|(_, pid)| *pid).collect::<Vec<_>>();
        let detected = detect_agent_processes_by_root(&root_pids, &processes);
        roots
            .iter()
            .filter_map(|(session_id, pid)| {
                detected
                    .get(pid)
                    .cloned()
                    .map(|process| (session_id.clone(), process))
            })
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DetectedAgentProcess {
    pub pid: u32,
    pub provider_id: ProviderId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentProcessRecord {
    pid: u32,
    parent_pid: Option<u32>,
    provider_id: Option<ProviderId>,
    blocks_descendant_discovery: bool,
}

fn classify_agent_process(
    providers: &[Arc<dyn AgentProvider>],
    process_name: &OsStr,
    command: &[OsString],
) -> Option<ProviderId> {
    if let Some(provider_id) = provider_for_executable(providers, &process_name.to_string_lossy()) {
        return Some(provider_id);
    }

    let executable = command.first()?.to_string_lossy();
    if let Some(provider_id) = provider_for_executable(providers, &executable) {
        return Some(provider_id);
    }
    if !is_script_interpreter(&executable) {
        return None;
    }

    let script = command
        .iter()
        .skip(1)
        .map(|argument| argument.to_string_lossy())
        .find(|argument| !argument.starts_with('-'))?;
    provider_for_executable(providers, &script)
        .or_else(|| provider_for_script_path(providers, &script))
}

fn provider_for_executable(
    providers: &[Arc<dyn AgentProvider>],
    executable: &str,
) -> Option<ProviderId> {
    let basename = executable.rsplit(['/', '\\']).next()?;
    let basename = [".exe", ".cmd", ".bat", ".ps1"]
        .into_iter()
        .find(|suffix| {
            basename
                .get(basename.len().saturating_sub(suffix.len())..)
                .is_some_and(|ending| ending.eq_ignore_ascii_case(suffix))
        })
        .map_or(basename, |suffix| {
            &basename[..basename.len() - suffix.len()]
        });
    let normalized = if basename.bytes().any(|byte| byte.is_ascii_uppercase()) {
        Cow::Owned(basename.to_ascii_lowercase())
    } else {
        Cow::Borrowed(basename)
    };
    providers
        .iter()
        .find(|provider| provider.matches_command(&normalized))
        .map(|provider| provider.descriptor().id)
        .or_else(|| {
            normalized
                .starts_with("codex-")
                .then(|| provider_by_id(providers, CODEX_PROVIDER_ID))
                .flatten()
        })
}

fn provider_for_script_path(
    providers: &[Arc<dyn AgentProvider>],
    script: &str,
) -> Option<ProviderId> {
    let normalized = script.replace('\\', "/").to_ascii_lowercase();
    let provider_id = if normalized.contains("/@oh-my-pi/pi-coding-agent/") {
        OMP_PROVIDER_ID
    } else if normalized.contains("/@mariozechner/pi-coding-agent/") {
        PI_PROVIDER_ID
    } else if normalized.contains("/@openai/codex/") || normalized.ends_with("/codex/bin/codex.js")
    {
        CODEX_PROVIDER_ID
    } else if normalized.contains("/@anthropic-ai/claude-code/") {
        CLAUDE_PROVIDER_ID
    } else if normalized.contains("/opencode-ai/") || normalized.contains("/opencode/bin/") {
        OPENCODE_PROVIDER_ID
    } else {
        return None;
    };
    provider_by_id(providers, provider_id)
}

fn provider_by_id(providers: &[Arc<dyn AgentProvider>], provider_id: &str) -> Option<ProviderId> {
    providers
        .iter()
        .map(|provider| provider.descriptor().id)
        .find(|candidate| candidate.as_str() == provider_id)
}

fn blocks_agent_process_discovery(process_name: &OsStr, command: &[OsString]) -> bool {
    is_yttt_executable(&process_name.to_string_lossy())
        || command
            .first()
            .is_some_and(|executable| is_yttt_executable(&executable.to_string_lossy()))
}

fn detect_agent_processes_by_root(
    root_pids: &[u32],
    processes: &[AgentProcessRecord],
) -> HashMap<u32, DetectedAgentProcess> {
    let mut by_pid = HashMap::with_capacity(processes.len());
    let mut children_by_parent = HashMap::<u32, Vec<u32>>::new();
    for process in processes {
        by_pid.insert(process.pid, process);
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
                .map(|process| (*root_pid, process))
        })
        .collect()
}

fn nearest_agent_process(
    root_pid: u32,
    by_pid: &HashMap<u32, &AgentProcessRecord>,
    children_by_parent: &HashMap<u32, Vec<u32>>,
) -> Option<DetectedAgentProcess> {
    let mut pending = VecDeque::from([root_pid]);
    let mut visited = HashSet::new();
    while let Some(pid) = pending.pop_front() {
        if !visited.insert(pid) {
            continue;
        }
        let Some(process) = by_pid.get(&pid) else {
            continue;
        };
        if process.blocks_descendant_discovery {
            continue;
        }
        if let Some(provider_id) = &process.provider_id {
            return Some(DetectedAgentProcess {
                pid,
                provider_id: provider_id.clone(),
            });
        }
        if let Some(children) = children_by_parent.get(&pid) {
            pending.extend(children);
        }
    }
    None
}

fn is_script_interpreter(executable: &str) -> bool {
    matches!(
        executable.rsplit(['/', '\\']).next().unwrap_or(executable),
        name if ["node", "node.exe", "bun", "bun.exe", "deno", "deno.exe"]
            .iter()
            .any(|candidate| name.eq_ignore_ascii_case(candidate))
    )
}

fn is_yttt_executable(executable: &str) -> bool {
    let basename = executable.rsplit(['/', '\\']).next().unwrap_or(executable);
    basename.eq_ignore_ascii_case("yttt") || basename.eq_ignore_ascii_case("yttt.exe")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(
        pid: u32,
        parent_pid: Option<u32>,
        provider_id: Option<&'static str>,
        blocks_descendant_discovery: bool,
    ) -> AgentProcessRecord {
        AgentProcessRecord {
            pid,
            parent_pid,
            provider_id: provider_id.map(ProviderId::from_static),
            blocks_descendant_discovery,
        }
    }

    #[test]
    fn classifies_grok_and_script_backed_agents() {
        let providers = builtin_providers();
        assert_eq!(
            classify_agent_process(&providers, OsStr::new("grok"), &[]),
            Some(ProviderId::from_static("grok"))
        );
        assert_eq!(
            classify_agent_process(
                &providers,
                OsStr::new("bun"),
                &[
                    OsString::from("bun"),
                    OsString::from("/tmp/node_modules/@oh-my-pi/pi-coding-agent/dist/cli.js"),
                ],
            ),
            Some(ProviderId::from_static("omp"))
        );
    }

    #[test]
    fn nearest_agent_wins_and_nested_yttt_is_a_boundary() {
        let processes = vec![
            record(1, None, None, false),
            record(2, Some(1), None, false),
            record(3, Some(2), Some("grok"), false),
            record(4, Some(1), None, true),
            record(5, Some(4), Some("omp"), false),
        ];
        let detected = detect_agent_processes_by_root(&[1, 4], &processes);
        assert_eq!(
            detected.get(&1),
            Some(&DetectedAgentProcess {
                pid: 3,
                provider_id: ProviderId::from_static("grok"),
            })
        );
        assert!(!detected.contains_key(&4));
    }
    #[cfg(unix)]
    #[test]
    fn system_scan_detects_a_grok_executable() {
        use std::{os::unix::fs::symlink, process::Command, thread};

        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("grok");
        symlink("/bin/sleep", &executable).unwrap();
        let mut child = Command::new(&executable).arg("5").spawn().unwrap();
        let session_id = TerminalSessionId::new("grok-scan");
        let roots = vec![(session_id.clone(), child.id())];
        let mut scanner = AgentProcessScanner::new();

        let mut detected = None;
        for _ in 0..50 {
            detected = scanner.scan(&roots).remove(&session_id);
            if detected.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(
            detected,
            Some(DetectedAgentProcess {
                pid: roots[0].1,
                provider_id: ProviderId::from_static("grok"),
            })
        );
    }

    #[cfg(unix)]
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn monitor_ends_a_manual_grok_run_while_the_shell_stays_alive() {
        use std::os::unix::fs::symlink;

        use yttt_core::model::ids::ProjectId;
        use yttt_protocol::{
            ProjectRelativePath,
            terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalSpawnSpec},
        };

        let temp = tempfile::tempdir().unwrap();
        let executable = temp.path().join("grok");
        symlink("/bin/sleep", &executable).unwrap();
        let runtime = HostRuntime::new();
        let agent_hooks = HostAgentHookRuntime::start(7).unwrap();
        let session_id = TerminalSessionId::new("manual-grok");
        let mut spec = TerminalSpawnSpec {
            session_id,
            project_id: ProjectId::new("project"),
            cwd: ProjectRelativePath::root(),
            execution: TerminalExecutionSpec::Shell {
                program: "/bin/sh".to_string(),
                args: vec!["-i".to_string()],
                initial_command: Some(format!("{} 30", executable.display())),
            },
            geometry: TerminalGeometry {
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
            },
            geometry_epoch: 1,
            query_palette: Vec::new(),
            palette_revision: 1,
            scrollback_limit: 100,
            environment: Vec::new(),
            removed_environment: Vec::new(),
        };
        agent_hooks.secure_terminal_environment(&mut spec);
        let terminal = runtime.spawn(spec).unwrap();
        let mut updates = agent_hooks.subscribe();
        let (stop_tx, stop_rx) = watch::channel(false);
        let monitor = tokio::spawn(run(runtime, agent_hooks, stop_rx));

        let started = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let update = updates.recv().await.unwrap();
                if update.snapshot.provider_id.as_str() == "grok"
                    && update.snapshot.process_state == yttt_agent_core::AgentProcessState::Running
                {
                    return update;
                }
            }
        })
        .await
        .expect("manual Grok process was not detected");
        terminal.input(vec![0x03]).unwrap();
        let exited = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let update = updates.recv().await.unwrap();
                if update.snapshot.instance_id == started.snapshot.instance_id
                    && update.snapshot.process_state == yttt_agent_core::AgentProcessState::Exited
                {
                    return update;
                }
            }
        })
        .await
        .expect("manual Grok process exit was not detected");

        assert!(exited.sequence > started.sequence);
        assert!(!terminal.is_exited());
        let _ = stop_tx.send(true);
        monitor.await.unwrap();
        terminal.terminate().unwrap();
    }
}
