use std::{
    collections::{HashMap, HashSet, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use tokio::sync::{broadcast, watch};
use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};
use yttt_protocol::{
    HostBlocker, TerminalController, TerminalLease, TerminalPlacement,
    terminal::{
        TerminalExecutionSpec, TerminalLeaseMode, TerminalProcessState, TerminalSpawnSpec,
        TerminatedTerminal,
    },
};
use yttt_ssh::TransportService;

use crate::diagnostics::{QueueDiagnostics, QueueDiagnosticsSnapshot};
use crate::terminal::{HostTerminalEvent, HostedTerminal, HostedTerminalError};

const RUNTIME_EVENT_CAPACITY: usize = 256;
const RETAINED_ATTACHMENT_QUEUE_DIAGNOSTICS: usize = 4_096;
pub const EXIT_ACK_TTL: Duration = Duration::from_secs(10 * 60);

#[derive(Clone)]
struct LeaseState {
    holder: ClientInstanceId,
    lease_epoch: u64,
}

/// How many Clients that have written are remembered, so a long-lived Host
/// does not grow without bound.
const REMEMBERED_WRITERS: usize = 1_024;

struct ControlState {
    /// Every lease is held by `controller`.
    leases: HashMap<TerminalSessionId, LeaseState>,
    controller: Option<ClientInstanceId>,
    controller_epoch: u64,
    /// Clients that have asked for an Interactive lease, oldest first. A
    /// Client asking for the first time takes control; one that lost it
    /// takes it back explicitly.
    writers: VecDeque<ClientInstanceId>,
    /// Connected Clients, least recently connected first.
    present: Vec<ClientInstanceId>,
    /// Clients whose connection dropped but whose leases and control are kept
    /// until they reconnect or their grace period ends.
    suspended: HashSet<ClientInstanceId>,
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
    /// The client renders this terminal; hidden attachments get metadata and exit only.
    pub visible: bool,
    pub display_offset: u64,
    /// Bumped by every scroll or viewport read so the data lane re-sends the view.
    pub view_revision: u64,
    pub lagged_events: u64,
    pub pending_scroll_resync: bool,
    pub last_resync_reason: Option<AttachmentResyncReason>,
}

impl AttachmentStreamState {
    pub(crate) fn attached() -> Self {
        Self {
            attached: true,
            visible: true,
            display_offset: 0,
            view_revision: 0,
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

struct ClientVisibility {
    revision: u64,
    visible: HashSet<TerminalSessionId>,
}

pub struct HostRuntime {
    terminals: Mutex<HashMap<TerminalSessionId, HostedTerminal>>,
    control: Mutex<ControlState>,
    /// Lock order: `visibility` before `attachments`.
    visibility: Mutex<HashMap<ClientInstanceId, ClientVisibility>>,
    attachments:
        Mutex<HashMap<(ClientInstanceId, TerminalSessionId), watch::Sender<AttachmentStreamState>>>,
    next_session_epoch: AtomicU64,
    next_lease_epoch: AtomicU64,
    events: broadcast::Sender<HostTerminalEvent>,
    /// Lease and control changes, apart from the frame updates in `events` so
    /// a burst of output cannot push them out.
    control_events: broadcast::Sender<HostTerminalEvent>,
    attachment_queues: Mutex<VecDeque<Arc<QueueDiagnostics>>>,
}

impl HostRuntime {
    pub fn new() -> Arc<Self> {
        let (events, _) = broadcast::channel(RUNTIME_EVENT_CAPACITY);
        let (control_events, _) = broadcast::channel(RUNTIME_EVENT_CAPACITY);
        Arc::new(Self {
            terminals: Mutex::new(HashMap::new()),
            control: Mutex::new(ControlState {
                leases: HashMap::new(),
                controller: None,
                controller_epoch: 0,
                writers: VecDeque::new(),
                present: Vec::new(),
                suspended: HashSet::new(),
            }),
            visibility: Mutex::new(HashMap::new()),
            attachments: Mutex::new(HashMap::new()),
            next_session_epoch: AtomicU64::new(1),
            next_lease_epoch: AtomicU64::new(1),
            events,
            control_events,
            attachment_queues: Mutex::new(VecDeque::new()),
        })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HostTerminalEvent> {
        self.events.subscribe()
    }

    /// Lease and control changes: `LeaseReleased`, `LeaseRevoked`,
    /// `ControlGranted` and `ControllerChanged`.
    pub fn subscribe_control(&self) -> broadcast::Receiver<HostTerminalEvent> {
        self.control_events.subscribe()
    }

    pub fn spawn(
        self: &Arc<Self>,
        spec: TerminalSpawnSpec,
    ) -> Result<HostedTerminal, HostRuntimeError> {
        self.spawn_with_transport(spec, None, None, |_, _| {})
    }

    pub fn spawn_with_transport(
        self: &Arc<Self>,
        mut spec: TerminalSpawnSpec,
        ssh: Option<TransportService>,
        local_cwd: Option<std::path::PathBuf>,
        prepare_environment: impl FnOnce(&mut TerminalSpawnSpec, u64),
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
        prepare_environment(&mut spec, session_epoch);
        let terminal = match &spec.execution {
            TerminalExecutionSpec::Ssh { .. } => HostedTerminal::spawn_remote(
                spec.clone(),
                session_epoch,
                ssh.as_ref()
                    .ok_or(HostedTerminalError::UnsupportedExecution)?,
            )?,
            TerminalExecutionSpec::Shell { .. } | TerminalExecutionSpec::Command { .. } => {
                HostedTerminal::spawn_in(spec.clone(), session_epoch, local_cwd)?
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
        let diagnostics =
            QueueDiagnostics::new("attachment_output_bytes", yttt_transport::RENDER_LANE_BYTES);
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

    pub fn terminal_controller(&self) -> TerminalController {
        controller_state(&self.control.lock())
    }

    /// Lets `client_id` write terminals. A Client asking for the first time,
    /// or while nobody is in control, takes control; one that lost it must
    /// take it back with `take_terminal_control`.
    pub fn claim_terminal_control(
        &self,
        client_id: &ClientInstanceId,
    ) -> Result<(), HostRuntimeError> {
        let sessions = self.terminal_ids();
        let mut control = self.control.lock();
        self.claim_locked(&mut control, &sessions, client_id)
    }

    /// Makes `client_id` the controller, moving every lease to it.
    pub fn take_terminal_control(&self, client_id: &ClientInstanceId) -> TerminalController {
        let sessions = self.terminal_ids();
        let mut control = self.control.lock();
        remember_writer(&mut control, client_id);
        self.hand_control_locked(&mut control, &sessions, client_id.clone());
        controller_state(&control)
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
        let sessions = self.terminal_ids();
        let mut control = self.control.lock();
        self.claim_locked(&mut control, &sessions, client_id)
            .map_err(|error| match error {
                HostRuntimeError::NotTerminalController { controller } => {
                    HostRuntimeError::LeaseConflict {
                        session_id: session_id.clone(),
                        holder: controller,
                    }
                }
                error => error,
            })?;
        // A terminal spawned after `sessions` was read has no lease yet.
        Ok(match control.leases.get(session_id) {
            Some(lease) if lease.holder == *client_id => wire_lease(session_id, lease),
            Some(lease) => {
                let previous_owner = lease.holder.clone();
                self.transfer_lease_locked(
                    &mut control,
                    session_id,
                    client_id.clone(),
                    previous_owner,
                )
            }
            None => self.grant_lease_locked(&mut control, session_id, client_id.clone()),
        })
    }

    /// Superseded by the terminal controller: grants the lease only to a
    /// Client that may write, like `acquire_lease`.
    pub fn request_terminal_control(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> Result<TerminalLease, HostRuntimeError> {
        self.acquire_lease(session_id, client_id, TerminalLeaseMode::Interactive)
    }

    pub fn validate_lease(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> Result<TerminalLease, HostRuntimeError> {
        let control = self.control.lock();
        let Some(lease) = control.leases.get(session_id) else {
            return Err(HostRuntimeError::LeaseRequired(session_id.clone()));
        };
        if lease.holder != *client_id {
            return Err(HostRuntimeError::LeaseConflict {
                session_id: session_id.clone(),
                holder: lease.holder.clone(),
            });
        }
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
        self.yield_lease_locked(&mut control, session_id, current.holder);
        Ok(())
    }

    /// Releases everything `client_id` holds, including its leases. A
    /// departing controller hands control to the most recently connected
    /// Client that renders a terminal, if any.
    pub fn release_client(&self, client_id: &ClientInstanceId) {
        let sessions = self.terminal_ids();
        let mut control = self.control.lock();
        control.suspended.remove(client_id);
        control.present.retain(|present| present != client_id);
        let held = control
            .leases
            .iter()
            .filter(|(_, lease)| lease.holder == *client_id)
            .map(|(session_id, _)| session_id.clone())
            .collect::<Vec<_>>();
        for session_id in held {
            self.yield_lease_locked(&mut control, &session_id, client_id.clone());
        }
        drop(control);
        self.detach_client(client_id);

        let mut control = self.control.lock();
        if control.controller.as_ref() != Some(client_id) {
            return;
        }
        let viewers = self
            .attachments
            .lock()
            .keys()
            .map(|(owner, _)| owner.clone())
            .collect::<HashSet<_>>();
        match control
            .present
            .iter()
            .rev()
            .find(|present| viewers.contains(*present))
            .cloned()
        {
            Some(successor) => self.hand_control_locked(&mut control, &sessions, successor),
            None => {
                control.controller = None;
                control.controller_epoch += 1;
                let _ = self
                    .control_events
                    .send(HostTerminalEvent::ControllerChanged(controller_state(
                        &control,
                    )));
            }
        }
    }

    /// Ends the streams of a Client whose connection dropped but keeps its
    /// leases and control, so the same Client session resumes them when it
    /// reconnects.
    pub fn suspend_client(&self, client_id: &ClientInstanceId) {
        let mut control = self.control.lock();
        control.suspended.insert(client_id.clone());
        control.present.retain(|present| present != client_id);
        drop(control);
        self.detach_client(client_id);
    }

    /// Marks a Client connected; a suspended one keeps its leases.
    pub fn resume_client(&self, client_id: &ClientInstanceId) {
        let mut control = self.control.lock();
        control.suspended.remove(client_id);
        control.present.retain(|present| present != client_id);
        control.present.push(client_id.clone());
    }

    fn detach_client(&self, client_id: &ClientInstanceId) {
        self.visibility.lock().remove(client_id);
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

    fn terminal_ids(&self) -> Vec<TerminalSessionId> {
        self.terminals.lock().keys().cloned().collect()
    }

    fn claim_locked(
        &self,
        control: &mut ControlState,
        sessions: &[TerminalSessionId],
        client_id: &ClientInstanceId,
    ) -> Result<(), HostRuntimeError> {
        if control.controller.as_ref() == Some(client_id) {
            return Ok(());
        }
        let first_ask = remember_writer(control, client_id);
        match control.controller.clone() {
            Some(controller) if !first_ask => {
                Err(HostRuntimeError::NotTerminalController { controller })
            }
            _ => {
                self.hand_control_locked(control, sessions, client_id.clone());
                Ok(())
            }
        }
    }

    /// Makes `controller` the controller and moves every lease to it,
    /// granting one for each terminal in `sessions` that has none.
    fn hand_control_locked(
        &self,
        control: &mut ControlState,
        sessions: &[TerminalSessionId],
        controller: ClientInstanceId,
    ) {
        if control.controller.as_ref() == Some(&controller) {
            return;
        }
        control.controller = Some(controller.clone());
        control.controller_epoch += 1;
        let _ = self
            .control_events
            .send(HostTerminalEvent::ControllerChanged(controller_state(
                control,
            )));
        let mut targets = sessions.iter().cloned().collect::<HashSet<_>>();
        targets.extend(control.leases.keys().cloned());
        for session_id in targets {
            let lease = match control.leases.get(&session_id) {
                Some(lease) if lease.holder == controller => continue,
                Some(lease) => {
                    let previous_owner = lease.holder.clone();
                    self.transfer_lease_locked(
                        control,
                        &session_id,
                        controller.clone(),
                        previous_owner,
                    );
                    continue;
                }
                None => self.grant_lease_locked(control, &session_id, controller.clone()),
            };
            let _ = self.control_events.send(HostTerminalEvent::ControlGranted {
                session_id: session_id.clone(),
                lease,
            });
        }
    }

    fn grant_lease_locked(
        &self,
        control: &mut ControlState,
        session_id: &TerminalSessionId,
        holder: ClientInstanceId,
    ) -> TerminalLease {
        let lease = LeaseState {
            holder,
            lease_epoch: self.next_lease_epoch.fetch_add(1, Ordering::Relaxed),
        };
        let wire = wire_lease(session_id, &lease);
        control.leases.insert(session_id.clone(), lease);
        wire
    }

    fn yield_lease_locked(
        &self,
        control: &mut ControlState,
        session_id: &TerminalSessionId,
        previous_owner: ClientInstanceId,
    ) {
        control.leases.remove(session_id);
        let _ = self.control_events.send(HostTerminalEvent::LeaseReleased {
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
    ) -> TerminalLease {
        let lease = self.grant_lease_locked(control, session_id, new_holder);
        let _ = self.control_events.send(HostTerminalEvent::LeaseRevoked {
            session_id: session_id.clone(),
            previous_owner,
        });
        let _ = self.control_events.send(HostTerminalEvent::ControlGranted {
            session_id: session_id.clone(),
            lease: lease.clone(),
        });
        lease
    }

    pub fn register_attachment(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) {
        let key = (client_id.clone(), session_id.clone());
        let visibility = self.visibility.lock();
        let visible = visibility
            .get(client_id)
            .is_none_or(|reported| reported.visible.contains(session_id));
        let mut attachments = self.attachments.lock();
        if let Some(attached) = attachments.get(&key) {
            // Re-attaching an already tracked stream keeps its scroll offset.
            let current = *attached.borrow();
            if !current.attached {
                let _ = attached.send(AttachmentStreamState {
                    attached: true,
                    visible,
                    ..current
                });
            }
        } else {
            let (attached, _) = watch::channel(AttachmentStreamState {
                visible,
                ..AttachmentStreamState::attached()
            });
            attachments.insert(key, attached);
        }
        drop(attachments);
        drop(visibility);
        self.refresh_terminal_subscriber_count(session_id);
    }

    /// Records which terminals `client_id` renders. Returns `false` for a
    /// revision not newer than the one already applied.
    pub fn set_client_visibility(
        &self,
        client_id: &ClientInstanceId,
        revision: u64,
        visible: HashSet<TerminalSessionId>,
    ) -> bool {
        let mut visibility = self.visibility.lock();
        if visibility
            .get(client_id)
            .is_some_and(|current| current.revision >= revision)
        {
            return false;
        }
        let changed = {
            let attachments = self.attachments.lock();
            attachments
                .iter()
                .filter(|((owner, _), _)| owner == client_id)
                .filter_map(|((_, session_id), attached)| {
                    let shown = visible.contains(session_id);
                    attached
                        .send_if_modified(|state| {
                            let modified = state.visible != shown;
                            state.visible = shown;
                            modified
                        })
                        .then(|| session_id.clone())
                })
                .collect::<Vec<_>>()
        };
        visibility.insert(client_id.clone(), ClientVisibility { revision, visible });
        drop(visibility);
        for session_id in changed {
            self.refresh_terminal_subscriber_count(&session_id);
        }
        true
    }

    /// Whether `client_id` views `session_id` scrolled back from the bottom.
    pub fn attachment_scrolled(
        &self,
        session_id: &TerminalSessionId,
        client_id: &ClientInstanceId,
    ) -> bool {
        self.attachments
            .lock()
            .get(&(client_id.clone(), session_id.clone()))
            .is_some_and(|attached| attached.borrow().display_offset != 0)
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
                view_revision: current.view_revision.wrapping_add(1),
                pending_scroll_resync: if display_offset == 0 {
                    false
                } else {
                    current.pending_scroll_resync
                },
                last_resync_reason,
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
            .iter()
            .filter(|((_, attached_session_id), attached)| {
                attached_session_id == session_id && attached.borrow().visible
            })
            .count();
        if let Some(terminal) = self.terminal(session_id) {
            terminal.set_subscriber_count(count);
        }
    }

    pub fn placements(&self) -> Vec<TerminalPlacement> {
        let terminals = self.terminals.lock();
        let control = self.control.lock();
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
        self.forget_terminal(session_id);
        Ok(())
    }

    /// Drops an exited terminal so its session can start again. Returns
    /// `false` when no exited terminal holds the session.
    pub fn discard_exited(&self, session_id: &TerminalSessionId) -> bool {
        let mut terminals = self.terminals.lock();
        if !terminals
            .get(session_id)
            .is_some_and(|terminal| terminal.is_exited())
        {
            return false;
        }
        terminals.remove(session_id);
        self.forget_terminal(session_id);
        true
    }

    /// Drops the leases and attachments of a removed terminal. Callers hold
    /// the terminals lock, so a new terminal for the session cannot interleave.
    fn forget_terminal(&self, session_id: &TerminalSessionId) {
        let mut control = self.control.lock();
        control.leases.remove(session_id);
        drop(control);
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

fn controller_state(control: &ControlState) -> TerminalController {
    TerminalController {
        controller: control.controller.clone(),
        epoch: control.controller_epoch,
    }
}

/// Records that `client_id` asked to write; true the first time.
fn remember_writer(control: &mut ControlState, client_id: &ClientInstanceId) -> bool {
    if control.writers.contains(client_id) {
        return false;
    }
    if control.writers.len() == REMEMBERED_WRITERS {
        control.writers.pop_front();
    }
    control.writers.push_back(client_id.clone());
    true
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
    #[error("terminals are controlled by client {controller}")]
    NotTerminalController { controller: ClientInstanceId },
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
    async fn reattaching_preserves_scroll_offset() {
        let runtime = HostRuntime::new();
        let client = ClientInstanceId::new("viewer");
        let session_id = TerminalSessionId::new("reattach");

        runtime.register_attachment(&session_id, &client);
        let receiver = runtime
            .attachment_receiver(&session_id, &client)
            .expect("attachment is tracked");
        runtime.update_attachment_display_offset(&session_id, &client, 7);
        runtime.register_attachment(&session_id, &client);

        let state = *receiver.borrow();
        assert!(state.attached);
        assert_eq!(state.display_offset, 7);
    }

    #[tokio::test]
    async fn visibility_applies_to_current_and_later_attachments_until_the_client_leaves() {
        let runtime = HostRuntime::new();
        let client = ClientInstanceId::new("viewer");
        let shown = TerminalSessionId::new("shown");
        let hidden = TerminalSessionId::new("hidden");
        let visible = |runtime: &HostRuntime, session_id: &TerminalSessionId| {
            runtime
                .attachment_receiver(session_id, &client)
                .expect("attachment is tracked")
                .borrow()
                .visible
        };

        runtime.register_attachment(&shown, &client);
        assert!(
            visible(&runtime, &shown),
            "unreported clients see everything"
        );
        assert!(runtime.set_client_visibility(&client, 2, HashSet::from([shown.clone()])));
        runtime.register_attachment(&hidden, &client);
        assert!(visible(&runtime, &shown));
        assert!(!visible(&runtime, &hidden));

        assert!(!runtime.set_client_visibility(&client, 1, HashSet::from([hidden.clone()])));
        assert!(!visible(&runtime, &hidden), "stale revisions are ignored");

        runtime.release_client(&client);
        runtime.register_attachment(&hidden, &client);
        assert!(
            visible(&runtime, &hidden),
            "a new connection starts unreported"
        );
    }

    #[tokio::test]
    async fn attaching_after_release_starts_from_a_clean_state() {
        let runtime = HostRuntime::new();
        let client = ClientInstanceId::new("viewer");
        let session_id = TerminalSessionId::new("reattach-after-release");

        runtime.register_attachment(&session_id, &client);
        runtime.update_attachment_display_offset(&session_id, &client, 7);
        runtime.release_attachment(&session_id, &client);
        runtime.register_attachment(&session_id, &client);

        let state = *runtime
            .attachment_receiver(&session_id, &client)
            .expect("attachment was re-registered")
            .borrow();
        assert_eq!(state, AttachmentStreamState::attached());
    }
}
