use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use parking_lot::Mutex;
use tokio::sync::{broadcast, watch};
use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};
use yttt_protocol::{
    HostBlocker, TerminalLease, TerminalPlacement,
    terminal::{
        TerminalExecutionSpec, TerminalGeometry, TerminalLeaseMode, TerminalSpawnSpec,
        TerminatedTerminal,
    },
};
use yttt_ssh::TransportService;

use crate::diagnostics::{QueueDiagnostics, QueueDiagnosticsSnapshot};
use crate::terminal::{HostTerminalEvent, HostedTerminal, HostedTerminalError};

const RUNTIME_EVENT_CAPACITY: usize = 256;
const LEASE_DURATION: Duration = Duration::from_secs(15);
const RETAINED_ATTACHMENT_QUEUE_DIAGNOSTICS: usize = 4_096;
pub const EXIT_ACK_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone)]
struct LeaseState {
    holder: ClientInstanceId,
    lease_epoch: u64,
    expires_millis: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct AttachmentStreamState {
    pub attached: bool,
    pub display_offset: u64,
}

pub struct HostRuntime {
    terminals: Mutex<HashMap<TerminalSessionId, HostedTerminal>>,
    leases: Mutex<HashMap<TerminalSessionId, LeaseState>>,
    attachments:
        Mutex<HashMap<(ClientInstanceId, TerminalSessionId), watch::Sender<AttachmentStreamState>>>,
    next_session_epoch: AtomicU64,
    next_lease_epoch: AtomicU64,
    events: broadcast::Sender<HostTerminalEvent>,
    attachment_queues: Mutex<VecDeque<Arc<QueueDiagnostics>>>,
}

impl HostRuntime {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(RUNTIME_EVENT_CAPACITY);
        Arc::new(Self {
            terminals: Mutex::new(HashMap::new()),
            leases: Mutex::new(HashMap::new()),
            attachments: Mutex::new(HashMap::new()),
            next_session_epoch: AtomicU64::new(1),
            next_lease_epoch: AtomicU64::new(1),
            events,
            attachment_queues: Mutex::new(VecDeque::new()),
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
        if let Some(existing) = self.terminals.lock().get(&spec.session_id) {
            let expected_fingerprint = existing.spec().address_fingerprint();
            let actual_fingerprint = spec.address_fingerprint();
            return if expected_fingerprint == actual_fingerprint {
                Err(HostRuntimeError::AlreadyExists(spec.session_id))
            } else {
                Err(HostRuntimeError::AddressConflict {
                    session_id: spec.session_id,
                    expected_fingerprint,
                    actual_fingerprint,
                })
            };
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
                update: terminal.shared_update(
                    yttt_protocol::terminal::TerminalStreamUpdate::Snapshot(viewport),
                ),
            });
        }
        Ok(terminal)
    }

    pub fn lifecycle_blockers(&self) -> Vec<HostBlocker> {
        self.terminals
            .lock()
            .values()
            .map(|terminal| {
                if terminal.is_exited() {
                    let checkpoint = terminal.checkpoint();
                    HostBlocker::ExitedTerminalAwaitingAck {
                        session_id: terminal.session_id().clone(),
                        session_epoch: terminal.session_epoch(),
                        final_sequence: checkpoint
                            .map_or(0, |checkpoint| checkpoint.viewport.sequence),
                    }
                } else {
                    HostBlocker::RunningTerminal(terminal.session_id().clone())
                }
            })
            .collect()
    }

    pub fn terminal(&self, session_id: &TerminalSessionId) -> Option<HostedTerminal> {
        self.terminals.lock().get(session_id).cloned()
    }

    pub fn session_count(&self) -> usize {
        self.terminals.lock().len()
    }

    pub fn attachment_count(&self) -> usize {
        self.attachments.lock().len()
    }

    pub fn terminal_diagnostics(
        &self,
    ) -> Vec<crate::diagnostics::TerminalPipelineDiagnosticsSnapshot> {
        let terminals = self.terminals.lock();
        let mut snapshots = terminals
            .values()
            .map(HostedTerminal::diagnostics)
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| left.session_id.cmp(&right.session_id));
        snapshots
    }

    pub(crate) fn new_attachment_queue_diagnostics(&self) -> Arc<QueueDiagnostics> {
        let diagnostics = QueueDiagnostics::new(
            "attachment_output_bytes",
            crate::terminal_data::MAX_ATTACHMENT_OUTPUT_BYTES,
        );
        let mut queues = self.attachment_queues.lock();
        if queues.len() == RETAINED_ATTACHMENT_QUEUE_DIAGNOSTICS {
            queues.pop_front();
        }
        queues.push_back(diagnostics.clone());
        diagnostics
    }

    pub fn attachment_queue_diagnostics(&self) -> Vec<QueueDiagnosticsSnapshot> {
        self.attachment_queues
            .lock()
            .iter()
            .map(|queue| queue.snapshot())
            .collect()
    }
    pub fn lease(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> Option<TerminalLease> {
        let now = now_millis();
        let mut leases = self.leases.lock();
        if leases
            .get(session_id)
            .is_some_and(|lease| lease.expires_millis <= now)
        {
            leases.remove(session_id);
            return None;
        }
        leases
            .get(session_id)
            .filter(|lease| lease.holder == *client_id)
            .map(|lease| wire_lease(session_id, lease))
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
        let previous_owner = leases
            .get(session_id)
            .filter(|current| current.expires_millis > now && current.holder != *client_id)
            .map(|current| current.holder.clone());
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
        drop(leases);
        if let Some(previous_owner) = previous_owner {
            let _ = self.events.send(HostTerminalEvent::LeaseRevoked {
                session_id: session_id.clone(),
                previous_owner,
            });
        }
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
        let affected = {
            let mut attachments = self.attachments.lock();
            let affected = attachments
                .keys()
                .filter(|(owner, _)| owner == client_id)
                .map(|(_, session_id)| session_id.clone())
                .collect::<HashSet<_>>();
            attachments.retain(|(owner, _), attached| {
                if owner == client_id {
                    let _ = attached.send(AttachmentStreamState {
                        attached: false,
                        display_offset: 0,
                    });
                    false
                } else {
                    true
                }
            });
            affected
        };
        for session_id in affected {
            self.refresh_terminal_subscriber_count(&session_id);
        }
    }

    pub(crate) fn register_attachment(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) {
        let key = (client_id.clone(), session_id.clone());
        let mut attachments = self.attachments.lock();
        if let Some(attached) = attachments.get(&key) {
            let _ = attached.send(AttachmentStreamState {
                attached: true,
                display_offset: 0,
            });
        } else {
            let (attached, _) = watch::channel(AttachmentStreamState {
                attached: true,
                display_offset: 0,
            });
            attachments.insert(key, attached);
        }
        drop(attachments);
        self.refresh_terminal_subscriber_count(session_id);
    }

    pub(crate) fn attachment_receiver(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> Option<watch::Receiver<AttachmentStreamState>> {
        self.attachments
            .lock()
            .get(&(client_id.clone(), session_id.clone()))
            .map(watch::Sender::subscribe)
    }

    pub(crate) fn release_attachment(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) {
        if let Some(attached) = self
            .attachments
            .lock()
            .remove(&(client_id.clone(), session_id.clone()))
        {
            let _ = attached.send(AttachmentStreamState {
                attached: false,
                display_offset: 0,
            });
        }
        self.refresh_terminal_subscriber_count(session_id);
    }

    pub(crate) fn update_attachment_display_offset(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
        display_offset: u64,
    ) {
        if let Some(attached) = self
            .attachments
            .lock()
            .get(&(client_id.clone(), session_id.clone()))
        {
            let _ = attached.send(AttachmentStreamState {
                attached: true,
                display_offset,
            });
        }
    }

    fn refresh_terminal_subscriber_count(&self, session_id: &TerminalSessionId) {
        let count = self
            .attachments
            .lock()
            .keys()
            .filter(|(_, attached_session_id)| attached_session_id == session_id)
            .count();
        if let Some(terminal) = self.terminal(session_id) {
            terminal.set_subscriber_count(count);
        }
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
                    spawn_fingerprint: spec.address_fingerprint(),
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

    pub fn terminate(
        &self,
        session_id: &TerminalSessionId,
    ) -> Result<TerminatedTerminal, HostRuntimeError> {
        let terminal = self
            .terminal(session_id)
            .ok_or_else(|| HostRuntimeError::NotFound(session_id.clone()))?;
        terminal.terminate()?;
        let final_sequence = terminal
            .checkpoint()
            .map_or(0, |checkpoint| checkpoint.viewport.sequence);
        Ok(TerminatedTerminal {
            session_epoch: terminal.session_epoch(),
            final_sequence,
        })
    }

    pub fn acknowledge_terminal_exit(
        &self,
        session_id: &TerminalSessionId,
        session_epoch: u64,
        final_sequence: u64,
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
        let expected_final_sequence = terminal
            .checkpoint()
            .map_or(0, |checkpoint| checkpoint.viewport.sequence);
        if expected_final_sequence != final_sequence {
            return Err(HostRuntimeError::StaleFinalSequence {
                session_id: session_id.clone(),
                expected: expected_final_sequence,
                actual: final_sequence,
            });
        }
        terminals.remove(session_id);
        self.leases.lock().remove(session_id);
        self.attachments
            .lock()
            .retain(|(_, attached_session_id), attached| {
                if attached_session_id == session_id {
                    let _ = attached.send(AttachmentStreamState {
                        attached: false,
                        display_offset: 0,
                    });
                    false
                } else {
                    true
                }
            });
        Ok(())
    }

    pub fn reap_expired_exits(&self, now_millis: u64) -> usize {
        let cutoff = now_millis.saturating_sub(EXIT_ACK_TTL.as_millis() as u64);
        let mut terminals = self.terminals.lock();
        let before = terminals.len();
        terminals.retain(|_, terminal| {
            terminal
                .exited_at_millis()
                .is_none_or(|exited_at| exited_at > cutoff)
        });
        let removed = before.saturating_sub(terminals.len());
        if removed != 0 {
            self.leases
                .lock()
                .retain(|session_id, _| terminals.contains_key(session_id));
            self.attachments.lock().retain(|(_, session_id), attached| {
                if terminals.contains_key(session_id) {
                    true
                } else {
                    let _ = attached.send(AttachmentStreamState {
                        attached: false,
                        display_offset: 0,
                    });
                    false
                }
            });
        }
        removed
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
    #[error(
        "terminal session {session_id} address conflicts: expected fingerprint {expected_fingerprint:#x}, received {actual_fingerprint:#x}"
    )]
    AddressConflict {
        session_id: TerminalSessionId,
        expected_fingerprint: u64,
        actual_fingerprint: u64,
    },
    #[error("terminal session {0} was not found")]
    NotFound(TerminalSessionId),
    #[error("terminal session {session_id} is controlled by client {holder}")]
    LeaseConflict {
        session_id: TerminalSessionId,
        holder: ClientInstanceId,
    },
    #[error("terminal session {0} requires an active ownership lease")]
    LeaseRequired(TerminalSessionId),
    #[error("host epoch is stale: expected {expected}, received {actual}")]
    StaleHostEpoch { expected: u64, actual: u64 },
    #[error("terminal session {session_id} epoch is stale: expected {expected}, received {actual}")]
    StaleSessionEpoch {
        session_id: TerminalSessionId,
        expected: u64,
        actual: u64,
    },
    #[error(
        "terminal session {session_id} final sequence is stale: expected {expected}, received {actual}"
    )]
    StaleFinalSequence {
        session_id: TerminalSessionId,
        expected: u64,
        actual: u64,
    },
    #[error(
        "terminal session {session_id} lease epoch is stale: expected {expected}, received {actual}"
    )]
    StaleLeaseEpoch {
        session_id: TerminalSessionId,
        expected: u64,
        actual: u64,
    },
    #[error(
        "terminal session {session_id} client sequence is stale: last accepted {last_accepted}, received {received}"
    )]
    StaleClientSequence {
        session_id: TerminalSessionId,
        last_accepted: u64,
        received: u64,
    },
    #[error(
        "terminal session {session_id} search generation is stale: current {current}, received {received}"
    )]
    StaleSearchGeneration {
        session_id: TerminalSessionId,
        received: u64,
        current: u64,
    },
    #[error("terminal session {0} is still running")]
    TerminalStillRunning(TerminalSessionId),
    #[error(transparent)]
    Terminal(#[from] HostedTerminalError),
}
