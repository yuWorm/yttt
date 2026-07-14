use crate::event::{GpuiEventProxy, TerminalEventMailbox};
use crate::perf::{InputPerformanceSample, TerminalPerformanceHandle};

use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::Term;
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use bytes::Bytes;
use parking_lot::{Condvar, Mutex, RwLock};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
#[cfg(any(test, debug_assertions))]
use std::sync::atomic::{AtomicU64, AtomicUsize};

use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

pub const READ_BUFFER_BYTES: usize = u16::MAX as usize;
pub const READ_QUEUE_CAPACITY: usize = 8;
pub const MAX_COMMANDS: usize = 1024;
pub const MAX_USER_COMMANDS: usize = 768;
pub const MAX_WRITE_CHUNK_BYTES: usize = 64 * 1024;
pub const MAX_QUEUED_INPUT_BYTES: usize = 8 * 1024 * 1024;
pub const MAX_QUEUED_REPLY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PtyIoOperation {
    Read,
    Write,
    Resize,
    Mailbox,
    InputQueue,
    Clipboard,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PtyEvent {
    Eof,
    IoError {
        operation: PtyIoOperation,
        message: String,
        fatal: bool,
    },
}

pub type ResizeCallback = Arc<dyn Fn(u16, u16) -> Result<(), String> + Send + Sync>;

#[derive(Debug)]
pub(crate) struct QueuedInput {
    bytes: Bytes,
    performance_sample: Option<InputPerformanceSample>,
}

#[derive(Debug)]
pub(crate) enum PtyCommand {
    WriteInput(QueuedInput),

    WriteReply(Bytes),
    Resize { cols: u16, rows: u16 },
    Shutdown,
}

pub(crate) struct ReadBatch {
    pub buffer: Box<[u8; READ_BUFFER_BYTES]>,
    pub len: usize,
}

pub(crate) enum PtyReadMessage {
    Data(ReadBatch),
    Eof,
    IoError(String),
    Shutdown,
}

#[derive(Default)]
pub(crate) struct PtyDiagnostics {
    #[cfg(any(test, debug_assertions))]
    bytes_read: AtomicU64,
    #[cfg(any(test, debug_assertions))]
    parser_batches: AtomicU64,
    #[cfg(any(test, debug_assertions))]
    read_batches_high_water: AtomicUsize,
    #[cfg(any(test, debug_assertions))]
    queued_input_high_water: AtomicUsize,
    #[cfg(any(test, debug_assertions))]
    queued_reply_high_water: AtomicUsize,
    #[cfg(any(test, debug_assertions))]
    queued_command_high_water: AtomicUsize,
}

#[cfg(any(test, debug_assertions))]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct PtyDiagnosticsSnapshot {
    pub bytes_read: u64,
    pub parser_batches: u64,
    pub read_batches_high_water: usize,
    pub queued_input_high_water: usize,
    pub queued_reply_high_water: usize,
    pub queued_command_high_water: usize,
}

impl PtyDiagnostics {
    #[cfg(any(test, debug_assertions))]
    fn snapshot(&self) -> PtyDiagnosticsSnapshot {
        PtyDiagnosticsSnapshot {
            bytes_read: self.bytes_read.load(Ordering::Relaxed),
            parser_batches: self.parser_batches.load(Ordering::Relaxed),
            read_batches_high_water: self.read_batches_high_water.load(Ordering::Relaxed),
            queued_input_high_water: self.queued_input_high_water.load(Ordering::Relaxed),
            queued_reply_high_water: self.queued_reply_high_water.load(Ordering::Relaxed),
            queued_command_high_water: self.queued_command_high_water.load(Ordering::Relaxed),
        }
    }

    #[inline]
    fn record_read(&self, _bytes: usize) {
        #[cfg(any(test, debug_assertions))]
        self.bytes_read.fetch_add(_bytes as u64, Ordering::Relaxed);
    }

    #[inline]
    fn record_parser_batch(&self) {
        #[cfg(any(test, debug_assertions))]
        self.parser_batches.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    fn record_read_queue_depth(&self, _depth: usize) {
        #[cfg(any(test, debug_assertions))]
        self.read_batches_high_water
            .fetch_max(_depth, Ordering::Relaxed);
    }

    #[inline]
    fn record_input_queue(&self, _bytes: usize, _commands: usize) {
        #[cfg(any(test, debug_assertions))]
        {
            self.queued_input_high_water
                .fetch_max(_bytes, Ordering::Relaxed);
            self.queued_command_high_water
                .fetch_max(_commands, Ordering::Relaxed);
        }
    }

    #[inline]
    fn record_reply_queue(&self, _bytes: usize, _commands: usize) {
        #[cfg(any(test, debug_assertions))]
        {
            self.queued_reply_high_water
                .fetch_max(_bytes, Ordering::Relaxed);
            self.queued_command_high_water
                .fetch_max(_commands, Ordering::Relaxed);
        }
    }

    #[inline]
    fn record_command_queue(&self, _commands: usize) {
        #[cfg(any(test, debug_assertions))]
        self.queued_command_high_water
            .fetch_max(_commands, Ordering::Relaxed);
    }
}

#[derive(Default)]
struct CommandQueueState {
    commands: VecDeque<PtyCommand>,
    input_bytes: usize,
    reply_bytes: usize,
    user_commands: usize,
    closed: bool,
}

pub(crate) struct PtyCommandQueue {
    state: Mutex<CommandQueueState>,
    ready: Condvar,
    diagnostics: Arc<PtyDiagnostics>,
}

impl PtyCommandQueue {
    fn new(diagnostics: Arc<PtyDiagnostics>) -> Self {
        Self {
            state: Mutex::new(CommandQueueState::default()),
            ready: Condvar::new(),
            diagnostics,
        }
    }

    #[cfg(test)]

    pub(crate) fn enqueue_input(&self, bytes: Bytes) -> Result<(), String> {
        self.enqueue_input_sampled(bytes, None)
    }

    fn enqueue_input_sampled(
        &self,
        bytes: Bytes,
        performance_sample: Option<InputPerformanceSample>,
    ) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        let chunks = bytes.len().div_ceil(MAX_WRITE_CHUNK_BYTES);
        let mut state = self.state.lock();
        if state.closed {
            return Err("PTY input queue is closed".to_string());
        }
        if state.input_bytes.saturating_add(bytes.len()) > MAX_QUEUED_INPUT_BYTES
            || state.user_commands.saturating_add(chunks) > MAX_USER_COMMANDS
            || state.commands.len().saturating_add(chunks) > MAX_COMMANDS
        {
            return Err("PTY input queue capacity exceeded".to_string());
        }

        for offset in (0..bytes.len()).step_by(MAX_WRITE_CHUNK_BYTES) {
            let end = (offset + MAX_WRITE_CHUNK_BYTES).min(bytes.len());
            state
                .commands
                .push_back(PtyCommand::WriteInput(QueuedInput {
                    bytes: bytes.slice(offset..end),
                    performance_sample: (end == bytes.len())
                        .then_some(performance_sample)
                        .flatten(),
                }));
        }
        state.input_bytes += bytes.len();
        state.user_commands += chunks;
        self.diagnostics
            .record_input_queue(state.input_bytes, state.commands.len());

        self.ready.notify_one();
        Ok(())
    }

    pub(crate) fn enqueue_reply(&self, bytes: Bytes) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        let chunks = bytes.len().div_ceil(MAX_WRITE_CHUNK_BYTES);
        let mut state = self.state.lock();
        if state.closed {
            return Err("PTY reply queue is closed".to_string());
        }
        if state.reply_bytes.saturating_add(bytes.len()) > MAX_QUEUED_REPLY_BYTES
            || state.commands.len().saturating_add(chunks) > MAX_COMMANDS
        {
            return Err("PTY reply queue capacity exceeded".to_string());
        }

        let mut insert_at = state
            .commands
            .iter()
            .position(|command| matches!(command, PtyCommand::WriteInput(_)))
            .unwrap_or(state.commands.len());
        for offset in (0..bytes.len()).step_by(MAX_WRITE_CHUNK_BYTES) {
            let end = (offset + MAX_WRITE_CHUNK_BYTES).min(bytes.len());
            state
                .commands
                .insert(insert_at, PtyCommand::WriteReply(bytes.slice(offset..end)));
            insert_at += 1;
        }
        state.reply_bytes += bytes.len();
        self.diagnostics
            .record_reply_queue(state.reply_bytes, state.commands.len());

        self.ready.notify_one();
        Ok(())
    }

    pub(crate) fn enqueue_resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        let mut state = self.state.lock();
        if state.closed {
            return Err("PTY command queue is closed".to_string());
        }
        if let Some(PtyCommand::Resize {
            cols: queued_cols,
            rows: queued_rows,
        }) = state.commands.back_mut()
        {
            *queued_cols = cols;
            *queued_rows = rows;
            return Ok(());
        }
        if state.commands.len() >= MAX_COMMANDS {
            return Err("PTY command queue capacity exceeded".to_string());
        }
        state.commands.push_back(PtyCommand::Resize { cols, rows });
        self.diagnostics.record_command_queue(state.commands.len());

        self.ready.notify_one();
        Ok(())
    }

    fn pop(&self) -> PtyCommand {
        let mut state = self.state.lock();
        loop {
            if let Some(command) = state.commands.pop_front() {
                match &command {
                    PtyCommand::WriteInput(input) => {
                        state.input_bytes = state.input_bytes.saturating_sub(input.bytes.len());
                        state.user_commands = state.user_commands.saturating_sub(1);
                    }

                    PtyCommand::WriteReply(bytes) => {
                        state.reply_bytes = state.reply_bytes.saturating_sub(bytes.len());
                    }
                    PtyCommand::Resize { .. } | PtyCommand::Shutdown => {}
                }
                return command;
            }
            self.ready.wait(&mut state);
        }
    }

    fn wait_retry(&self, duration: Duration) -> bool {
        let mut state = self.state.lock();
        self.ready.wait_for(&mut state, duration);
        state.closed
    }

    pub(crate) fn shutdown(&self) {
        let mut state = self.state.lock();
        if state.closed {
            return;
        }
        state.closed = true;
        state.commands.clear();
        state.input_bytes = 0;
        state.reply_bytes = 0;
        state.user_commands = 0;
        state.commands.push_back(PtyCommand::Shutdown);
        self.ready.notify_all();
    }

    #[cfg(test)]
    fn drain_for_test(&self) -> Vec<PtyCommand> {
        let mut state = self.state.lock();
        state.commands.drain(..).collect()
    }
}

#[derive(Clone)]
pub(crate) struct PtyIoHandle {
    queue: Arc<PtyCommandQueue>,
    mailbox: Arc<TerminalEventMailbox>,
    resize_callback: Arc<RwLock<Option<ResizeCallback>>>,
    performance: TerminalPerformanceHandle,
}

impl PtyIoHandle {
    pub(crate) fn write_input(&self, bytes: Bytes) -> Result<(), String> {
        if bytes.is_empty() {
            return Ok(());
        }
        let performance_sample = self.performance.begin_input(&bytes);

        self.queue
            .enqueue_input_sampled(bytes, Some(performance_sample))
            .inspect_err(|message| {
                self.performance.cancel_input(performance_sample);
                self.mailbox.push_pty_event(PtyEvent::IoError {
                    operation: PtyIoOperation::InputQueue,
                    message: message.clone(),
                    fatal: false,
                });
            })
    }

    pub(crate) fn write_reply(&self, bytes: Bytes) -> Result<(), String> {
        self.queue.enqueue_reply(bytes).inspect_err(|message| {
            self.mailbox.push_pty_event(PtyEvent::IoError {
                operation: PtyIoOperation::Write,
                message: message.clone(),
                fatal: true,
            });
        })
    }

    pub(crate) fn resize(&self, cols: u16, rows: u16) -> Result<(), String> {
        self.queue
            .enqueue_resize(cols, rows)
            .inspect_err(|message| {
                self.mailbox.push_pty_event(PtyEvent::IoError {
                    operation: PtyIoOperation::Resize,
                    message: message.clone(),
                    fatal: false,
                });
            })
    }

    pub(crate) fn set_resize_callback(&self, callback: ResizeCallback) {
        *self.resize_callback.write() = Some(callback);
    }

    pub(crate) fn shutdown(&self) {
        self.queue.shutdown();
    }
}

pub(crate) struct PtyIoDriver {
    handle: PtyIoHandle,
    cancelled: Arc<AtomicBool>,
    read_tx: flume::Sender<PtyReadMessage>,
    #[cfg(any(test, debug_assertions))]
    diagnostics: Arc<PtyDiagnostics>,
    _threads: Vec<JoinHandle<()>>,
}

impl PtyIoDriver {
    #[cfg(test)]

    pub(crate) fn start<W, R>(
        writer: W,
        reader: R,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        mailbox: Arc<TerminalEventMailbox>,
    ) -> Self
    where
        W: Write + Send + 'static,
        R: Read + Send + 'static,
    {
        Self::start_with_performance(
            writer,
            reader,
            term,
            mailbox,
            TerminalPerformanceHandle::new(),
        )
    }

    pub(crate) fn start_with_performance<W, R>(
        writer: W,
        reader: R,
        term: Arc<FairMutex<Term<GpuiEventProxy>>>,
        mailbox: Arc<TerminalEventMailbox>,
        performance: TerminalPerformanceHandle,
    ) -> Self
    where
        W: Write + Send + 'static,
        R: Read + Send + 'static,
    {
        let diagnostics = Arc::new(PtyDiagnostics::default());
        let queue = Arc::new(PtyCommandQueue::new(diagnostics.clone()));
        let resize_callback = Arc::new(RwLock::new(None));
        let handle = PtyIoHandle {
            queue: queue.clone(),
            mailbox: mailbox.clone(),
            resize_callback: resize_callback.clone(),
            performance: performance.clone(),
        };
        let cancelled = Arc::new(AtomicBool::new(false));
        let (read_tx, read_rx) = flume::bounded(READ_QUEUE_CAPACITY);
        let (buffer_tx, buffer_rx) = flume::bounded(READ_QUEUE_CAPACITY + 1);
        for _ in 0..=READ_QUEUE_CAPACITY {
            let buffer: Box<[u8; READ_BUFFER_BYTES]> = vec![0; READ_BUFFER_BYTES]
                .into_boxed_slice()
                .try_into()
                .expect("fixed PTY read buffer length");
            buffer_tx
                .send(buffer)
                .expect("buffer pool receiver must exist during setup");
        }

        let reader_thread = spawn_reader(
            reader,
            read_tx.clone(),
            buffer_rx,
            buffer_tx.clone(),
            cancelled.clone(),
            diagnostics.clone(),
            performance.clone(),
        );
        let parser_thread = spawn_parser(
            term,
            read_rx,
            buffer_tx,
            cancelled.clone(),
            mailbox.clone(),
            diagnostics.clone(),
            performance.clone(),
        );
        let writer_thread = spawn_writer(writer, queue, resize_callback, mailbox, performance);

        Self {
            handle,
            cancelled,
            read_tx,
            #[cfg(any(test, debug_assertions))]
            diagnostics,
            _threads: vec![reader_thread, parser_thread, writer_thread],
        }
    }

    pub(crate) fn handle(&self) -> PtyIoHandle {
        self.handle.clone()
    }

    #[cfg(any(test, debug_assertions))]

    pub(crate) fn diagnostics(&self) -> PtyDiagnosticsSnapshot {
        self.diagnostics.snapshot()
    }

    pub(crate) fn shutdown(&self) {
        self.cancelled.store(true, Ordering::Release);
        self.handle.shutdown();
        let _ = self.read_tx.try_send(PtyReadMessage::Shutdown);
    }
}

impl Drop for PtyIoDriver {
    fn drop(&mut self) {
        self.shutdown();
    }
}

fn spawn_reader<R: Read + Send + 'static>(
    mut reader: R,
    read_tx: flume::Sender<PtyReadMessage>,
    buffer_rx: flume::Receiver<Box<[u8; READ_BUFFER_BYTES]>>,
    buffer_tx: flume::Sender<Box<[u8; READ_BUFFER_BYTES]>>,
    cancelled: Arc<AtomicBool>,
    diagnostics: Arc<PtyDiagnostics>,
    performance: TerminalPerformanceHandle,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("yttt-pty-reader".to_string())
        .spawn(move || {
            while !cancelled.load(Ordering::Acquire) {
                let Ok(mut buffer) = buffer_rx.recv() else {
                    break;
                };
                let result = reader.read(buffer.as_mut_slice());
                if cancelled.load(Ordering::Acquire) {
                    let _ = buffer_tx.send(buffer);
                    break;
                }
                match result {
                    Ok(0) => {
                        let _ = read_tx.send(PtyReadMessage::Eof);
                        break;
                    }
                    Ok(len) => {
                        diagnostics.record_read(len);

                        performance.record_read(len);

                        if read_tx
                            .send(PtyReadMessage::Data(ReadBatch { buffer, len }))
                            .is_err()
                        {
                            break;
                        }
                        let depth = read_tx.len();
                        diagnostics.record_read_queue_depth(depth);

                        performance.set_read_queue_depth(depth);
                    }
                    Err(error) if error.kind() == io::ErrorKind::Interrupted => {
                        if buffer_tx.send(buffer).is_err() {
                            break;
                        }
                    }
                    Err(error) => {
                        let _ = read_tx.send(PtyReadMessage::IoError(error.to_string()));
                        break;
                    }
                }
            }
        })
        .expect("failed to spawn PTY reader")
}

fn spawn_parser(
    term: Arc<FairMutex<Term<GpuiEventProxy>>>,
    read_rx: flume::Receiver<PtyReadMessage>,
    buffer_tx: flume::Sender<Box<[u8; READ_BUFFER_BYTES]>>,
    cancelled: Arc<AtomicBool>,
    mailbox: Arc<TerminalEventMailbox>,
    diagnostics: Arc<PtyDiagnostics>,
    performance: TerminalPerformanceHandle,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("yttt-pty-parser".to_string())
        .spawn(move || {
            let mut processor: Processor<StdSyncHandler> = Processor::new();
            loop {
                if cancelled.load(Ordering::Acquire) {
                    break;
                }
                let message = if let Some(deadline) = processor.sync_timeout().sync_timeout() {
                    read_rx.recv_timeout(deadline.saturating_duration_since(Instant::now()))
                } else {
                    read_rx
                        .recv()
                        .map_err(|_| flume::RecvTimeoutError::Disconnected)
                };
                performance.set_read_queue_depth(read_rx.len());

                if cancelled.load(Ordering::Acquire) {
                    if let Ok(PtyReadMessage::Data(batch)) = message {
                        let _ = buffer_tx.send(batch.buffer);
                    }
                    break;
                }
                match message {
                    Ok(PtyReadMessage::Data(batch)) => {
                        diagnostics.record_parser_batch();

                        let batch_started = Instant::now();
                        let lock_started = Instant::now();
                        {
                            let mut term = term.lock();
                            let lock_wait = lock_started.elapsed();
                            let advance_started = Instant::now();
                            processor.advance(&mut *term, &batch.buffer[..batch.len]);
                            let advance = advance_started.elapsed();
                            let completed_at = Instant::now();
                            performance.record_parser_batch(
                                batch_started.elapsed(),
                                lock_wait,
                                advance,
                                completed_at,
                                &batch.buffer[..batch.len],
                            );
                        }
                        // Match Alacritty's event loop: unsynchronized parser output must wake
                        // the UI. Waiting only for a synchronized-update timeout starves redraws
                        // indefinitely while a TUI continuously fills the read queue.
                        if batch.len > 0 && processor.sync_bytes_count() < batch.len {
                            mailbox.request_redraw();
                        }
                        let _ = buffer_tx.send(batch.buffer);
                    }

                    Ok(PtyReadMessage::Eof) => {
                        mailbox.push_pty_event(PtyEvent::Eof);
                        break;
                    }
                    Ok(PtyReadMessage::IoError(message)) => {
                        mailbox.push_pty_event(PtyEvent::IoError {
                            operation: PtyIoOperation::Read,
                            message,
                            fatal: true,
                        });
                        break;
                    }
                    Ok(PtyReadMessage::Shutdown) | Err(flume::RecvTimeoutError::Disconnected) => {
                        break;
                    }
                    Err(flume::RecvTimeoutError::Timeout) => {
                        let mut term = term.lock();
                        processor.stop_sync(&mut *term);
                        drop(term);
                        mailbox.request_redraw();
                    }
                }
            }
        })
        .expect("failed to spawn PTY parser")
}

fn spawn_writer<W: Write + Send + 'static>(
    mut writer: W,
    queue: Arc<PtyCommandQueue>,
    resize_callback: Arc<RwLock<Option<ResizeCallback>>>,
    mailbox: Arc<TerminalEventMailbox>,
    performance: TerminalPerformanceHandle,
) -> JoinHandle<()> {
    thread::Builder::new()
        .name("yttt-pty-writer".to_string())
        .spawn(move || {
            loop {
                match queue.pop() {
                    PtyCommand::WriteInput(input) => {
                        if !write_pty_bytes(&mut writer, &input.bytes, &queue, &mailbox) {
                            return;
                        }
                        if let Some(sample) = input.performance_sample {
                            performance.record_input_written(sample, Instant::now());
                        }
                    }
                    PtyCommand::WriteReply(bytes) => {
                        if !write_pty_bytes(&mut writer, &bytes, &queue, &mailbox) {
                            return;
                        }
                    }

                    PtyCommand::Resize { cols, rows } => {
                        let callback = resize_callback.read().clone();
                        if let Some(callback) = callback
                            && let Err(message) = callback(cols, rows)
                        {
                            mailbox.push_pty_event(PtyEvent::IoError {
                                operation: PtyIoOperation::Resize,
                                message,
                                fatal: false,
                            });
                        }
                    }
                    PtyCommand::Shutdown => {
                        let _ = writer.flush();
                        return;
                    }
                }
            }
        })
        .expect("failed to spawn PTY writer")
}
fn write_pty_bytes<W: Write>(
    writer: &mut W,
    bytes: &Bytes,
    queue: &PtyCommandQueue,
    mailbox: &TerminalEventMailbox,
) -> bool {
    let mut offset = 0;
    let mut backoff = Duration::from_millis(1);
    while offset < bytes.len() {
        match writer.write(&bytes[offset..]) {
            Ok(0) => {
                if queue.wait_retry(backoff) {
                    return false;
                }
                backoff = (backoff * 2).min(Duration::from_millis(16));
            }
            Ok(count) => {
                offset += count;
                backoff = Duration::from_millis(1);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if queue.wait_retry(backoff) {
                    return false;
                }
                backoff = (backoff * 2).min(Duration::from_millis(16));
            }
            Err(error) => {
                mailbox.push_pty_event(PtyEvent::IoError {
                    operation: PtyIoOperation::Write,
                    message: error.to_string(),
                    fatal: true,
                });
                queue.shutdown();
                return false;
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::TerminalEvent;
    use crate::terminal::TerminalState;
    use crate::test_support::{ReadStep, RecordingWriter, ScriptedReader, WriteStep};
    use alacritty_terminal::index::{Column, Line};

    fn wait_for_events(
        mailbox: &TerminalEventMailbox,
        predicate: impl Fn(&[TerminalEvent]) -> bool,
    ) -> Vec<TerminalEvent> {
        let deadline = Instant::now() + Duration::from_secs(2);
        let mut events = Vec::new();
        loop {
            events.extend(mailbox.drain().events);
            if predicate(&events) {
                return events;
            }
            assert!(
                Instant::now() < deadline,
                "timed out waiting for PTY event: {events:?}"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    fn queue() -> Arc<PtyCommandQueue> {
        Arc::new(PtyCommandQueue::new(Arc::new(PtyDiagnostics::default())))
    }

    #[test]
    fn pty_driver_bounds_input_queue() {
        let queue = queue();
        let oversized = Bytes::from(vec![0; MAX_QUEUED_INPUT_BYTES + 1]);
        assert!(queue.enqueue_input(oversized).is_err());
        assert!(queue.drain_for_test().is_empty());
    }

    #[test]
    fn pty_driver_prioritizes_terminal_replies() {
        let queue = queue();
        queue.enqueue_resize(10, 10).unwrap();
        queue.enqueue_input(Bytes::from_static(b"input")).unwrap();
        queue.enqueue_reply(Bytes::from_static(b"reply")).unwrap();
        let commands = queue.drain_for_test();
        assert!(matches!(commands[0], PtyCommand::Resize { .. }));
        assert!(
            matches!(&commands[1], PtyCommand::WriteReply(bytes) if bytes.as_ref() == b"reply")
        );
        assert!(
            matches!(&commands[2], PtyCommand::WriteInput(input) if input.bytes.as_ref() == b"input")
        );
    }

    #[test]
    fn pty_driver_coalesces_adjacent_resize() {
        let queue = queue();
        queue.enqueue_resize(10, 10).unwrap();
        queue.enqueue_resize(20, 20).unwrap();
        queue.enqueue_input(Bytes::from_static(b"x")).unwrap();
        queue.enqueue_resize(30, 30).unwrap();
        let commands = queue.drain_for_test();
        assert_eq!(commands.len(), 3);
        assert!(matches!(
            commands[0],
            PtyCommand::Resize { cols: 20, rows: 20 }
        ));
        assert!(matches!(
            commands[2],
            PtyCommand::Resize { cols: 30, rows: 30 }
        ));
    }

    #[test]
    fn pty_driver_preserves_partial_write_order() {
        let writer = RecordingWriter::scripted([
            WriteStep::Interrupted,
            WriteStep::Accept(1),
            WriteStep::WouldBlock,
            WriteStep::Zero,
            WriteStep::Accept(usize::MAX),
        ]);
        let recorded = writer.clone();
        let queue = queue();
        let mailbox = TerminalEventMailbox::new().0;
        let thread = spawn_writer(
            writer,
            queue.clone(),
            Arc::new(RwLock::new(None)),
            mailbox,
            TerminalPerformanceHandle::new(),
        );
        queue.enqueue_input(Bytes::from_static(b"abcdef")).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.bytes() != b"abcdef" && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        queue.shutdown();
        thread.join().unwrap();
        assert_eq!(recorded.bytes(), b"abcdef");
    }

    #[test]
    fn pty_driver_reports_eof_exactly_once() {
        let (mailbox, _) = TerminalEventMailbox::new();
        let state = TerminalState::new(8, 2, GpuiEventProxy::new(mailbox.clone()));
        let driver = PtyIoDriver::start(
            RecordingWriter::default(),
            ScriptedReader::new([ReadStep::Eof]),
            state.term_arc(),
            mailbox.clone(),
        );
        let events = wait_for_events(&mailbox, |events| {
            events
                .iter()
                .any(|event| matches!(event, TerminalEvent::Pty(PtyEvent::Eof)))
        });
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(event, TerminalEvent::Pty(PtyEvent::Eof)))
                .count(),
            1
        );
        thread::sleep(Duration::from_millis(10));
        assert!(
            !mailbox
                .drain()
                .events
                .iter()
                .any(|event| matches!(event, TerminalEvent::Pty(PtyEvent::Eof)))
        );
        driver.shutdown();
    }

    #[test]
    fn pty_driver_reports_read_errors_exactly_once() {
        let (mailbox, _) = TerminalEventMailbox::new();
        let state = TerminalState::new(8, 2, GpuiEventProxy::new(mailbox.clone()));
        let driver = PtyIoDriver::start(
            RecordingWriter::default(),
            ScriptedReader::new([ReadStep::Error(io::ErrorKind::BrokenPipe)]),
            state.term_arc(),
            mailbox.clone(),
        );
        let events = wait_for_events(&mailbox, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    TerminalEvent::Pty(PtyEvent::IoError {
                        operation: PtyIoOperation::Read,
                        fatal: true,
                        ..
                    })
                )
            })
        });
        assert_eq!(
            events
                .iter()
                .filter(|event| matches!(
                    event,
                    TerminalEvent::Pty(PtyEvent::IoError {
                        operation: PtyIoOperation::Read,
                        ..
                    })
                ))
                .count(),
            1
        );
        driver.shutdown();
    }

    #[test]
    fn pty_driver_parses_bounded_read_batches() {
        let (mailbox, _) = TerminalEventMailbox::new();
        let state = TerminalState::new(8, 2, GpuiEventProxy::new(mailbox.clone()));
        let driver = PtyIoDriver::start(
            RecordingWriter::default(),
            ScriptedReader::new([
                ReadStep::Data(b"hello".to_vec()),
                ReadStep::Data(b" world".to_vec()),
                ReadStep::Eof,
            ]),
            state.term_arc(),
            mailbox.clone(),
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while driver.diagnostics().parser_batches < 2 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            state.with_term(|term| term.grid()[Line(0)][Column(0)].c),
            'h'
        );
        let diagnostics = driver.diagnostics();
        assert_eq!(diagnostics.bytes_read, 11);
        assert_eq!(diagnostics.parser_batches, 2);
        assert!(diagnostics.read_batches_high_water <= READ_QUEUE_CAPACITY);
        let redraw_deadline = Instant::now() + Duration::from_secs(2);
        let mut redraw_requested = false;
        while !redraw_requested && Instant::now() < redraw_deadline {
            redraw_requested = mailbox.drain().redraw;
            if !redraw_requested {
                thread::sleep(Duration::from_millis(2));
            }
        }
        assert!(
            redraw_requested,
            "unsynchronized parser output must request a redraw"
        );
        driver.shutdown();
    }

    #[test]
    fn pty_driver_flushes_synchronized_updates_on_timeout() {
        let (mailbox, _) = TerminalEventMailbox::new();
        let state = TerminalState::new(8, 2, GpuiEventProxy::new(mailbox.clone()));
        let driver = PtyIoDriver::start(
            RecordingWriter::default(),
            ScriptedReader::new([
                ReadStep::Data(b"\x1b[?2026hhello".to_vec()),
                ReadStep::Sleep(Duration::from_millis(300)),
                ReadStep::Eof,
            ]),
            state.term_arc(),
            mailbox,
        );
        let deadline = Instant::now() + Duration::from_secs(2);
        while state.with_term(|term| term.grid()[Line(0)][Column(0)].c) != 'h'
            && Instant::now() < deadline
        {
            thread::sleep(Duration::from_millis(2));
        }
        assert_eq!(
            state.with_term(|term| term.grid()[Line(0)][Column(0)].c),
            'h'
        );
        driver.shutdown();
    }

    #[test]
    fn pty_driver_reports_write_errors_as_fatal() {
        let (mailbox, _) = TerminalEventMailbox::new();
        let state = TerminalState::new(8, 2, GpuiEventProxy::new(mailbox.clone()));
        let driver = PtyIoDriver::start(
            RecordingWriter::scripted([WriteStep::Error(io::ErrorKind::BrokenPipe)]),
            ScriptedReader::new([ReadStep::Sleep(Duration::from_millis(200)), ReadStep::Eof]),
            state.term_arc(),
            mailbox.clone(),
        );
        driver
            .handle()
            .write_input(Bytes::from_static(b"x"))
            .unwrap();
        wait_for_events(&mailbox, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    TerminalEvent::Pty(PtyEvent::IoError {
                        operation: PtyIoOperation::Write,
                        fatal: true,
                        ..
                    })
                )
            })
        });
        driver.shutdown();
    }

    #[test]
    fn pty_driver_runs_resize_callback_on_writer_worker_and_keeps_errors_nonfatal() {
        let (mailbox, _) = TerminalEventMailbox::new();
        let state = TerminalState::new(8, 2, GpuiEventProxy::new(mailbox.clone()));
        let driver = PtyIoDriver::start(
            RecordingWriter::default(),
            ScriptedReader::new([ReadStep::Sleep(Duration::from_millis(200)), ReadStep::Eof]),
            state.term_arc(),
            mailbox.clone(),
        );
        let (thread_tx, thread_rx) = std::sync::mpsc::channel();
        driver.handle().set_resize_callback(Arc::new(move |_, _| {
            thread_tx
                .send(thread::current().name().unwrap_or_default().to_string())
                .unwrap();
            Err("resize failed".to_string())
        }));
        driver.handle().resize(10, 4).unwrap();
        assert_eq!(
            thread_rx.recv_timeout(Duration::from_secs(1)).unwrap(),
            "yttt-pty-writer"
        );
        let events = wait_for_events(&mailbox, |events| {
            events.iter().any(|event| {
                matches!(
                    event,
                    TerminalEvent::Pty(PtyEvent::IoError {
                        operation: PtyIoOperation::Resize,
                        ..
                    })
                )
            })
        });
        assert!(events.iter().any(|event| {
            matches!(
                event,
                TerminalEvent::Pty(PtyEvent::IoError {
                    operation: PtyIoOperation::Resize,
                    fatal: false,
                    ..
                })
            )
        }));
        driver.shutdown();
    }

    #[test]
    fn pty_driver_shutdown_flushes_writer() {
        let (mailbox, _) = TerminalEventMailbox::new();
        let state = TerminalState::new(8, 2, GpuiEventProxy::new(mailbox.clone()));
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let driver = PtyIoDriver::start(
            writer,
            ScriptedReader::new([ReadStep::Sleep(Duration::from_millis(100)), ReadStep::Eof]),
            state.term_arc(),
            mailbox,
        );
        driver.shutdown();
        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.flushes() == 0 && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorded.flushes(), 1);
    }
}
