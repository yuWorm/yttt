//! Terminal frames for one Session connection: each attached terminal streams
//! over its own render lane, opening with a Snapshot and then sending only frames
//! that apply to what the lane already sent.

use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

use parking_lot::Mutex;
use tokio::{
    sync::{broadcast, watch},
    task::JoinHandle,
};
use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};
use yttt_protocol::{
    ControlMessage, FrameKind, HostEvent, ProtocolCodecError, ServerEvent, encode_message,
    terminal::{
        SemanticDelta, SemanticViewport, TerminalModes, TerminalProcessState, TerminalStreamApply,
        TerminalStreamUpdate,
    },
};
use yttt_transport::{RenderLane, RenderRejected, SessionWriter};

use crate::{
    runtime::{AttachmentResyncReason, AttachmentStreamState, HostRuntime},
    terminal::{HostTerminalEvent, HostedTerminal},
};

/// Paces render frames so scheduler lateness does not accumulate.
const TERMINAL_FRAME_INTERVAL: Duration = Duration::from_millis(16);

#[derive(Debug)]
pub struct SharedTerminalUpdate {
    update: TerminalStreamUpdate,
    encoded: Mutex<Option<Arc<[u8]>>>,
    encode_count: Arc<AtomicU64>,
}

impl SharedTerminalUpdate {
    pub(crate) fn new(update: TerminalStreamUpdate, encode_count: Arc<AtomicU64>) -> Arc<Self> {
        Arc::new(Self {
            update,
            encoded: Mutex::new(None),
            encode_count,
        })
    }

    /// Encodes the frame once; every attached client shares the bytes.
    fn encoded(&self, host_sequence: &AtomicU64) -> Result<Arc<[u8]>, ProtocolCodecError> {
        let mut encoded = self.encoded.lock();
        if let Some(frame) = encoded.as_ref() {
            return Ok(frame.clone());
        }
        let message = terminal_message(
            host_sequence.fetch_add(1, Ordering::Relaxed),
            self.update.clone(),
        );
        let frame = Arc::<[u8]>::from(encode_message(FrameKind::Control, &message)?);
        self.encode_count.fetch_add(1, Ordering::Relaxed);
        *encoded = Some(frame.clone());
        Ok(frame)
    }

    pub fn update(&self) -> &TerminalStreamUpdate {
        &self.update
    }
}

fn enqueue_update(
    lane: &RenderLane,
    host_sequence: &AtomicU64,
    update: TerminalStreamUpdate,
) -> Result<(), RenderRejected> {
    let message = terminal_message(host_sequence.fetch_add(1, Ordering::Relaxed), update);
    let frame = encode_message(FrameKind::Control, &message).map_err(|_| RenderRejected)?;
    lane.enqueue_frame(Arc::from(frame))
}

fn enqueue_shared(
    lane: &RenderLane,
    host_sequence: &AtomicU64,
    update: &SharedTerminalUpdate,
) -> Result<(), RenderRejected> {
    lane.enqueue_frame(update.encoded(host_sequence).map_err(|_| RenderRejected)?)
}

fn terminal_message(host_sequence: u64, update: TerminalStreamUpdate) -> ControlMessage {
    ControlMessage::Event(HostEvent {
        host_sequence,
        body: ServerEvent::Terminal(update),
    })
}

/// What the client mirror holds once every frame queued on a render lane is applied.
struct SentBaseline {
    session_epoch: u64,
    sequence: u64,
    geometry_epoch: u64,
    scrollback_epoch: u64,
    history_size: u64,
    display_offset: u64,
    modes: TerminalModes,
    process_state: TerminalProcessState,
    /// The rows equal the Host's live view at `sequence`, so shared deltas apply.
    follows_live: bool,
}

impl SentBaseline {
    fn from_viewport(viewport: &SemanticViewport) -> Self {
        Self {
            session_epoch: viewport.session_epoch,
            sequence: viewport.sequence,
            geometry_epoch: viewport.geometry_epoch,
            scrollback_epoch: viewport.scrollback_epoch,
            history_size: viewport.history_size,
            display_offset: viewport.display_offset,
            modes: viewport.modes.clone(),
            process_state: viewport.process_state,
            follows_live: viewport.display_offset == 0,
        }
    }

    fn advance(&mut self, update: &TerminalStreamUpdate) {
        match update {
            TerminalStreamUpdate::Snapshot(viewport) => *self = Self::from_viewport(viewport),
            TerminalStreamUpdate::Delta(delta) => {
                self.sequence = delta.sequence;
                self.geometry_epoch = delta.geometry_epoch;
                self.scrollback_epoch = delta.scrollback_epoch;
                self.history_size = delta.history_size;
                self.display_offset = delta.display_offset;
                if let Some(modes) = &delta.modes {
                    self.modes.clone_from(modes);
                }
                if let Some(process_state) = delta.process_state {
                    self.process_state = process_state;
                }
            }
            TerminalStreamUpdate::RawTail { .. } | TerminalStreamUpdate::ResyncRequired { .. } => {}
        }
    }

    fn metadata_differs(&self, update: &TerminalStreamUpdate) -> bool {
        match update {
            TerminalStreamUpdate::Snapshot(viewport) => {
                viewport.modes != self.modes || viewport.process_state != self.process_state
            }
            TerminalStreamUpdate::Delta(delta) => {
                delta
                    .modes
                    .as_ref()
                    .is_some_and(|modes| *modes != self.modes)
                    || delta
                        .process_state
                        .is_some_and(|process_state| process_state != self.process_state)
            }
            TerminalStreamUpdate::RawTail { .. } | TerminalStreamUpdate::ResyncRequired { .. } => {
                false
            }
        }
    }

    /// Title, cwd, mode bits and process state for a client that does not render
    /// the rows. The rows it holds stay as they are.
    fn metadata_delta(&self, current: &SemanticViewport) -> Option<SemanticDelta> {
        if current.session_epoch != self.session_epoch || current.sequence <= self.sequence {
            return None;
        }
        let modes = (current.modes != self.modes).then(|| current.modes.clone());
        let process_state =
            (current.process_state != self.process_state).then_some(current.process_state);
        if modes.is_none() && process_state.is_none() {
            return None;
        }
        Some(SemanticDelta {
            session_id: current.session_id.clone(),
            session_epoch: self.session_epoch,
            base_sequence: self.sequence,
            sequence: current.sequence,
            geometry_epoch: self.geometry_epoch,
            scrollback_epoch: self.scrollback_epoch,
            history_size: self.history_size,
            display_offset: self.display_offset,
            changed_rows: Vec::new(),
            cursor: None,
            modes,
            palette: None,
            process_state,
            images: None,
            placements: None,
        })
    }
}

/// The newest frame a render lane owes its client. It keeps merging while the
/// writer is busy, so a slow client receives one combined frame, never a gap.
enum PendingTerminalFrame {
    Shared(Arc<SharedTerminalUpdate>),
    Update(Box<TerminalStreamUpdate>),
    SnapshotRequired,
}

impl PendingTerminalFrame {
    fn push(
        current: Option<Self>,
        baseline: &SentBaseline,
        update: Arc<SharedTerminalUpdate>,
    ) -> Option<Self> {
        let Some(current) = current else {
            return Self::start(baseline, update);
        };
        if matches!(update.update(), TerminalStreamUpdate::Snapshot(_)) {
            return Some(Self::Shared(update));
        }
        Some(match current {
            Self::Shared(current) => {
                let mut current = current.update().clone();
                if merge_terminal_updates(&mut current, update.update()) {
                    Self::Update(Box::new(current))
                } else {
                    Self::SnapshotRequired
                }
            }
            Self::Update(mut current) => {
                if merge_terminal_updates(current.as_mut(), update.update()) {
                    Self::Update(current)
                } else {
                    Self::SnapshotRequired
                }
            }
            Self::SnapshotRequired => Self::SnapshotRequired,
        })
    }

    fn start(baseline: &SentBaseline, update: Arc<SharedTerminalUpdate>) -> Option<Self> {
        match update.update() {
            TerminalStreamUpdate::Snapshot(viewport)
                if viewport.session_epoch == baseline.session_epoch
                    && viewport.sequence < baseline.sequence =>
            {
                None
            }
            TerminalStreamUpdate::Snapshot(_) => Some(Self::Shared(update)),
            TerminalStreamUpdate::Delta(delta)
                if delta.session_epoch == baseline.session_epoch
                    && delta.sequence <= baseline.sequence =>
            {
                None
            }
            TerminalStreamUpdate::Delta(delta)
                if baseline.follows_live
                    && delta.session_epoch == baseline.session_epoch
                    && delta.base_sequence == baseline.sequence =>
            {
                Some(Self::Shared(update))
            }
            _ => Some(Self::SnapshotRequired),
        }
    }

    fn enqueue(
        self,
        terminal: &HostedTerminal,
        lane: &RenderLane,
        host_sequence: &AtomicU64,
        baseline: &mut SentBaseline,
        display_offset: u64,
    ) -> Result<(), ()> {
        match self {
            Self::Shared(update) => {
                enqueue_shared(lane, host_sequence, update.as_ref()).map_err(|_| ())?;
                baseline.advance(update.update());
            }
            Self::Update(update) => {
                baseline.advance(&update);
                enqueue_update(lane, host_sequence, *update).map_err(|_| ())?;
            }
            Self::SnapshotRequired => {
                *baseline = send_snapshot(terminal, lane, host_sequence, display_offset)?;
            }
        }
        Ok(())
    }
}

fn merge_terminal_updates(current: &mut TerminalStreamUpdate, next: &TerminalStreamUpdate) -> bool {
    match current {
        TerminalStreamUpdate::Snapshot(viewport) => !matches!(
            viewport.apply_stream_update_ref(next),
            TerminalStreamApply::SequenceGap
        ),
        TerminalStreamUpdate::Delta(current) => match next {
            TerminalStreamUpdate::Delta(next) => current.merge_contiguous(next.clone()),
            _ => false,
        },
        _ => false,
    }
}

fn send_snapshot(
    terminal: &HostedTerminal,
    lane: &RenderLane,
    host_sequence: &AtomicU64,
    display_offset: u64,
) -> Result<SentBaseline, ()> {
    let viewport = terminal.current_viewport(display_offset).ok_or(())?;
    let baseline = SentBaseline::from_viewport(&viewport);
    enqueue_update(
        lane,
        host_sequence,
        TerminalStreamUpdate::Snapshot(viewport),
    )
    .map_err(|_| ())?;
    Ok(baseline)
}

/// One terminal streaming to one Session connection. Dropping it stops the
/// stream and discards frames not yet started.
pub(crate) struct TerminalStream {
    task: JoinHandle<()>,
}

impl Drop for TerminalStream {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub(crate) struct TerminalStreamContext {
    pub(crate) writer: SessionWriter,
    pub(crate) runtime: Arc<HostRuntime>,
    pub(crate) host_sequence: Arc<AtomicU64>,
    pub(crate) client_id: ClientInstanceId,
}

impl TerminalStream {
    /// Starts streaming `session_id`. Call it after the response that attached
    /// the terminal is queued, so the opening Snapshot follows that response.
    pub(crate) fn start(context: &TerminalStreamContext, session_id: TerminalSessionId) -> Self {
        let terminal = context.runtime.terminal(&session_id);
        let attachment = context
            .runtime
            .attachment_receiver(&session_id, &context.client_id);
        let events = terminal.as_ref().map(|terminal| terminal.subscribe());
        let lane = context
            .writer
            .open_render_lane(Some(context.runtime.new_attachment_queue_diagnostics()));
        let writer = context.writer.clone();
        let runtime = context.runtime.clone();
        let host_sequence = context.host_sequence.clone();
        let client_id = context.client_id.clone();
        let task = tokio::spawn(async move {
            let (Some(terminal), Some(attachment), Some(events)) = (terminal, attachment, events)
            else {
                return;
            };
            let _ = stream_terminal(StreamInputs {
                terminal: &terminal,
                session_id,
                client_id,
                runtime: &runtime,
                host_sequence: &host_sequence,
                writer: &writer,
                lane: &lane,
                attachment,
                events,
            })
            .await;
        });
        Self { task }
    }
}

struct StreamInputs<'a> {
    terminal: &'a HostedTerminal,
    session_id: TerminalSessionId,
    client_id: ClientInstanceId,
    runtime: &'a HostRuntime,
    host_sequence: &'a AtomicU64,
    writer: &'a SessionWriter,
    lane: &'a RenderLane,
    attachment: watch::Receiver<AttachmentStreamState>,
    events: broadcast::Receiver<HostTerminalEvent>,
}

/// Streams one terminal until it detaches or exits. The final frame of an
/// exited terminal reaches the client before its `TerminalExit` event.
async fn stream_terminal(inputs: StreamInputs<'_>) -> Result<(), ()> {
    let StreamInputs {
        terminal,
        session_id,
        client_id,
        runtime,
        host_sequence,
        writer,
        lane,
        mut attachment,
        mut events,
    } = inputs;
    let mut view = *attachment.borrow();
    if !view.attached {
        return Err(());
    }
    let live = |view: &AttachmentStreamState| view.visible && view.display_offset == 0;
    let mut baseline = send_snapshot(terminal, lane, host_sequence, view.display_offset)?;
    if terminal.is_exited() {
        return lane.drain().await.map_err(|_| ());
    }
    let mut pending_frame = None::<PendingTerminalFrame>;
    let mut metadata_dirty = false;
    // Skip missed ticks after idle periods instead of bursting to catch up.
    let mut frame_interval = tokio::time::interval(TERMINAL_FRAME_INTERVAL);
    frame_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            event = events.recv() => {
                match event {
                    Ok(HostTerminalEvent::Update {
                        session_id: update_session_id,
                        update,
                    }) if update_session_id == session_id => {
                        if live(&view) {
                            pending_frame =
                                PendingTerminalFrame::push(pending_frame.take(), &baseline, update);
                        } else if baseline.metadata_differs(update.update()) {
                            metadata_dirty = true;
                        }
                    }
                    Ok(HostTerminalEvent::Exited {
                        session_id: exited_session_id,
                        session_epoch,
                        code,
                        final_sequence,
                    }) if exited_session_id == session_id => {
                        // The final frame goes out even to hidden attachments, after
                        // everything already queued, and the exit event after it.
                        lane.drain().await.map_err(|_| ())?;
                        send_snapshot(terminal, lane, host_sequence, view.display_offset)?;
                        writer
                            .send_reliable_after(
                                lane,
                                &ControlMessage::Event(HostEvent {
                                    host_sequence: host_sequence.fetch_add(1, Ordering::Relaxed),
                                    body: ServerEvent::TerminalExit {
                                        session_id,
                                        session_epoch,
                                        code,
                                        final_sequence,
                                    },
                                }),
                            )
                            .map_err(|_| ())?;
                        // The lane must outlive its queued frames.
                        return lane.drain().await.map_err(|_| ());
                    }
                    Ok(_) => {}
                    Err(broadcast::error::RecvError::Lagged(skipped)) => {
                        runtime.note_attachment_lag(&session_id, &client_id, skipped, !live(&view));
                        if live(&view) {
                            pending_frame = Some(PendingTerminalFrame::SnapshotRequired);
                        } else {
                            metadata_dirty = true;
                        }
                    }
                    Err(broadcast::error::RecvError::Closed) => return Err(()),
                }
            }
            changed = attachment.changed() => {
                if changed.is_err() {
                    return Ok(());
                }
                let previous = view;
                view = *attachment.borrow();
                if !view.attached {
                    return Ok(());
                }
                if !view.visible {
                    pending_frame = None;
                } else if !previous.visible
                    || previous.display_offset != view.display_offset
                    || previous.view_revision != view.view_revision
                {
                    if previous.display_offset != 0 && view.display_offset == 0 {
                        runtime.mark_attachment_resync(
                            &session_id,
                            &client_id,
                            AttachmentResyncReason::ReturnToBottom,
                        );
                    }
                    pending_frame = Some(PendingTerminalFrame::SnapshotRequired);
                }
            }
            _ = frame_interval.tick(), if pending_frame.is_some() || metadata_dirty => {
                // One frame queued at a time: a busy connection leaves the frame
                // merging in its slot instead of growing the lane.
                if lane.is_idle() {
                    if let Some(pending) = pending_frame.take() {
                        pending.enqueue(
                            terminal,
                            lane,
                            host_sequence,
                            &mut baseline,
                            view.display_offset,
                        )?;
                    } else if let Some(delta) = terminal
                        .latest_viewport()
                        .and_then(|current| baseline.metadata_delta(&current))
                    {
                        let update = TerminalStreamUpdate::Delta(delta);
                        baseline.advance(&update);
                        baseline.follows_live = false;
                        enqueue_update(lane, host_sequence, update).map_err(|_| ())?;
                    }
                    metadata_dirty = false;
                }
            }
            () = writer.closed() => return Err(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_protocol::{decode_frame, decode_message};

    #[test]
    fn subscribers_share_one_immutable_encoded_frame() {
        let encode_count = Arc::new(AtomicU64::new(0));
        let shared = SharedTerminalUpdate::new(
            TerminalStreamUpdate::RawTail {
                session_id: TerminalSessionId::new("shared"),
                session_epoch: 1,
                sequence: 7,
                bytes: b"shared-output".to_vec(),
            },
            encode_count.clone(),
        );
        let host_sequence = AtomicU64::new(11);
        let first = shared.encoded(&host_sequence).unwrap();
        let second = shared.encoded(&host_sequence).unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert_eq!(encode_count.load(Ordering::Acquire), 1);
        assert_eq!(host_sequence.load(Ordering::Acquire), 12);
        let message: ControlMessage = decode_message(&decode_frame(&first).unwrap()).unwrap();
        assert!(matches!(
            message,
            ControlMessage::Event(HostEvent {
                host_sequence: 11,
                body: ServerEvent::Terminal(TerminalStreamUpdate::RawTail { sequence: 7, .. }),
            })
        ));
    }
}
