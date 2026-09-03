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
    HostBlocker, TerminalControlDeniedReason, TerminalLease, TerminalPlacement,
    terminal::{
        TerminalExecutionSpec, TerminalLeaseMode, TerminalProcessState, TerminalSpawnSpec,
        TerminatedTerminal,
    },
};
use yttt_ssh::TransportService;

use crate::diagnostics::{QueueDiagnostics, QueueDiagnosticsSnapshot};
use crate::terminal::{HostTerminalEvent, HostedTerminal, HostedTerminalError, ReplayBudget};

const RUNTIME_EVENT_CAPACITY: usize = 256;
const RETAINED_ATTACHMENT_QUEUE_DIAGNOSTICS: usize = 4_096;
pub const EXIT_ACK_TTL: Duration = Duration::from_secs(10 * 60);
pub const DEFAULT_CONTROL_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnattendedTakeoverPolicy {
    Never,
    AfterIdle { idle: Duration },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostRuntimeConfig {
    pub takeover_policy: UnattendedTakeoverPolicy,
    pub control_request_timeout: Duration,
    pub replay_budget: ReplayBudget,
}

impl Default for HostRuntimeConfig {
    fn default() -> Self {
        Self {
            takeover_policy: UnattendedTakeoverPolicy::Never,
            control_request_timeout: DEFAULT_CONTROL_REQUEST_TIMEOUT,
            replay_budget: ReplayBudget::default(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TerminalControlOutcome {
    Granted(TerminalLease),
    Pending { holder: ClientInstanceId },
}

#[derive(Clone)]
struct LeaseState {
    holder: ClientInstanceId,
    lease_epoch: u64,
    last_activity_millis: u64,
}

#[derive(Clone)]
struct PendingControlRequest {
    requester: ClientInstanceId,
    requested_at_millis: u64,
}

struct ControlState {
    leases: HashMap<TerminalSessionId, LeaseState>,
    pending: HashMap<TerminalSessionId, PendingControlRequest>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AttachmentResyncReason {
    ScrolledLagged,
    ReturnToBottom,
    ViewportRead,
}

impl AttachmentResyncReason {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::ScrolledLagged => "scrolled_lagged",
            Self::ReturnToBottom => "return_to_bottom",
            Self::ViewportRead => "viewport_read",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AttachmentStreamState {
    pub attached: bool,
    pub display_offset: u64,
    pub pending_raw_after: Option<u64>,
    pub lagged_events: u64,
    pub pending_scroll_resync: bool,
    pub last_resync_reason: Option<AttachmentResyncReason>,
}

impl AttachmentStreamState {
    pub(crate) fn attached() -> Self {
        Self {
            attached: true,
            display_offset: 0,
            pending_raw_after: None,
            lagged_events: 0,
            pending_scroll_resync: false,
            last_resync_reason: None,
        }
    }

    pub(crate) fn detached() -> Self {
        Self {
            attached: false,
            ..Self::attached()
        }
    }
}

pub struct HostRuntime {
    terminals: Mutex<HashMap<TerminalSessionId, HostedTerminal>>,
    control: Mutex<ControlState>,
    attachments:
        Mutex<HashMap<(ClientInstanceId, TerminalSessionId), watch::Sender<AttachmentStreamState>>>,
    next_session_epoch: AtomicU64,
    next_lease_epoch: AtomicU64,
    events: broadcast::Sender<HostTerminalEvent>,
    attachment_queues: Mutex<VecDeque<Arc<QueueDiagnostics>>>,
    config: HostRuntimeConfig,
    clock_override: Mutex<Option<u64>>,
}

impl HostRuntime {
    pub fn new() -> Arc<Self> {
        Self::new_with_config(HostRuntimeConfig::default())
    }

    pub fn new_with_config(config: HostRuntimeConfig) -> Arc<Self> {
        let (events, _) = broadcast::channel(RUNTIME_EVENT_CAPACITY);
        Arc::new(Self {
            terminals: Mutex::new(HashMap::new()),
            control: Mutex::new(ControlState {
                leases: HashMap::new(),
                pending: HashMap::new(),
            }),
            attachments: Mutex::new(HashMap::new()),
            next_session_epoch: AtomicU64::new(1),
            next_lease_epoch: AtomicU64::new(1),
            events,
            attachment_queues: Mutex::new(VecDeque::new()),
            config,
            clock_override: Mutex::new(None),
        })
    }

    pub fn set_now_millis(&self, now: u64) {
        *self.clock_override.lock() = Some(now);
    }

    fn now_millis(&self) -> u64 {
        self.clock_override.lock().unwrap_or_else(system_now_millis)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HostTerminalEvent> {
        self.events.subscribe()
    }

    pub fn spawn(
        self: &Arc<Self>,
        spec: TerminalSpawnSpec,
    ) -> Result<HostedTerminal, HostRuntimeError> {
        self.spawn_with_transport(spec, None, None)
    }

    pub fn spawn_with_transport(
        self: &Arc<Self>,
        spec: TerminalSpawnSpec,
        ssh: Option<TransportService>,
        local_cwd: Option<std::path::PathBuf>,
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
            TerminalExecutionSpec::Ssh { .. } => HostedTerminal::spawn_remote_with_budget(
                spec.clone(),
                session_epoch,
                ssh.as_ref()
                    .ok_or(HostedTerminalError::UnsupportedExecution)?,
                self.config.replay_budget,
            )?,
            TerminalExecutionSpec::Shell { .. } | TerminalExecutionSpec::Command { .. } => {
                HostedTerminal::spawn_in_with_budget(
                    spec.clone(),
                    session_epoch,
                    local_cwd,
                    self.config.replay_budget,
                )?
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

    pub(crate) fn local_process_roots(&self) -> Vec<(TerminalSessionId, u32)> {
        self.terminals
            .lock()
            .values()
            .filter(|terminal| !terminal.is_exited())
            .filter_map(|terminal| {
                terminal
                    .local_process_id()
                    .map(|pid| (terminal.session_id().clone(), pid))
            })
            .collect()
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
        let control = self.control.lock();
        control
            .leases
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
        let now = self.now_millis();
        let mut control = self.control.lock();
        self.reap_session_locked(&mut control, session_id, now);
        if let Some(current) = control.leases.get_mut(session_id) {
            if current.holder == *client_id {
                current.last_activity_millis = now;
                return Ok(wire_lease(session_id, current));
            }
            return Err(HostRuntimeError::LeaseConflict {
                session_id: session_id.clone(),
                holder: current.holder.clone(),
            });
        }
        Ok(self.grant_lease_locked(&mut control, session_id, client_id.clone(), now))
    }

    pub fn request_terminal_control(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> Result<TerminalControlOutcome, HostRuntimeError> {
        if self.terminal(session_id).is_none() {
            return Err(HostRuntimeError::NotFound(session_id.clone()));
        }
        let now = self.now_millis();
        let mut control = self.control.lock();
        self.reap_session_locked(&mut control, session_id, now);
        if let Some(current) = control.leases.get(session_id).cloned() {
            if current.holder == *client_id {
                if let Some(lease) = control.leases.get_mut(session_id) {
                    lease.last_activity_millis = now;
                }
                return Ok(TerminalControlOutcome::Granted(wire_lease(
                    session_id, &current,
                )));
            }
            if self.holder_is_idle(&current, now) {
                let lease = self.transfer_lease_locked(
                    &mut control,
                    session_id,
                    client_id.clone(),
                    current.holder,
                    TransferReason::Expired,
                    now,
                );
                return Ok(TerminalControlOutcome::Granted(lease));
            }
            if let Some(pending) = control.pending.get(session_id) {
                if pending.requester == *client_id {
                    return Ok(TerminalControlOutcome::Pending {
                        holder: current.holder,
                    });
                }
                return Err(HostRuntimeError::ControlRequestConflict {
                    session_id: session_id.clone(),
                    requester: pending.requester.clone(),
                });
            }
            control.pending.insert(
                session_id.clone(),
                PendingControlRequest {
                    requester: client_id.clone(),
                    requested_at_millis: now,
                },
            );
            let holder = current.holder.clone();
            drop(control);
            let _ = self.events.send(HostTerminalEvent::ControlRequested {
                session_id: session_id.clone(),
                holder: holder.clone(),
                requester: client_id.clone(),
            });
            return Ok(TerminalControlOutcome::Pending { holder });
        }
        Ok(TerminalControlOutcome::Granted(self.grant_lease_locked(
            &mut control,
            session_id,
            client_id.clone(),
            now,
        )))
    }

    pub fn validate_lease(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> Result<TerminalLease, HostRuntimeError> {
        let now = self.now_millis();
        let mut control = self.control.lock();
        let Some(lease) = control.leases.get_mut(session_id) else {
            return Err(HostRuntimeError::LeaseRequired(session_id.clone()));
        };
        if lease.holder != *client_id {
            return Err(HostRuntimeError::LeaseConflict {
                session_id: session_id.clone(),
                holder: lease.holder.clone(),
            });
        }
        lease.last_activity_millis = now;
        Ok(wire_lease(session_id, lease))
    }

    pub fn release_lease(&self, session_id: &TerminalSessionId, client_id: &ClientInstanceId) {
        let _ = self.release_control(session_id, client_id);
    }

    pub fn release_control(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> Result<(), HostRuntimeError> {
        let now = self.now_millis();
        let mut control = self.control.lock();
        let Some(current) = control.leases.get(session_id).cloned() else {
            return Err(HostRuntimeError::LeaseRequired(session_id.clone()));
        };
        if current.holder != *client_id {
            return Err(HostRuntimeError::LeaseConflict {
                session_id: session_id.clone(),
                holder: current.holder,
            });
        }
        self.yield_lease_locked(&mut control, session_id, current.holder, now);
        Ok(())
    }

    pub fn reap_pending_control(&self, now_millis: u64) -> usize {
        let session_ids = self
            .control
            .lock()
            .pending
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let mut resolved = 0;
        let mut control = self.control.lock();
        for session_id in session_ids {
            if self.reap_session_locked(&mut control, &session_id, now_millis) {
                resolved += 1;
            }
        }
        resolved
    }

    pub fn release_client(&self, client_id: &ClientInstanceId) {
        let now = self.now_millis();
        let mut control = self.control.lock();
        let held = control
            .leases
            .iter()
            .filter(|(_, lease)| lease.holder == *client_id)
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>();
        for session_id in held {
            self.yield_lease_locked(&mut control, &session_id, client_id.clone(), now);
        }
        control
            .pending
            .retain(|_, pending| pending.requester != *client_id);
        let affected = {
            let mut attachments = self.attachments.lock();
            let affected = attachments
                .keys()
                .filter(|(owner, _)| owner == client_id)
                .map(|(_, session_id)| session_id.clone())
                .collect::<HashSet<_>>();
            attachments.retain(|(owner, _), attached| {
                if owner == client_id {
                    let _ = attached.send(AttachmentStreamState::detached());
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

    fn grant_lease_locked(
        &self,
        control: &mut ControlState,
        session_id: &TerminalSessionId,
        holder: ClientInstanceId,
        now: u64,
    ) -> TerminalLease {
        let lease = LeaseState {
            holder,
            lease_epoch: self.next_lease_epoch.fetch_add(1, Ordering::Relaxed),
            last_activity_millis: now,
        };
        let wire = wire_lease(session_id, &lease);
        control.leases.insert(session_id.clone(), lease);
        control.pending.remove(session_id);
        wire
    }

    fn yield_lease_locked(
        &self,
        control: &mut ControlState,
        session_id: &TerminalSessionId,
        previous_owner: ClientInstanceId,
        now: u64,
    ) {
        control.leases.remove(session_id);
        if let Some(pending) = control.pending.remove(session_id) {
            let lease = self.grant_lease_locked(control, session_id, pending.requester, now);
            let _ = self.events.send(HostTerminalEvent::LeaseReleased {
                session_id: session_id.clone(),
                previous_owner,
            });
            let _ = self.events.send(HostTerminalEvent::ControlGranted {
                session_id: session_id.clone(),
                lease,
            });
            return;
        }
        let _ = self.events.send(HostTerminalEvent::LeaseReleased {
            session_id: session_id.clone(),
            previous_owner,
        });
    }

    fn transfer_lease_locked(
        &self,
        control: &mut ControlState,
        session_id: &TerminalSessionId,
        new_holder: ClientInstanceId,
        previous_owner: ClientInstanceId,
        reason: TransferReason,
        now: u64,
    ) -> TerminalLease {
        control.pending.remove(session_id);
        let lease = self.grant_lease_locked(control, session_id, new_holder, now);
        let event = match reason {
            TransferReason::Expired => HostTerminalEvent::LeaseExpired {
                session_id: session_id.clone(),
                previous_owner,
            },
            TransferReason::Revoked => HostTerminalEvent::LeaseRevoked {
                session_id: session_id.clone(),
                previous_owner,
            },
        };
        let _ = self.events.send(event);
        let _ = self.events.send(HostTerminalEvent::ControlGranted {
            session_id: session_id.clone(),
            lease: lease.clone(),
        });
        lease
    }

    fn holder_is_idle(&self, lease: &LeaseState, now: u64) -> bool {
        match self.config.takeover_policy {
            UnattendedTakeoverPolicy::Never => false,
            UnattendedTakeoverPolicy::AfterIdle { idle } => {
                now.saturating_sub(lease.last_activity_millis) >= idle.as_millis() as u64
            }
        }
    }

    fn request_timed_out(&self, pending: &PendingControlRequest, now: u64) -> bool {
        now.saturating_sub(pending.requested_at_millis)
            >= self.config.control_request_timeout.as_millis() as u64
    }

    fn reap_session_locked(
        &self,
        control: &mut ControlState,
        session_id: &TerminalSessionId,
        now: u64,
    ) -> bool {
        let Some(pending) = control.pending.get(session_id).cloned() else {
            return false;
        };
        if let Some(lease) = control.leases.get(session_id).cloned()
            && self.holder_is_idle(&lease, now)
        {
            self.transfer_lease_locked(
                control,
                session_id,
                pending.requester,
                lease.holder,
                TransferReason::Expired,
                now,
            );
            return true;
        }
        if self.request_timed_out(&pending, now) {
            control.pending.remove(session_id);
            let _ = self.events.send(HostTerminalEvent::ControlDenied {
                session_id: session_id.clone(),
                requester: pending.requester,
                reason: TerminalControlDeniedReason::TimedOut,
            });
            return true;
        }
        false
    }

    pub fn register_attachment(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) {
        let key = (client_id.clone(), session_id.clone());
        let mut attachments = self.attachments.lock();
        if let Some(attached) = attachments.get(&key) {
            // Re-attaching an already tracked stream must not discard the raw cursor,
            // scroll offset or pending resync the client is still waiting on.
            let current = *attached.borrow();
            if !current.attached {
                let _ = attached.send(AttachmentStreamState {
                    attached: true,
                    ..current
                });
            }
        } else {
            let (attached, _) = watch::channel(AttachmentStreamState::attached());
            attachments.insert(key, attached);
        }
        drop(attachments);
        self.refresh_terminal_subscriber_count(session_id);
    }

    pub fn attachment_receiver(
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
            let _ = attached.send(AttachmentStreamState::detached());
        }
        self.refresh_terminal_subscriber_count(session_id);
    }

    pub fn update_attachment_display_offset(
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
            let current = *attached.borrow();
            let last_resync_reason = if current.display_offset != 0 && display_offset == 0 {
                Some(AttachmentResyncReason::ReturnToBottom)
            } else {
                current.last_resync_reason
            };
            let _ = attached.send(AttachmentStreamState {
                attached: true,
                display_offset,
                pending_raw_after: current.pending_raw_after,
                lagged_events: current.lagged_events,
                pending_scroll_resync: if display_offset == 0 {
                    false
                } else {
                    current.pending_scroll_resync
                },
                last_resync_reason,
            });
        }
    }

    pub(crate) fn set_pending_raw_after(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
        after_sequence: Option<u64>,
    ) {
        if let Some(attached) = self
            .attachments
            .lock()
            .get(&(client_id.clone(), session_id.clone()))
        {
            let current = *attached.borrow();
            let _ = attached.send(AttachmentStreamState {
                pending_raw_after: after_sequence,
                ..current
            });
        }
    }

    pub fn note_attachment_lag(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
        skipped: u64,
        scrolled: bool,
    ) {
        if let Some(attached) = self
            .attachments
            .lock()
            .get(&(client_id.clone(), session_id.clone()))
        {
            let current = *attached.borrow();
            let skipped = skipped.max(1);
            let _ = attached.send(AttachmentStreamState {
                lagged_events: current.lagged_events.saturating_add(skipped),
                pending_scroll_resync: current.pending_scroll_resync || scrolled,
                last_resync_reason: if scrolled {
                    Some(AttachmentResyncReason::ScrolledLagged)
                } else {
                    current.last_resync_reason
                },
                ..current
            });
        }
    }

    pub fn mark_attachment_resync(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
        reason: AttachmentResyncReason,
    ) {
        if let Some(attached) = self
            .attachments
            .lock()
            .get(&(client_id.clone(), session_id.clone()))
        {
            let current = *attached.borrow();
            let _ = attached.send(AttachmentStreamState {
                pending_scroll_resync: false,
                last_resync_reason: Some(reason),
                ..current
            });
        }
    }

    pub fn attachment_resync_diagnostics(
        &self,
    ) -> Vec<crate::diagnostics::AttachmentResyncDiagnosticsSnapshot> {
        let mut snapshots = self
            .attachments
            .lock()
            .iter()
            .map(|((client_id, session_id), sender)| {
                let state = *sender.borrow();
                crate::diagnostics::AttachmentResyncDiagnosticsSnapshot {
                    session_id: session_id.as_str().to_string(),
                    client_id: client_id.as_str().to_string(),
                    lagged_events: state.lagged_events,
                    pending_scroll_resync: state.pending_scroll_resync,
                    last_resync_reason: state
                        .last_resync_reason
                        .map(AttachmentResyncReason::as_str)
                        .map(str::to_string),
                }
            })
            .collect::<Vec<_>>();
        snapshots.sort_by(|left, right| {
            left.session_id
                .cmp(&right.session_id)
                .then(left.client_id.cmp(&right.client_id))
        });
        snapshots
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
        let now = self.now_millis();
        let mut control = self.control.lock();
        for session_id in control.pending.keys().cloned().collect::<Vec<_>>() {
            self.reap_session_locked(&mut control, &session_id, now);
        }
        let mut placements = terminals
            .values()
            .map(|terminal| {
                let spec = terminal.spec();
                let (geometry, geometry_epoch, last_sequence, process_state) =
                    terminal.placement_state().unwrap_or((
                        spec.geometry,
                        spec.geometry_epoch,
                        0,
                        TerminalProcessState::Starting,
                    ));
                TerminalPlacement {
                    session_id: spec.session_id.clone(),
                    session_epoch: terminal.session_epoch(),
                    project_id: spec.project_id.clone(),
                    geometry,
                    geometry_epoch,
                    last_sequence,
                    spawn_fingerprint: spec.address_fingerprint(),
                    owner: control
                        .leases
                        .get(&spec.session_id)
                        .map(|lease| lease.holder.clone()),
                    process_state,
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
        let mut control = self.control.lock();
        control.leases.remove(session_id);
        control.pending.remove(session_id);
        self.attachments
            .lock()
            .retain(|(_, attached_session_id), attached| {
                if attached_session_id == session_id {
                    let _ = attached.send(AttachmentStreamState::detached());
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
            let mut control = self.control.lock();
            control
                .leases
                .retain(|session_id, _| terminals.contains_key(session_id));
            control
                .pending
                .retain(|session_id, _| terminals.contains_key(session_id));
            self.attachments.lock().retain(|(_, session_id), attached| {
                if terminals.contains_key(session_id) {
                    true
                } else {
                    let _ = attached.send(AttachmentStreamState::detached());
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

#[derive(Clone, Copy)]
enum TransferReason {
    Expired,
    #[allow(dead_code)]
    Revoked,
}

fn system_now_millis() -> u64 {
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
    #[error("terminal session {session_id} already has a pending control request from {requester}")]
    ControlRequestConflict {
        session_id: TerminalSessionId,
        requester: ClientInstanceId,
    },
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

#[cfg(test)]
mod attachment_state_tests {
    use super::*;

    #[tokio::test]
    async fn reattaching_preserves_pending_raw_cursor_and_scroll_offset() {
        let runtime = HostRuntime::new();
        let client = ClientInstanceId::new("viewer");
        let session_id = TerminalSessionId::new("reattach");

        runtime.register_attachment(&session_id, &client);
        let receiver = runtime
            .attachment_receiver(&session_id, &client)
            .expect("attachment is tracked");
        runtime.set_pending_raw_after(&session_id, &client, Some(42));
        runtime.update_attachment_display_offset(&session_id, &client, 7);
        runtime.register_attachment(&session_id, &client);

        let state = *receiver.borrow();
        assert!(state.attached);
        assert_eq!(state.pending_raw_after, Some(42));
        assert_eq!(state.display_offset, 7);
    }

    #[tokio::test]
    async fn attaching_after_release_starts_from_a_clean_state() {
        let runtime = HostRuntime::new();
        let client = ClientInstanceId::new("viewer");
        let session_id = TerminalSessionId::new("reattach-after-release");

        runtime.register_attachment(&session_id, &client);
        runtime.set_pending_raw_after(&session_id, &client, Some(42));
        runtime.release_attachment(&session_id, &client);
        runtime.register_attachment(&session_id, &client);

        let state = *runtime
            .attachment_receiver(&session_id, &client)
            .expect("attachment was re-registered")
            .borrow();
        assert_eq!(state, AttachmentStreamState::attached());
    }
}
