use std::{collections::HashMap, time::Duration};

use gpui::{App, Context, Window};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use yttt_agent_core::AgentExitReason;

use super::{WorkbenchView, combine_load_messages, state::terminal::AgentProcessObservation};
use crate::{
    config::default_layout::BuiltinAgent,
    model::ids::ProjectId,
    runtime::{
        agent::{AgentProcessRecord, classify_agent_process, detect_agent_processes_by_root},
        agent_manager::AgentPaneAddress,
    },
};

const AGENT_PROCESS_SCAN_INTERVAL: Duration = Duration::from_millis(500);
const AGENT_PROCESS_MISSED_SAMPLES_BEFORE_EXIT: u8 = 2;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AgentProcessProbe {
    address: AgentPaneAddress,
    generation: u64,
    root_pid: u32,
}

impl WorkbenchView {
    pub(super) fn sync_agent_process_monitoring(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal.agent_process_monitor_task.is_some()
            || !self.terminal.start_processes
            || !sysinfo::IS_SUPPORTED_SYSTEM
        {
            return;
        }

        self.terminal.agent_process_monitor_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                let mut system = System::new();
                loop {
                    let probes = match this.update_in(cx, |view, window, cx| {
                        view.apply_agent_hook_requests(window, cx);
                        view.agent_process_probes(cx)
                    }) {
                        Ok(probes) => probes,
                        Err(_) => break,
                    };
                    let root_pids = probes
                        .iter()
                        .map(|probe| probe.root_pid)
                        .collect::<Vec<_>>();
                    let scan = cx.background_executor().spawn(async move {
                        let detected = scan_agent_processes(&mut system, &root_pids);
                        (system, detected)
                    });
                    let (refreshed_system, detected) = scan.await;
                    system = refreshed_system;
                    if this
                        .update_in(cx, |view, window, cx| {
                            view.apply_agent_process_scan(&detected, window, cx);
                        })
                        .is_err()
                    {
                        break;
                    }
                    cx.background_executor()
                        .timer(AGENT_PROCESS_SCAN_INTERVAL)
                        .await;
                }
            }));
    }

    fn agent_process_probes(&self, cx: &App) -> Vec<AgentProcessProbe> {
        self.terminal
            .terminal_panes
            .values()
            .filter_map(|pane| {
                let pane = pane.read(cx);
                if !pane.is_running() || pane.agent_instance_id().is_some() {
                    return None;
                }
                Some(AgentProcessProbe {
                    address: pane.agent_pane_address(),
                    generation: pane.generation(),
                    root_pid: pane.local_process_id()?,
                })
            })
            .collect()
    }
    fn apply_agent_hook_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let requests = self.agent_manager.drain_hook_requests();
        let mut changed = false;
        for request in requests {
            match self.agent_manager.ingest_hook_request(request) {
                Ok(Some((address, snapshot))) => {
                    if let Err(error) =
                        self.record_agent_event_snapshot(address, snapshot, window, cx)
                    {
                        self.load_error = Some(error.to_string());
                    } else {
                        changed = true;
                    }
                }
                Ok(None) => {}
                Err(error) => self.load_error = Some(error.to_string()),
            }
        }
        if changed {
            cx.notify();
        }
    }

    fn apply_agent_process_scan(
        &mut self,
        detected_by_root: &HashMap<u32, BuiltinAgent>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let probes = self.agent_process_probes(cx);
        let active_generations = probes
            .iter()
            .map(|probe| (probe.address.clone(), probe.generation))
            .collect::<HashMap<_, _>>();
        let vanished = self
            .terminal
            .agent_process_observations
            .iter()
            .filter(|(address, observation)| {
                active_generations
                    .get(*address)
                    .is_none_or(|generation| *generation != observation.generation)
            })
            .map(|(address, observation)| (address.clone(), observation.generation))
            .collect::<Vec<_>>();
        let mut changed = false;
        for (address, generation) in vanished {
            self.terminal.agent_process_observations.remove(&address);
            changed |= self.finish_detected_agent(
                &address,
                generation,
                AgentExitReason::KilledByUser,
                window,
                cx,
            );
        }

        for probe in &probes {
            let detected = detected_by_root.get(&probe.root_pid).copied();
            let previous = self
                .terminal
                .agent_process_observations
                .get(&probe.address)
                .copied();
            match (previous, detected) {
                (Some(previous), Some(agent))
                    if previous.generation == probe.generation && previous.agent == agent =>
                {
                    if previous.missed_samples != 0 {
                        self.terminal.agent_process_observations.insert(
                            probe.address.clone(),
                            AgentProcessObservation {
                                missed_samples: 0,
                                ..previous
                            },
                        );
                    }
                }
                (Some(previous), Some(agent)) => {
                    self.terminal
                        .agent_process_observations
                        .remove(&probe.address);
                    changed |= self.finish_detected_agent(
                        &probe.address,
                        previous.generation,
                        if previous.generation == probe.generation {
                            AgentExitReason::Completed
                        } else {
                            AgentExitReason::KilledByUser
                        },
                        window,
                        cx,
                    );
                    changed |= self.start_detected_agent(probe, agent);
                }
                (None, Some(agent)) => {
                    changed |= self.start_detected_agent(probe, agent);
                }
                (Some(previous), None) if previous.generation == probe.generation => {
                    let missed_samples = previous.missed_samples.saturating_add(1);
                    if missed_samples >= AGENT_PROCESS_MISSED_SAMPLES_BEFORE_EXIT {
                        self.terminal
                            .agent_process_observations
                            .remove(&probe.address);
                        changed |= self.finish_detected_agent(
                            &probe.address,
                            probe.generation,
                            AgentExitReason::Completed,
                            window,
                            cx,
                        );
                    } else {
                        self.terminal.agent_process_observations.insert(
                            probe.address.clone(),
                            AgentProcessObservation {
                                missed_samples,
                                ..previous
                            },
                        );
                    }
                }
                (Some(previous), None) => {
                    self.terminal
                        .agent_process_observations
                        .remove(&probe.address);
                    changed |= self.finish_detected_agent(
                        &probe.address,
                        previous.generation,
                        AgentExitReason::KilledByUser,
                        window,
                        cx,
                    );
                }
                (None, None) => {}
            }
        }

        if changed {
            cx.notify();
        }
    }

    fn start_detected_agent(&mut self, probe: &AgentProcessProbe, agent: BuiltinAgent) -> bool {
        self.terminal.agent_process_observations.insert(
            probe.address.clone(),
            AgentProcessObservation {
                agent,
                generation: probe.generation,
                missed_samples: 0,
            },
        );
        let Some(snapshot) = self.agent_manager.detected_process_started(
            probe.address.clone(),
            agent,
            probe.generation,
        ) else {
            return false;
        };
        if let Err(error) = self.record_agent_runtime_snapshot(probe.address.clone(), snapshot) {
            self.load_error = Some(error.to_string());
        }
        true
    }

    pub(super) fn finish_detected_agent(
        &mut self,
        address: &AgentPaneAddress,
        generation: u64,
        reason: AgentExitReason,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(snapshot) = self
            .agent_manager
            .detected_process_exited(address, generation, reason)
        else {
            return false;
        };
        let notification = (reason != AgentExitReason::KilledByUser)
            .then(|| self.agent_transition_notification(address, &snapshot))
            .flatten();
        self.update_terminal_agent_title(address, snapshot.provider_id.as_str(), None, cx);
        if let Err(error) = self.workspace.clear_agent_snapshot(
            &ProjectId::new(&address.project_id),
            &address.tab_id,
            &address.pane_id,
        ) {
            self.load_error = Some(error.to_string());
        }
        if let Some(error) = self.agent_manager.take_error() {
            self.load_error = combine_load_messages(self.load_error.take(), Some(error));
        }
        if let Some(notification) = notification {
            self.present_notification(notification, window, cx);
        }
        true
    }
}

fn scan_agent_processes(system: &mut System, root_pids: &[u32]) -> HashMap<u32, BuiltinAgent> {
    if root_pids.is_empty() {
        return HashMap::new();
    }

    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        ProcessRefreshKind::new()
            .with_cmd(UpdateKind::Always)
            .with_exe(UpdateKind::Always),
    );
    let processes = system
        .processes()
        .iter()
        .map(|(pid, process)| AgentProcessRecord {
            pid: pid.as_u32(),
            parent_pid: process.parent().map(|parent| parent.as_u32()),
            agent: classify_agent_process(process.name(), process.cmd()),
        })
        .collect::<Vec<_>>();
    detect_agent_processes_by_root(root_pids, &processes)
}

#[cfg(all(test, unix))]
mod tests {
    use std::{os::unix::fs::symlink, process::Command, thread, time::Duration};

    use tempfile::tempdir;

    use super::*;

    #[test]
    fn system_scan_detects_a_known_agent_executable() {
        let temp = tempdir().unwrap();
        let executable = temp.path().join("codex");
        symlink("/bin/sleep", &executable).unwrap();
        let mut child = Command::new(&executable).arg("5").spawn().unwrap();
        let root_pid = child.id();
        let mut system = System::new();

        let mut detected = None;
        for _ in 0..50 {
            detected = scan_agent_processes(&mut system, &[root_pid])
                .get(&root_pid)
                .copied();
            if detected.is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }

        let _ = child.kill();
        let _ = child.wait();
        assert_eq!(detected, Some(BuiltinAgent::Codex));
    }
}
