use std::{
    collections::HashMap,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use tokio::sync::broadcast;
use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};
use yttt_protocol::{
    TerminalLease, TerminalPlacement,
    terminal::{TerminalExecutionSpec, TerminalGeometry, TerminalLeaseMode, TerminalSpawnSpec},
};
use yttt_ssh::TransportService;

use crate::terminal::{HostTerminalEvent, HostedTerminal, HostedTerminalError};

const RUNTIME_EVENT_CAPACITY: usize = 256;
const LEASE_DURATION: Duration = Duration::from_secs(15);

#[derive(Clone)]
struct LeaseState {
    holder: ClientInstanceId,
    lease_epoch: u64,
    expires_millis: u64,
}

pub struct HostRuntime {
    terminals: Mutex<HashMap<TerminalSessionId, HostedTerminal>>,
    leases: Mutex<HashMap<TerminalSessionId, LeaseState>>,
    next_session_epoch: AtomicU64,
    next_lease_epoch: AtomicU64,
    events: broadcast::Sender<HostTerminalEvent>,
}

impl HostRuntime {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(RUNTIME_EVENT_CAPACITY);
        Arc::new(Self {
            terminals: Mutex::new(HashMap::new()),
            leases: Mutex::new(HashMap::new()),
            next_session_epoch: AtomicU64::new(1),
            next_lease_epoch: AtomicU64::new(1),
            events,
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HostTerminalEvent> {
        self.events.subscribe()
    }

    pub fn spawn(
        self: &Arc<Self>,
        spec: TerminalSpawnSpec,
    ) -> Result<HostedTerminal, HostRuntimeError> {
        self.spawn_with_transport(spec, None)
    }

    pub fn spawn_with_transport(
        self: &Arc<Self>,
        spec: TerminalSpawnSpec,
        ssh: Option<TransportService>,
    ) -> Result<HostedTerminal, HostRuntimeError> {
        if self.terminals.lock().contains_key(&spec.session_id) {
            return Err(HostRuntimeError::AlreadyExists(spec.session_id));
        }
        let session_epoch = self.next_session_epoch.fetch_add(1, Ordering::Relaxed);
        let terminal = match &spec.execution {
            TerminalExecutionSpec::Ssh { .. } => HostedTerminal::spawn_remote(
                spec.clone(),
                session_epoch,
                ssh.as_ref()
                    .ok_or(HostedTerminalError::UnsupportedExecution)?,
            )?,
            TerminalExecutionSpec::Shell { .. } | TerminalExecutionSpec::Command { .. } => {
                HostedTerminal::spawn(spec.clone(), session_epoch)?
            }
        };
        self.terminals
            .lock()
            .insert(spec.session_id.clone(), terminal.clone());
        let mut events = terminal.subscribe();
        let runtime_events = self.events.clone();
        tokio::spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        let _ = runtime_events.send(event);
                    }
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        if let Some(viewport) = terminal.latest_viewport() {
            let _ = self.events.send(HostTerminalEvent::Update {
                session_id: spec.session_id,
                update: yttt_protocol::terminal::TerminalStreamUpdate::Snapshot(viewport),
            });
        }
        Ok(terminal)
    }

    pub fn terminal(&self, session_id: &TerminalSessionId) -> Option<HostedTerminal> {
        self.terminals.lock().get(session_id).cloned()
    }

    pub fn acquire_lease(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
        mode: TerminalLeaseMode,
    ) -> Result<TerminalLease, HostRuntimeError> {
        if self.terminal(session_id).is_none() {
            return Err(HostRuntimeError::NotFound(session_id.clone()));
        }
        if mode == TerminalLeaseMode::Observer {
            return Ok(TerminalLease {
                session_id: session_id.clone(),
                owner: client_id.clone(),
                mode,
                lease_epoch: self.next_lease_epoch.fetch_add(1, Ordering::Relaxed),
            });
        }
        let now = now_millis();
        let mut leases = self.leases.lock();
        if let Some(current) = leases.get_mut(session_id)
            && current.expires_millis > now
            && current.holder != *client_id
        {
            return Err(HostRuntimeError::LeaseConflict {
                session_id: session_id.clone(),
                holder: current.holder.clone(),
            });
        }
        if let Some(current) = leases.get_mut(session_id)
            && current.expires_millis > now
            && current.holder == *client_id
        {
            current.expires_millis = lease_expiry_millis();
            return Ok(wire_lease(session_id, current));
        }
        let lease = LeaseState {
            holder: client_id.clone(),
            lease_epoch: self.next_lease_epoch.fetch_add(1, Ordering::Relaxed),
            expires_millis: lease_expiry_millis(),
        };
        let wire = wire_lease(session_id, &lease);
        leases.insert(session_id.clone(), lease);
        Ok(wire)
    }

    pub fn validate_lease(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> Result<TerminalLease, HostRuntimeError> {
        let now = now_millis();
        let mut leases = self.leases.lock();
        let Some(lease) = leases.get_mut(session_id) else {
            return Err(HostRuntimeError::LeaseRequired(session_id.clone()));
        };
        if lease.expires_millis <= now {
            leases.remove(session_id);
            return Err(HostRuntimeError::LeaseRequired(session_id.clone()));
        }
        if lease.holder != *client_id {
            return Err(HostRuntimeError::LeaseConflict {
                session_id: session_id.clone(),
                holder: lease.holder.clone(),
            });
        }
        lease.expires_millis = lease_expiry_millis();
        Ok(wire_lease(session_id, lease))
    }

    pub fn release_lease(&self, session_id: &TerminalSessionId, client_id: &ClientInstanceId) {
        let mut leases = self.leases.lock();
        if leases
            .get(session_id)
            .is_some_and(|lease| lease.holder == *client_id)
        {
            leases.remove(session_id);
        }
    }

    pub fn release_client(&self, client_id: &ClientInstanceId) {
        self.leases
            .lock()
            .retain(|_, lease| lease.holder != *client_id);
    }

    pub fn placements(&self) -> Vec<TerminalPlacement> {
        let terminals = self.terminals.lock();
        let now = now_millis();
        let mut leases = self.leases.lock();
        leases.retain(|_, lease| lease.expires_millis > now);
        let mut placements = terminals
            .values()
            .map(|terminal| {
                let viewport = terminal.latest_viewport();
                let geometry = viewport
                    .as_ref()
                    .map(|viewport| viewport.geometry)
                    .unwrap_or(TerminalGeometry {
                        cols: 0,
                        rows: 0,
                        cell_width: 0,
                        cell_height: 0,
                    });
                let spec = terminal.spec();
                TerminalPlacement {
                    session_id: spec.session_id.clone(),
                    session_epoch: terminal.session_epoch(),
                    project_id: spec.project_id.clone(),
                    tab_id: spec.tab_id.clone(),
                    pane_id: spec.pane_id.clone(),
                    geometry,
                    last_sequence: viewport.as_ref().map_or(0, |viewport| viewport.sequence),
                    owner: leases
                        .get(&spec.session_id)
                        .map(|lease| lease.holder.clone()),
                    viewport,
                }
            })
            .collect::<Vec<_>>();
        placements.sort_by(|left, right| left.session_id.as_str().cmp(right.session_id.as_str()));
        placements
    }

    pub fn terminate(&self, session_id: &TerminalSessionId) -> Result<(), HostRuntimeError> {
        let terminal = self
            .terminal(session_id)
            .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
        terminal.terminate()?;
        Ok(())
    }

    pub fn acknowledge_terminal_exit(
        &self,
        session_id: &TerminalSessionId,
        session_epoch: u64,
    ) -> Result<(), HostRuntimeError> {
        let mut terminals = self.terminals.lock();
        let Some(terminal) = terminals.get(session_id) else {
            return Err(HostRuntimeError::NotFound(session_id.clone()));
        };
        if terminal.session_epoch() != session_epoch {
            return Err(HostRuntimeError::StaleSessionEpoch {
                session_id: session_id.clone(),
                expected: terminal.session_epoch(),
                actual: session_epoch,
            });
        }
        if !terminal.is_exited() {
            return Err(HostRuntimeError::TerminalStillRunning(session_id.clone()));
        }
        terminals.remove(session_id);
        self.leases.lock().remove(session_id);
        Ok(())
    }

    pub fn terminate_all(&self) -> usize {
        let terminals = self.terminals.lock().values().cloned().collect::<Vec<_>>();
        for terminal in &terminals {
            let _ = terminal.terminate();
        }
        terminals.len()
    }
}

fn wire_lease(session_id: &TerminalSessionId, lease: &LeaseState) -> TerminalLease {
    TerminalLease {
        session_id: session_id.clone(),
        owner: lease.holder.clone(),
        mode: TerminalLeaseMode::Interactive,
        lease_epoch: lease.lease_epoch,
    }
}

fn lease_expiry_millis() -> u64 {
    now_millis().saturating_add(LEASE_DURATION.as_millis() as u64)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[derive(Debug, thiserror::Error)]
pub enum HostRuntimeError {
    #[error("terminal session {0} already exists")]
    AlreadyExists(TerminalSessionId),
    #[error("terminal session {0} was not found")]
    NotFound(TerminalSessionId),
    #[error("terminal session {session_id} is controlled by client {holder}")]
    LeaseConflict {
        session_id: TerminalSessionId,
        holder: ClientInstanceId,
    },
    #[error("terminal session {0} requires an active ownership lease")]
    LeaseRequired(TerminalSessionId),
    #[error("terminal session {session_id} epoch is stale: expected {expected}, received {actual}")]
    StaleSessionEpoch {
        session_id: TerminalSessionId,
        expected: u64,
        actual: u64,
    },
    #[error("terminal session {0} is still running")]
    TerminalStillRunning(TerminalSessionId),
    #[error(transparent)]
    Terminal(#[from] HostedTerminalError),
}
