use std::{
    collections::HashMap,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use gpui::{App, Context, Window};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
use yttt_agent_core::{AGENT_ACTIVITY_STALE_AFTER_MILLIS, AgentExitReason};

use super::{
    WorkbenchView, combine_load_messages, helpers::terminal_pane_key,
    state::terminal::AgentProcessObservation,
};
use crate::{
    config::default_layout::BuiltinAgent,
    model::ids::ProjectId,
    runtime::{
        agent::{AgentProcessRecord, classify_agent_process, detect_agent_processes_by_root},
        agent_hooks::AgentHookRequest,
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
        if self.terminal.agent_process_monitor_task.is_some() || !self.terminal.start_processes {
            return;
        }

        self.terminal.agent_process_monitor_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                let mut system = System::new();
                loop {
                    let probes = match this
                        .update_in(cx, |view, _window, cx| view.agent_process_probes(cx))
                    {
                        Ok(probes) => probes,
                        Err(_) => break,
                    };
                    let root_pids = probes
                        .iter()
                        .map(|probe| probe.root_pid)
                        .collect::<Vec<_>>();
                    let scan = cx.background_executor().spawn(async move {
                        let detected = if sysinfo::IS_SUPPORTED_SYSTEM {
                            scan_agent_processes(&mut system, &root_pids)
                        } else {
                            HashMap::new()
                        };
                        (system, detected)
                    });
                    let (refreshed_system, detected) = scan.await;
                    system = refreshed_system;
                    if this
                        .update_in(cx, |view, window, cx| {
                            view.apply_agent_process_scan(&detected, window, cx);
                            view.apply_agent_hook_requests(window, cx);
                            if view.workspace.decay_stale_agent_activity(
                                unix_timestamp_millis(),
                                AGENT_ACTIVITY_STALE_AFTER_MILLIS,
                            ) > 0
                            {
                                cx.notify();
                            }
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
    fn agent_hook_request_belongs_to_live_pane(
        &self,
        request: &AgentHookRequest,
        cx: &App,
    ) -> bool {
        let key = terminal_pane_key(
            &request.address.project_id,
            &request.address.tab_id,
            &request.address.pane_id,
        );
        let Some(pane) = self.terminal.terminal_panes.get(&key) else {
            return false;
        };
        let pane = pane.read(cx);
        hook_request_belongs_to_pane(
            request,
            pane.matches_agent_pane_address(&request.address),
            pane.generation(),
            pane.is_running(),
        )
    }

    fn apply_agent_hook_requests(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let requests = self.agent_manager.drain_hook_requests();
        let mut changed = false;
        for request in requests {
            if !self.agent_hook_request_belongs_to_live_pane(&request, cx) {
                continue;
            }
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
        let result = if snapshot.view_state() == yttt_agent_core::AgentViewState::Completed {
            self.record_agent_runtime_snapshot(address.clone(), snapshot)
        } else {
            self.workspace.clear_agent_snapshot(
                &ProjectId::new(&address.project_id),
                &address.tab_id,
                &address.pane_id,
            )
        };
        if let Err(error) = result {
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

fn hook_request_belongs_to_pane(
    request: &AgentHookRequest,
    pane_address_matches: bool,
    pane_generation: u64,
    pane_running: bool,
) -> bool {
    pane_running && pane_address_matches && pane_generation == request.generation
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
fn unix_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[cfg(test)]
mod hook_ownership_tests {
    use serde_json::Value;

    use super::*;

    fn request(generation: u64, source: BuiltinAgent) -> AgentHookRequest {
        AgentHookRequest {
            address: AgentPaneAddress::new("project", "tab", "pane"),
            generation,
            source,
            event: "event".to_string(),
            payload: Value::Null,
        }
    }

    #[test]
    fn hook_request_requires_the_live_pane_generation() {
        let request = request(7, BuiltinAgent::Codex);

        assert!(hook_request_belongs_to_pane(&request, true, 7, true));
        assert!(!hook_request_belongs_to_pane(&request, true, 8, true));
        assert!(!hook_request_belongs_to_pane(&request, false, 7, true));
        assert!(!hook_request_belongs_to_pane(&request, true, 7, false));
    }

    #[test]
    fn live_shell_pane_hook_does_not_require_process_recognition() {
        let request = request(7, BuiltinAgent::OhMyPi);

        assert!(hook_request_belongs_to_pane(&request, true, 7, true));
    }
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
