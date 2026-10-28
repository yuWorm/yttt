//! One `Session` connection carries every message of an authenticated Client.
//!
//! [`SessionWriter`] owns the write half and sends, in priority order:
//! 1. interactive messages (terminal input, resize, scroll, heartbeats, and their responses);
//! 2. reliable messages (other requests, responses, and events) in FIFO order;
//! 3. render frames, one item at a time per [`RenderLane`], round-robin across lanes.
//!
//! Queues never wait for space: a sender that exceeds a bound closes the
//! connection, and the peer reconnects and reconciles.
//!
//! Messages larger than [`CHUNK_PAYLOAD_BYTES`] are split into
//! `ControlMessage::Chunk` frames. Interactive frames may be written between
//! the chunks of another message; reliable and render messages never
//! interleave with each other, so reliable order holds end to end.
//! [`SessionReader`] reassembles chunks on the receiving side.

use std::{
    collections::{HashMap, VecDeque},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use tokio::{
    io::{AsyncRead, AsyncWrite},
    sync::{Notify, watch},
    task::JoinHandle,
};
use yttt_protocol::{
    ControlMessage, FrameKind, HEADER_LEN, MAX_FRAME_BYTES, MessageChunk, ProtocolCodecError,
    decode_control_message, decode_control_payload, encode_message,
};

use crate::wire::{WireError, WireReceiveDiagnostics, receive_frame, resolve_control, send_frame};

/// Encoded payloads above this size travel as `ControlMessage::Chunk` frames.
pub const CHUNK_PAYLOAD_BYTES: usize = 256 * 1024;
/// Byte budget of one render lane. A larger frame is admitted only into an empty lane.
pub const RENDER_LANE_BYTES: usize = 512 * 1024;
const INTERACTIVE_ITEMS: usize = 1024;
const RELIABLE_ITEMS: usize = 1024;
/// A larger reliable message is admitted only into an empty queue.
const RELIABLE_BYTES: usize = MAX_FRAME_BYTES;
const MAX_OPEN_CHUNK_STREAMS: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum SessionSendError {
    #[error("session connection closed")]
    Closed,
    #[error("session send queue overflowed; the connection was closed")]
    Overflow,
    #[error(transparent)]
    Encode(#[from] ProtocolCodecError),
}

/// A render lane refused a frame: it exceeds the lane budget or the connection closed.
#[derive(Debug)]
pub struct RenderRejected;

/// Observes one render lane's queue for diagnostics.
pub trait RenderObserver: Send + Sync {
    fn observe(&self, queued_bytes: usize);
    fn dropped(&self);
    fn record_service(&self, elapsed: Duration);
}

struct Item {
    frames: VecDeque<Arc<[u8]>>,
    bytes: usize,
    /// A reliable item waits until this render lane has no queued frame.
    after_lane: Option<u64>,
    /// A later reliable item with the same key replaces this one while it waits.
    latest: Option<Arc<str>>,
}

struct RenderQueue {
    items: VecDeque<Item>,
    bytes: usize,
    ready: bool,
    observer: Option<Arc<dyn RenderObserver>>,
}

enum Source {
    Reliable,
    Render { lane: u64, started_at: Instant },
}

#[derive(Clone, Copy)]
enum Slot {
    Interactive,
    Current,
}

#[derive(Default)]
struct State {
    closed: bool,
    interactive: VecDeque<Item>,
    current_interactive: Option<Item>,
    reliable: VecDeque<Item>,
    reliable_bytes: usize,
    current: Option<(Source, Item)>,
    render: HashMap<u64, RenderQueue>,
    render_ready: VecDeque<u64>,
}

impl State {
    fn select(&mut self) -> Option<(Arc<[u8]>, Slot)> {
        if self.current_interactive.is_none() {
            self.current_interactive = self.interactive.pop_front();
        }
        if let Some(item) = &mut self.current_interactive {
            return item
                .frames
                .pop_front()
                .map(|frame| (frame, Slot::Interactive));
        }
        if self.current.is_none() {
            self.current = self.next_current();
        }
        let (_, item) = self.current.as_mut()?;
        item.frames.pop_front().map(|frame| (frame, Slot::Current))
    }

    fn next_current(&mut self) -> Option<(Source, Item)> {
        if let Some(front) = self.reliable.front() {
            // Terminal barrier: frames already queued for the lane go first.
            if let Some(lane) = front.after_lane
                && let Some(item) = self.pop_render(lane)
            {
                return Some(item);
            }
            let item = self.reliable.pop_front()?;
            return Some((Source::Reliable, item));
        }
        while let Some(lane) = self.render_ready.pop_front() {
            let Some(queue) = self.render.get_mut(&lane) else {
                continue;
            };
            queue.ready = false;
            if let Some(item) = self.pop_render(lane) {
                return Some(item);
            }
        }
        None
    }

    fn pop_render(&mut self, lane: u64) -> Option<(Source, Item)> {
        let queue = self.render.get_mut(&lane)?;
        let item = queue.items.pop_front()?;
        if !queue.items.is_empty() && !queue.ready {
            queue.ready = true;
            self.render_ready.push_back(lane);
        }
        Some((
            Source::Render {
                lane,
                started_at: Instant::now(),
            },
            item,
        ))
    }

    fn finish(&mut self, slot: Slot) {
        match slot {
            Slot::Interactive => {
                if self
                    .current_interactive
                    .as_ref()
                    .is_some_and(|item| item.frames.is_empty())
                {
                    self.current_interactive = None;
                }
            }
            Slot::Current => {
                if !self
                    .current
                    .as_ref()
                    .is_some_and(|(_, item)| item.frames.is_empty())
                {
                    return;
                }
                let Some((source, item)) = self.current.take() else {
                    return;
                };
                match source {
                    Source::Reliable => {
                        self.reliable_bytes = self.reliable_bytes.saturating_sub(item.bytes);
                    }
                    Source::Render { lane, started_at } => {
                        if let Some(queue) = self.render.get_mut(&lane) {
                            queue.bytes = queue.bytes.saturating_sub(item.bytes);
                            if let Some(observer) = &queue.observer {
                                observer.observe(queue.bytes);
                                observer.record_service(started_at.elapsed());
                            }
                        }
                    }
                }
            }
        }
    }

    fn is_empty(&self) -> bool {
        self.interactive.is_empty()
            && self.current_interactive.is_none()
            && self.reliable.is_empty()
            && self.current.is_none()
            && self.render.values().all(|queue| queue.items.is_empty())
    }

    fn close(&mut self) {
        self.closed = true;
        self.interactive.clear();
        self.current_interactive = None;
        self.reliable.clear();
        self.reliable_bytes = 0;
        self.current = None;
        for queue in self.render.values_mut() {
            queue.items.clear();
            queue.bytes = 0;
            if let Some(observer) = &queue.observer {
                observer.observe(0);
            }
        }
        self.render_ready.clear();
    }
}

struct Shared {
    state: Mutex<State>,
    /// Wakes the writer task; it is the only waiter.
    ready: Notify,
    /// Wakes tasks waiting for queues to drain.
    progress: Notify,
    closed: watch::Sender<bool>,
    next_chunk_stream: AtomicU64,
    next_lane: AtomicU64,
}

impl Shared {
    fn close(&self) {
        self.state.lock().close();
        self.closed.send_replace(true);
        self.ready.notify_one();
        self.progress.notify_waiters();
    }

    /// Splits an encoded control frame into chunk frames when its payload is large.
    fn split(&self, frame: Arc<[u8]>) -> Result<Item, ProtocolCodecError> {
        let payload_len = frame.len().saturating_sub(HEADER_LEN);
        if payload_len <= CHUNK_PAYLOAD_BYTES {
            let bytes = frame.len();
            return Ok(Item {
                frames: VecDeque::from([frame]),
                bytes,
                after_lane: None,
                latest: None,
            });
        }
        let stream = self.next_chunk_stream.fetch_add(1, Ordering::Relaxed);
        let pieces = frame[HEADER_LEN..].chunks(CHUNK_PAYLOAD_BYTES);
        let count = pieces.len();
        let mut frames = VecDeque::with_capacity(count);
        let mut bytes = 0;
        for (index, piece) in pieces.enumerate() {
            let chunk = Arc::<[u8]>::from(encode_message(
                FrameKind::Control,
                &ControlMessage::Chunk(MessageChunk {
                    stream,
                    last: index + 1 == count,
                    bytes: piece.to_vec(),
                }),
            )?);
            bytes += chunk.len();
            frames.push_back(chunk);
        }
        Ok(Item {
            frames,
            bytes,
            after_lane: None,
            latest: None,
        })
    }

    async fn wait_until(&self, done: impl Fn(&State) -> bool) -> Result<(), SessionSendError> {
        loop {
            let notified = self.progress.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            {
                let state = self.state.lock();
                if state.closed {
                    return Err(SessionSendError::Closed);
                }
                if done(&state) {
                    return Ok(());
                }
            }
            notified.await;
        }
    }
}

/// Cloneable handle to a connection's write scheduler.
#[derive(Clone)]
pub struct SessionWriter {
    shared: Arc<Shared>,
}

/// Owns the writer task; dropping it closes the connection's write side.
pub struct SessionWriterTask {
    shared: Arc<Shared>,
    task: JoinHandle<()>,
}

impl Drop for SessionWriterTask {
    fn drop(&mut self) {
        self.shared.close();
        self.task.abort();
    }
}

#[derive(Clone, Copy)]
enum Priority {
    Interactive,
    Reliable,
}

impl SessionWriter {
    pub fn spawn(writer: impl AsyncWrite + Unpin + Send + 'static) -> (Self, SessionWriterTask) {
        let (closed, _) = watch::channel(false);
        let shared = Arc::new(Shared {
            state: Mutex::new(State::default()),
            ready: Notify::new(),
            progress: Notify::new(),
            closed,
            next_chunk_stream: AtomicU64::new(1),
            next_lane: AtomicU64::new(1),
        });
        let task = tokio::spawn(run_writer(shared.clone(), writer));
        (
            Self {
                shared: shared.clone(),
            },
            SessionWriterTask { shared, task },
        )
    }

    /// Queues a message ahead of reliable and render traffic.
    pub fn send_interactive(&self, message: &ControlMessage) -> Result<(), SessionSendError> {
        let item = self.encode(message)?;
        self.push(Priority::Interactive, item)
    }

    /// Queues a message in reliable FIFO order.
    pub fn send_reliable(&self, message: &ControlMessage) -> Result<(), SessionSendError> {
        let item = self.encode(message)?;
        self.push(Priority::Reliable, item)
    }

    /// Queues a reliable message that is written only after every frame
    /// already queued on `lane`, such as a terminal's exit after its final frame.
    pub fn send_reliable_after(
        &self,
        lane: &RenderLane,
        message: &ControlMessage,
    ) -> Result<(), SessionSendError> {
        let mut item = self.encode(message)?;
        item.after_lane = Some(lane.id);
        self.push(Priority::Reliable, item)
    }

    /// Queues a reliable message that supersedes a queued message with the
    /// same `key` whose writing has not started. The newer message takes the
    /// older one's place, so it stays ahead of everything queued after that.
    pub fn send_reliable_latest(
        &self,
        key: &str,
        message: &ControlMessage,
    ) -> Result<(), SessionSendError> {
        let mut item = self.encode(message)?;
        item.latest = Some(Arc::from(key));
        let mut state = self.shared.state.lock();
        if state.closed {
            return Err(SessionSendError::Closed);
        }
        let Some(position) = state
            .reliable
            .iter()
            .position(|queued| queued.latest.as_deref() == Some(key))
        else {
            return self.push_locked(state, Priority::Reliable, item);
        };
        let others = state.reliable_bytes - state.reliable[position].bytes;
        if others != 0 && item.bytes > RELIABLE_BYTES.saturating_sub(others) {
            drop(state);
            self.shared.close();
            return Err(SessionSendError::Overflow);
        }
        state.reliable_bytes = others + item.bytes;
        state.reliable[position] = item;
        Ok(())
    }

    /// Opens a render lane whose frames are written only when no reliable message waits.
    pub fn open_render_lane(&self, observer: Option<Arc<dyn RenderObserver>>) -> RenderLane {
        let id = self.shared.next_lane.fetch_add(1, Ordering::Relaxed);
        self.shared.state.lock().render.insert(
            id,
            RenderQueue {
                items: VecDeque::new(),
                bytes: 0,
                ready: false,
                observer,
            },
        );
        RenderLane {
            shared: self.shared.clone(),
            id,
        }
    }

    /// Waits until every queued message has been written.
    pub async fn flush(&self) -> Result<(), SessionSendError> {
        self.shared.wait_until(State::is_empty).await
    }

    pub fn close(&self) {
        self.shared.close();
    }

    /// Resolves once the connection's write side has closed.
    pub async fn closed(&self) {
        let mut closed = self.shared.closed.subscribe();
        let _ = closed.wait_for(|closed| *closed).await;
    }

    fn encode(&self, message: &ControlMessage) -> Result<Item, SessionSendError> {
        let frame = Arc::<[u8]>::from(encode_message(FrameKind::Control, message)?);
        Ok(self.shared.split(frame)?)
    }

    fn push(&self, priority: Priority, item: Item) -> Result<(), SessionSendError> {
        let state = self.shared.state.lock();
        if state.closed {
            return Err(SessionSendError::Closed);
        }
        self.push_locked(state, priority, item)
    }

    fn push_locked(
        &self,
        mut state: parking_lot::MutexGuard<'_, State>,
        priority: Priority,
        item: Item,
    ) -> Result<(), SessionSendError> {
        let admitted = match priority {
            Priority::Interactive => state.interactive.len() < INTERACTIVE_ITEMS,
            Priority::Reliable => {
                state.reliable.len() < RELIABLE_ITEMS
                    && (state.reliable_bytes == 0
                        || item.bytes <= RELIABLE_BYTES.saturating_sub(state.reliable_bytes))
            }
        };
        if !admitted {
            drop(state);
            self.shared.close();
            return Err(SessionSendError::Overflow);
        }
        match priority {
            Priority::Interactive => state.interactive.push_back(item),
            Priority::Reliable => {
                state.reliable_bytes += item.bytes;
                state.reliable.push_back(item);
            }
        }
        drop(state);
        self.shared.ready.notify_one();
        Ok(())
    }
}

/// A per-terminal render queue on a [`SessionWriter`]. Dropping it discards
/// frames not yet started.
pub struct RenderLane {
    shared: Arc<Shared>,
    id: u64,
}

impl RenderLane {
    /// Admits a pre-encoded control frame while the lane stays within budget.
    pub fn enqueue_frame(&self, frame: Arc<[u8]>) -> Result<(), RenderRejected> {
        let item = self.shared.split(frame).map_err(|_| RenderRejected)?;
        let mut state = self.shared.state.lock();
        if state.closed {
            return Err(RenderRejected);
        }
        let Some(queue) = state.render.get_mut(&self.id) else {
            return Err(RenderRejected);
        };
        let budget = RENDER_LANE_BYTES.max(item.bytes);
        if item.bytes > budget.saturating_sub(queue.bytes) {
            if let Some(observer) = &queue.observer {
                observer.dropped();
            }
            return Err(RenderRejected);
        }
        queue.bytes += item.bytes;
        queue.items.push_back(item);
        if let Some(observer) = &queue.observer {
            observer.observe(queue.bytes);
        }
        let schedule = !queue.ready;
        queue.ready = true;
        if schedule {
            state.render_ready.push_back(self.id);
        }
        drop(state);
        self.shared.ready.notify_one();
        Ok(())
    }

    /// No frame of this lane is queued or being written.
    pub fn is_idle(&self) -> bool {
        self.shared
            .state
            .lock()
            .render
            .get(&self.id)
            .is_none_or(|queue| queue.bytes == 0)
    }

    /// Waits until this lane's frames have been written.
    pub async fn drain(&self) -> Result<(), SessionSendError> {
        let id = self.id;
        self.shared
            .wait_until(|state| state.render.get(&id).is_none_or(|queue| queue.bytes == 0))
            .await
    }
}

impl Drop for RenderLane {
    fn drop(&mut self) {
        let mut state = self.shared.state.lock();
        if let Some(queue) = state.render.remove(&self.id)
            && let Some(observer) = &queue.observer
        {
            observer.observe(0);
        }
        drop(state);
        self.shared.progress.notify_waiters();
    }
}

async fn run_writer(shared: Arc<Shared>, mut writer: impl AsyncWrite + Unpin) {
    loop {
        let notified = shared.ready.notified();
        let next = {
            let mut state = shared.state.lock();
            if state.closed {
                return;
            }
            state.select()
        };
        let Some((frame, slot)) = next else {
            notified.await;
            continue;
        };
        if send_frame(&mut writer, &frame).await.is_err() {
            shared.close();
            return;
        }
        shared.state.lock().finish(slot);
        shared.progress.notify_waiters();
    }
}

#[derive(Default)]
struct ChunkAssembler {
    streams: HashMap<u64, Vec<u8>>,
    buffered: usize,
}

impl ChunkAssembler {
    fn push(&mut self, chunk: MessageChunk) -> Result<Option<Vec<u8>>, WireError> {
        if !self.streams.contains_key(&chunk.stream) && self.streams.len() >= MAX_OPEN_CHUNK_STREAMS
        {
            return Err(WireError::InvalidChunk("too many open chunk streams"));
        }
        let payload = self.streams.entry(chunk.stream).or_default();
        if payload.len() + chunk.bytes.len() > MAX_FRAME_BYTES
            || self.buffered + chunk.bytes.len() > MAX_FRAME_BYTES
        {
            return Err(WireError::InvalidChunk(
                "chunked messages exceed the frame limit",
            ));
        }
        self.buffered += chunk.bytes.len();
        payload.extend_from_slice(&chunk.bytes);
        if !chunk.last {
            return Ok(None);
        }
        let payload = self
            .streams
            .remove(&chunk.stream)
            .expect("chunk stream was just extended");
        self.buffered -= payload.len();
        Ok(Some(payload))
    }
}

/// Reads `Session` connection messages, reassembling chunked ones.
pub struct SessionReader<R> {
    stream: R,
    chunks: ChunkAssembler,
}

impl<R: AsyncRead + Unpin> SessionReader<R> {
    pub fn new(stream: R) -> Self {
        Self {
            stream,
            chunks: ChunkAssembler::default(),
        }
    }

    pub async fn receive(&mut self) -> Result<ControlMessage, WireError> {
        self.receive_observed().await.map(|(message, _)| message)
    }

    /// Skips events added by a newer peer, resolves unrecognized responses as
    /// `FailureCode::Unsupported`, and surfaces unrecognized requests as
    /// `WireError::unsupported_request_id`. Never returns a `Chunk`.
    pub async fn receive_observed(
        &mut self,
    ) -> Result<(ControlMessage, WireReceiveDiagnostics), WireError> {
        let mut payload_read_and_check = Duration::ZERO;
        loop {
            let (frame, read) = receive_frame(
                &mut self.stream,
                FrameKind::Control,
                MAX_FRAME_BYTES + HEADER_LEN,
            )
            .await?;
            payload_read_and_check += read;
            let started_at = Instant::now();
            let (decoded, payload_bytes) = match decode_control_message(&frame) {
                Ok(ControlMessage::Chunk(chunk)) => {
                    let Some(payload) = self.chunks.push(chunk)? else {
                        continue;
                    };
                    let decoded = decode_control_payload(&payload);
                    if matches!(decoded, Ok(ControlMessage::Chunk(_))) {
                        return Err(WireError::InvalidChunk("chunks cannot nest"));
                    }
                    (decoded, payload.len())
                }
                decoded => (decoded, frame.payload.len()),
            };
            let Some(message) = resolve_control(decoded)? else {
                payload_read_and_check = Duration::ZERO;
                continue;
            };
            return Ok((
                message,
                WireReceiveDiagnostics {
                    payload_bytes,
                    payload_read_and_check,
                    message_decode: started_at.elapsed(),
                },
            ));
        }
    }
}

#[cfg(test)]
mod tests;
