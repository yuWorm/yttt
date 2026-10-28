use std::{
    collections::VecDeque,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};

use parking_lot::Mutex;
use tokio::{
    io::{AsyncReadExt, split},
    sync::{Notify, watch},
    task::JoinHandle,
};
use yttt_protocol::{
    ControlMessage, FrameKind, HostEvent, ProtocolCodecError, ServerEvent, encode_message,
    terminal::TerminalStreamUpdate,
};
use yttt_transport::TransportStream;

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

    pub fn update(&self) -> &TerminalStreamUpdate {
        &self.update
    }
}

#[derive(Default)]
struct OutputState {
    frames: VecDeque<Arc<[u8]>>,
    bytes_in_use: usize,
    in_flight_bytes: usize,
    closed: bool,
}

struct AttachmentOutputQueue {
    state: Mutex<OutputState>,
    ready: Notify,
    diagnostics: Arc<QueueDiagnostics>,
}

/// The queue refused a frame: it would exceed the attachment budget, or the
/// connection is closing. The stream cannot continue from the sent baseline.
#[derive(Debug)]
pub(crate) struct FrameRejected;

impl From<ProtocolCodecError> for FrameRejected {
    fn from(_: ProtocolCodecError) -> Self {
        Self
    }
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
    ) -> Result<(), FrameRejected> {
        let message = terminal_message(host_sequence, update);
        let frame = Arc::<[u8]>::from(encode_message(FrameKind::Control, &message)?);
        self.enqueue_frame(frame)
    }

    fn enqueue_shared(
        &self,
        host_sequence: &AtomicU64,
        update: &SharedTerminalUpdate,
    ) -> Result<(), FrameRejected> {
        let frame = update.encoded(host_sequence)?;
        self.enqueue_frame(frame)
    }

    /// Admits a frame while the queue stays within its byte budget. A frame
    /// larger than the budget (images) is admitted only into an empty queue;
    /// `encode_message` already enforced the hard wire frame limit.
    fn enqueue_frame(&self, frame: Arc<[u8]>) -> Result<(), FrameRejected> {
        let mut state = self.state.lock();
        if state.closed {
            return Err(FrameRejected);
        }
        let budget = MAX_ATTACHMENT_OUTPUT_BYTES.max(frame.len());
        if frame.len() > budget.saturating_sub(state.bytes_in_use) {
            self.diagnostics.dropped();
            return Err(FrameRejected);
        }
        state.bytes_in_use = state.bytes_in_use.saturating_add(frame.len());
        state.frames.push_back(frame);
        self.diagnostics.observe(state.bytes_in_use);
        drop(state);
        self.ready.notify_one();
        Ok(())
    }

    fn is_idle(&self) -> bool {
        self.state.lock().bytes_in_use == 0
    }

    async fn next_frame(&self) -> Option<Arc<[u8]>> {
        loop {
            let notified = self.ready.notified();
            {
                let mut state = self.state.lock();
                if state.closed {
                    return None;
                }
                if let Some(frame) = state.frames.pop_front() {
                    state.in_flight_bytes = frame.len();
                    return Some(frame);
                }
            }
            notified.await;
        }
    }

    fn complete(&self) {
        let mut state = self.state.lock();
        state.bytes_in_use = state.bytes_in_use.saturating_sub(state.in_flight_bytes);
        state.in_flight_bytes = 0;
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
                if state.bytes_in_use == 0 {
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
    pub(crate) fn new(stream: TransportStream, diagnostics: Arc<QueueDiagnostics>) -> Self {
        let (mut reader, mut writer) = split(stream);
        let queue = AttachmentOutputQueue::with_diagnostics(diagnostics);
        let writer_queue = queue.clone();
        let (failed_tx, failed) = watch::channel(false);
        let task = tokio::spawn(async move {
            let write = async {
                while let Some(frame) = writer_queue.next_frame().await {
                    let write_started_at = std::time::Instant::now();
                    let result = yttt_transport::send_frame(&mut writer, &frame).await;
                    writer_queue.complete();
                    writer_queue
                        .diagnostics
                        .record_service(write_started_at.elapsed());
                    if result.is_err() {
                        writer_queue.close();
                        return;
                    }
                }
            };
            let mut byte = [0];
            tokio::select! {
                _ = write => {},
                _ = reader.read(&mut byte) => {},
            }
            writer_queue.close();
            let _ = failed_tx.send(true);
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
    ) -> Result<(), FrameRejected> {
        self.queue
            .enqueue(host_sequence.fetch_add(1, Ordering::Relaxed), update)
    }

    pub(crate) fn enqueue_shared(
        &self,
        host_sequence: &AtomicU64,
        update: &SharedTerminalUpdate,
    ) -> Result<(), FrameRejected> {
        self.queue.enqueue_shared(host_sequence, update)
    }

    /// No frame is queued or being written.
    pub(crate) fn is_idle(&self) -> bool {
        self.queue.is_idle()
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

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_core::model::ids::TerminalSessionId;
    use yttt_protocol::{decode_frame, decode_message};

    fn raw(sequence: u64, bytes: usize) -> TerminalStreamUpdate {
        TerminalStreamUpdate::RawTail {
            session_id: TerminalSessionId::new("budget"),
            session_epoch: 1,
            sequence,
            bytes: vec![b'x'; bytes],
        }
    }

    #[tokio::test]
    async fn over_budget_frames_are_refused_without_dropping_queued_frames() {
        let queue = AttachmentOutputQueue::new();
        queue.enqueue(1, raw(1, 300 * 1024)).unwrap();
        assert!(queue.enqueue(2, raw(2, 300 * 1024)).is_err());
        assert_eq!(queue.state.lock().frames.len(), 1);
        assert_eq!(queue.diagnostics.snapshot().dropped, 1);

        let frame = queue.next_frame().await.unwrap();
        assert!(!queue.is_idle(), "an in-flight frame keeps the queue busy");
        let decoded = decode_frame(&frame).unwrap();
        let message: ControlMessage = decode_message(&decoded).unwrap();
        assert!(matches!(
            message,
            ControlMessage::Event(HostEvent {
                body: ServerEvent::Terminal(TerminalStreamUpdate::RawTail { sequence: 1, .. }),
                ..
            })
        ));
        queue.complete();
        assert!(queue.is_idle());
        assert_eq!(queue.diagnostics.snapshot().current, 0);
    }

    #[tokio::test]
    async fn an_oversized_frame_is_admitted_only_into_an_empty_queue() {
        let queue = AttachmentOutputQueue::new();
        queue.enqueue(1, raw(1, 16)).unwrap();
        let oversized = MAX_ATTACHMENT_OUTPUT_BYTES + 1024;
        assert!(queue.enqueue(2, raw(2, oversized)).is_err());
        let _ = queue.next_frame().await.unwrap();
        queue.complete();
        queue.enqueue(3, raw(3, oversized)).unwrap();
        assert!(!queue.is_idle());
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
        assert!(Arc::ptr_eq(&first_frame, &second_frame));
        assert_eq!(encode_count.load(Ordering::Acquire), 1);
        assert_eq!(host_sequence.load(Ordering::Acquire), 12);
    }
}

#[cfg(test)]
mod disconnect_tests {
    use super::*;

    #[tokio::test]
    async fn idle_data_writer_notices_peer_disconnect() {
        let (server, client) = tokio::io::duplex(1024);
        let writer = TerminalDataWriter::new(Box::new(server), QueueDiagnostics::new("test", 1024));
        let mut failed = writer.subscribe_failure();
        drop(client);
        tokio::time::timeout(std::time::Duration::from_secs(1), failed.changed())
            .await
            .unwrap()
            .unwrap();
        assert!(*failed.borrow());
    }
}
