use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::Mutex;
use tokio::{
    io::{AsyncWriteExt as _, split},
    sync::{Notify, watch},
    task::JoinHandle,
};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::{
    ControlMessage, FrameKind, HostEvent, ProtocolCodecError, ServerEvent, encode_message,
    terminal::TerminalStreamUpdate,
};
use yttt_transport_local::LocalStream;

use crate::diagnostics::QueueDiagnostics;

pub(crate) const MAX_ATTACHMENT_OUTPUT_BYTES: usize = 512 * 1024;

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

    fn session_id(&self) -> &TerminalSessionId {
        update_session_id(&self.update)
    }

    fn sequence(&self) -> u64 {
        update_sequence(&self.update)
    }

    pub fn update(&self) -> &TerminalStreamUpdate {
        &self.update
    }
}

struct OutputFrame {
    bytes: Arc<[u8]>,
    is_resync: bool,
}

struct PendingResync {
    host_sequence: u64,
    session_id: TerminalSessionId,
    available_from_sequence: u64,
}

#[derive(Default)]
struct OutputState {
    frames: VecDeque<OutputFrame>,
    bytes_in_use: usize,
    in_flight_bytes: usize,
    resync_pending: bool,
    pending_resync: Option<PendingResync>,
    closed: bool,
}

struct AttachmentOutputQueue {
    state: Mutex<OutputState>,
    ready: Notify,
    diagnostics: Arc<QueueDiagnostics>,
}

impl AttachmentOutputQueue {
    #[cfg(test)]
    fn new() -> Arc<Self> {
        Self::with_diagnostics(QueueDiagnostics::new(
            "attachment_output_bytes",
            MAX_ATTACHMENT_OUTPUT_BYTES,
        ))
    }

    fn with_diagnostics(diagnostics: Arc<QueueDiagnostics>) -> Arc<Self> {
        Arc::new(Self {
            state: Mutex::new(OutputState::default()),
            ready: Notify::new(),
            diagnostics,
        })
    }

    fn enqueue(
        &self,
        host_sequence: u64,
        update: TerminalStreamUpdate,
    ) -> Result<(), ProtocolCodecError> {
        let session_id = update_session_id(&update).clone();
        let available_from_sequence = update_sequence(&update);
        let is_resync = matches!(update, TerminalStreamUpdate::ResyncRequired { .. });
        let message = terminal_message(host_sequence, update);
        let frame = Arc::<[u8]>::from(encode_message(FrameKind::Control, &message)?);
        self.enqueue_frame(
            host_sequence,
            session_id,
            available_from_sequence,
            is_resync,
            frame,
        );
        Ok(())
    }

    fn enqueue_shared(
        &self,
        host_sequence: &AtomicU64,
        update: &SharedTerminalUpdate,
    ) -> Result<(), ProtocolCodecError> {
        let frame = update.encoded(host_sequence)?;
        self.enqueue_frame(
            update.sequence(),
            update.session_id().clone(),
            update.sequence(),
            false,
            frame,
        );
        Ok(())
    }

    fn enqueue_frame(
        &self,
        host_sequence: u64,
        session_id: TerminalSessionId,
        available_from_sequence: u64,
        is_resync: bool,
        frame: Arc<[u8]>,
    ) {
        let mut state = self.state.lock();
        if state.closed || state.resync_pending {
            return;
        }
        if frame.len() <= MAX_ATTACHMENT_OUTPUT_BYTES.saturating_sub(state.bytes_in_use) {
            state.bytes_in_use = state.bytes_in_use.saturating_add(frame.len());
            state.resync_pending = is_resync;
            state.frames.push_back(OutputFrame {
                bytes: frame,
                is_resync,
            });
            self.diagnostics.observe(state.bytes_in_use);
            drop(state);
            self.ready.notify_one();
            return;
        }

        while let Some(frame) = state.frames.pop_front() {
            state.bytes_in_use = state.bytes_in_use.saturating_sub(frame.bytes.len());
        }
        self.diagnostics.dropped();
        self.diagnostics.resync();
        state.resync_pending = true;
        state.pending_resync = Some(PendingResync {
            host_sequence,
            session_id,
            available_from_sequence,
        });
        self.diagnostics.observe(state.bytes_in_use);
        drop(state);
        self.ready.notify_one();
    }

    async fn next_frame(&self) -> Option<OutputFrame> {
        loop {
            let notified = self.ready.notified();
            {
                let mut state = self.state.lock();
                if state.closed {
                    return None;
                }
                if let Some(frame) = state.frames.pop_front() {
                    state.in_flight_bytes = frame.bytes.len();
                    return Some(frame);
                }
                if state.in_flight_bytes == 0
                    && let Some(pending) = state.pending_resync.take()
                {
                    let message = terminal_message(
                        pending.host_sequence,
                        TerminalStreamUpdate::ResyncRequired {
                            session_id: pending.session_id,
                            available_from_sequence: pending.available_from_sequence,
                        },
                    );
                    let frame =
                        Arc::<[u8]>::from(encode_message(FrameKind::Control, &message).ok()?);
                    if frame.len() > MAX_ATTACHMENT_OUTPUT_BYTES {
                        state.closed = true;
                        return None;
                    }
                    state.bytes_in_use = state.bytes_in_use.saturating_add(frame.len());
                    state.in_flight_bytes = frame.len();
                    self.diagnostics.observe(state.bytes_in_use);
                    return Some(OutputFrame {
                        bytes: frame,
                        is_resync: true,
                    });
                }
            }
            notified.await;
        }
    }

    fn complete(&self, bytes: usize, is_resync: bool) {
        let mut state = self.state.lock();
        state.bytes_in_use = state.bytes_in_use.saturating_sub(bytes);
        state.in_flight_bytes = 0;
        if is_resync {
            state.resync_pending = false;
        }
        self.diagnostics.observe(state.bytes_in_use);
        drop(state);
        self.ready.notify_waiters();
    }

    async fn drain(&self) -> Result<(), ()> {
        loop {
            let notified = self.ready.notified();
            {
                let state = self.state.lock();
                if state.closed {
                    return Err(());
                }
                if state.bytes_in_use == 0 && state.pending_resync.is_none() {
                    return Ok(());
                }
            }
            notified.await;
        }
    }

    fn close(&self) {
        let mut state = self.state.lock();
        state.closed = true;
        state.frames.clear();
        state.pending_resync = None;
        state.bytes_in_use = 0;
        state.in_flight_bytes = 0;
        self.diagnostics.observe(0);
        drop(state);
        self.ready.notify_waiters();
    }
}

pub(crate) struct TerminalDataWriter {
    queue: Arc<AttachmentOutputQueue>,
    failed: watch::Receiver<bool>,
    task: JoinHandle<()>,
}

impl TerminalDataWriter {
    pub(crate) fn new(stream: LocalStream, diagnostics: Arc<QueueDiagnostics>) -> Self {
        let (_, mut writer) = split(stream);
        let queue = AttachmentOutputQueue::with_diagnostics(diagnostics);
        let writer_queue = queue.clone();
        let (failed_tx, failed) = watch::channel(false);
        let task = tokio::spawn(async move {
            while let Some(frame) = writer_queue.next_frame().await {
                let bytes = frame.bytes.len();
                let write_started_at = std::time::Instant::now();
                let is_resync = frame.is_resync;
                let result = async {
                    writer.write_all(&frame.bytes).await?;
                    writer.flush().await
                }
                .await;
                writer_queue.complete(bytes, is_resync);
                writer_queue
                    .diagnostics
                    .record_service(write_started_at.elapsed());
                if result.is_err() {
                    writer_queue.close();
                    let _ = failed_tx.send(true);
                    return;
                }
            }
        });
        Self {
            queue,
            failed,
            task,
        }
    }

    pub(crate) fn enqueue(
        &self,
        host_sequence: &AtomicU64,
        update: TerminalStreamUpdate,
    ) -> Result<(), ProtocolCodecError> {
        self.queue
            .enqueue(host_sequence.fetch_add(1, Ordering::Relaxed), update)
    }

    pub(crate) fn enqueue_shared(
        &self,
        host_sequence: &AtomicU64,
        update: &SharedTerminalUpdate,
    ) -> Result<(), ProtocolCodecError> {
        self.queue.enqueue_shared(host_sequence, update)
    }

    pub(crate) fn subscribe_failure(&self) -> watch::Receiver<bool> {
        self.failed.clone()
    }

    pub(crate) async fn drain(&self) -> Result<(), ()> {
        self.queue.drain().await
    }
}

impl Drop for TerminalDataWriter {
    fn drop(&mut self) {
        self.queue.close();
        self.task.abort();
    }
}

fn terminal_message(host_sequence: u64, update: TerminalStreamUpdate) -> ControlMessage {
    ControlMessage::Event(HostEvent {
        host_sequence,
        body: ServerEvent::Terminal(update),
    })
}

fn update_session_id(update: &TerminalStreamUpdate) -> &TerminalSessionId {
    match update {
        TerminalStreamUpdate::Snapshot(viewport) => &viewport.session_id,
        TerminalStreamUpdate::Delta(delta) => &delta.session_id,
        TerminalStreamUpdate::RawTail { session_id, .. }
        | TerminalStreamUpdate::ResyncRequired { session_id, .. } => session_id,
    }
}

fn update_sequence(update: &TerminalStreamUpdate) -> u64 {
    match update {
        TerminalStreamUpdate::Snapshot(viewport) => viewport.sequence,
        TerminalStreamUpdate::Delta(delta) => delta.sequence,
        TerminalStreamUpdate::RawTail { sequence, .. } => *sequence,
        TerminalStreamUpdate::ResyncRequired {
            available_from_sequence,
            ..
        } => *available_from_sequence,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_protocol::{decode_frame, decode_message};

    #[tokio::test]
    async fn overflow_drops_only_this_queue_and_emits_one_resync() {
        let queue = AttachmentOutputQueue::new();
        let session_id = TerminalSessionId::new("overflow");
        let update = |sequence| TerminalStreamUpdate::RawTail {
            session_id: session_id.clone(),
            session_epoch: 1,
            sequence,
            bytes: vec![b'x'; 300 * 1024],
        };
        queue.enqueue(1, update(1)).unwrap();
        queue.enqueue(2, update(2)).unwrap();
        queue.enqueue(3, update(3)).unwrap();

        let frame = queue.next_frame().await.unwrap();
        assert!(frame.bytes.len() <= MAX_ATTACHMENT_OUTPUT_BYTES);
        let decoded = decode_frame(&frame.bytes).unwrap();
        let message: ControlMessage = decode_message(&decoded).unwrap();
        assert!(matches!(
            message,
            ControlMessage::Event(HostEvent {
                body: ServerEvent::Terminal(TerminalStreamUpdate::ResyncRequired {
                    session_id: received,
                    available_from_sequence: 2,
                }),
                ..
            }) if received == session_id
        ));
        let bytes = frame.bytes.len();
        queue.complete(bytes, frame.is_resync);
        assert!(queue.state.lock().frames.is_empty());
        assert_eq!(queue.state.lock().bytes_in_use, 0);
        let diagnostics = queue.diagnostics.snapshot();
        assert_eq!(diagnostics.dropped, 1);
        assert_eq!(diagnostics.resyncs, 1);
        assert_eq!(diagnostics.current, 0);
    }

    #[tokio::test]
    async fn subscribers_share_one_immutable_encoded_frame() {
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
        let first = AttachmentOutputQueue::new();
        let second = AttachmentOutputQueue::new();
        first.enqueue_shared(&host_sequence, &shared).unwrap();
        second.enqueue_shared(&host_sequence, &shared).unwrap();

        let first_frame = first.next_frame().await.unwrap();
        let second_frame = second.next_frame().await.unwrap();
        assert!(Arc::ptr_eq(&first_frame.bytes, &second_frame.bytes));
        assert_eq!(encode_count.load(Ordering::Acquire), 1);
        assert_eq!(host_sequence.load(Ordering::Acquire), 12);
    }
}
