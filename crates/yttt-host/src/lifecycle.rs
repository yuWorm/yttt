use std::{
    collections::BTreeSet,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
};

use parking_lot::Mutex;
use tokio::sync::{Notify, watch};
use yttt_protocol::{HostBlocker, HostLifecycleState};

use crate::{project::HostProjectRuntime, runtime::HostRuntime, ssh_runtime::HostSshRuntime};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StopMode {
    Running,
    Draining,
    ForceStopping,
}

#[derive(Clone, Copy, Debug)]
struct LifecycleState {
    mode: StopMode,
    pending_resources: usize,
}

pub(crate) struct HostLifecycle {
    clients: AtomicUsize,
    state: Mutex<LifecycleState>,
    changed: Notify,
    stop_tx: watch::Sender<bool>,
}

pub(crate) struct ResourceAdmission {
    lifecycle: Arc<HostLifecycle>,
}

impl Drop for ResourceAdmission {
    fn drop(&mut self) {
        let mut state = self.lifecycle.state.lock();
        state.pending_resources = state.pending_resources.saturating_sub(1);
        drop(state);
        self.lifecycle.changed.notify_waiters();
    }
}

impl HostLifecycle {
    pub(crate) fn new(stop_tx: watch::Sender<bool>) -> Self {
        Self {
            clients: AtomicUsize::new(0),
            state: Mutex::new(LifecycleState {
                mode: StopMode::Running,
                pending_resources: 0,
            }),
            changed: Notify::new(),
            stop_tx,
        }
    }

    pub(crate) fn client_connected(&self) {
        self.clients.fetch_add(1, Ordering::AcqRel);
        self.changed.notify_waiters();
    }

    pub(crate) fn client_disconnected(&self) {
        self.clients.fetch_sub(1, Ordering::AcqRel);
        self.changed.notify_waiters();
    }

    pub(crate) fn client_count(&self) -> usize {
        self.clients.load(Ordering::Acquire)
    }

    pub(crate) fn state(&self) -> HostLifecycleState {
        match self.state.lock().mode {
            StopMode::Running => HostLifecycleState::Running,
            StopMode::Draining => HostLifecycleState::Draining,
            StopMode::ForceStopping => HostLifecycleState::ForceStopping,
        }
    }

    pub(crate) fn blockers(
        &self,
        runtime: &HostRuntime,
        ssh_connections: Vec<String>,
        projects: Vec<yttt_core::model::ids::ProjectId>,
    ) -> Vec<HostBlocker> {
        let state = *self.state.lock();
        collect_blockers(
            runtime,
            ssh_connections,
            projects,
            state.pending_resources,
            0,
        )
    }

    pub(crate) fn resource_changed(&self) {
        self.changed.notify_waiters();
    }

    pub(crate) fn admit_resource(self: &Arc<Self>) -> Option<ResourceAdmission> {
        let mut state = self.state.lock();
        if state.mode != StopMode::Running {
            return None;
        }
        state.pending_resources = state.pending_resources.saturating_add(1);
        Some(ResourceAdmission {
            lifecycle: self.clone(),
        })
    }

    pub(crate) fn begin_drain(&self) {
        self.state.lock().mode = StopMode::Draining;
        self.changed.notify_waiters();
    }

    pub(crate) fn force_stop(&self) {
        self.state.lock().mode = StopMode::ForceStopping;
        self.changed.notify_waiters();
    }

    pub(crate) fn stop_if_idle(
        &self,
        runtime: &HostRuntime,
        ssh_connections: Vec<String>,
        projects: Vec<yttt_core::model::ids::ProjectId>,
    ) -> Result<(), Vec<HostBlocker>> {
        let mut state = self.state.lock();
        if state.mode != StopMode::Running {
            return Ok(());
        }
        let blockers = collect_blockers(
            runtime,
            ssh_connections,
            projects,
            state.pending_resources,
            0,
        );
        if blockers.is_empty() {
            state.mode = StopMode::Draining;
            let _ = self.stop_tx.send(true);
            Ok(())
        } else {
            Err(blockers)
        }
    }

    pub(crate) async fn run(
        self: Arc<Self>,
        runtime: Arc<HostRuntime>,
        ssh: Arc<HostSshRuntime>,
        projects: Arc<HostProjectRuntime>,
    ) {
        const IDLE_EXIT_DELAY: std::time::Duration = std::time::Duration::from_secs(30);
        loop {
            let notified = self.changed.notified();
            let state = *self.state.lock();
            if state.mode == StopMode::ForceStopping {
                runtime.terminate_all();
                let _ = self.stop_tx.send(true);
                return;
            }
            let resources = collect_blockers(
                &runtime,
                ssh.connections(),
                projects.projects(),
                state.pending_resources,
                0,
            );
            if state.mode == StopMode::Draining && resources.is_empty() {
                let _ = self.stop_tx.send(true);
                return;
            }
            if state.mode == StopMode::Running
                && self.clients.load(Ordering::Acquire) == 0
                && resources.is_empty()
            {
                tokio::select! {
                    _ = tokio::time::sleep(IDLE_EXIT_DELAY) => {
                        let state = *self.state.lock();
                        if state.mode == StopMode::Running
                            && self.clients.load(Ordering::Acquire) == 0
                            && collect_blockers(
                                &runtime,
                                ssh.connections(),
                                projects.projects(),
                                state.pending_resources,
                                0,
                            )
                            .is_empty()
                        {
                            let _ = self.stop_tx.send(true);
                            return;
                        }
                    }
                    _ = notified => {}
                }
            } else {
                notified.await;
            }
        }
    }
}

fn collect_blockers(
    runtime: &HostRuntime,
    ssh_connections: Vec<String>,
    projects: Vec<yttt_core::model::ids::ProjectId>,
    pending_resources: usize,
    clients: usize,
) -> Vec<HostBlocker> {
    let mut blockers = runtime.lifecycle_blockers();
    blockers.extend(ssh_connections.into_iter().map(HostBlocker::SshConnection));
    blockers.extend(projects.into_iter().map(HostBlocker::Project));
    if pending_resources != 0 {
        blockers.push(HostBlocker::PendingResourceOperations {
            count: pending_resources.min(u32::MAX as usize) as u32,
        });
    }
    if clients != 0 {
        blockers.push(HostBlocker::ConnectedClients {
            count: clients.min(u32::MAX as usize) as u32,
        });
    }
    blockers.sort_by(|left, right| format!("{left:?}").cmp(&format!("{right:?}")));
    let mut seen = BTreeSet::new();
    blockers.retain(|blocker| seen.insert(format!("{blocker:?}")));
    blockers
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_if_idle_cannot_cross_an_admitted_resource_creation() {
        let (stop_tx, stop_rx) = watch::channel(false);
        let lifecycle = Arc::new(HostLifecycle::new(stop_tx));
        let runtime = HostRuntime::new();
        let admission = lifecycle.admit_resource().unwrap();

        let blockers = lifecycle
            .stop_if_idle(&runtime, Vec::new(), Vec::new())
            .unwrap_err();
        assert_eq!(
            blockers,
            vec![HostBlocker::PendingResourceOperations { count: 1 }]
        );
        assert!(!*stop_rx.borrow());

        drop(admission);
        lifecycle
            .stop_if_idle(&runtime, Vec::new(), Vec::new())
            .unwrap();
        assert!(*stop_rx.borrow());
    }
}
