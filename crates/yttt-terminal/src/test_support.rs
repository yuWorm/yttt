use crate::event::{GpuiEventProxy, TerminalEvent, TerminalEventMailbox};
use crate::terminal::TerminalState;
use async_channel::Receiver;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

pub(crate) struct TerminalFixture {
    pub terminal: TerminalState,
    events: Arc<TerminalEventMailbox>,
    _signal: Receiver<()>,
}

impl TerminalFixture {
    pub fn new(cols: usize, rows: usize) -> Self {
        let (events, signal) = TerminalEventMailbox::new();
        Self {
            terminal: TerminalState::new(cols, rows, GpuiEventProxy::new(events.clone())),
            events,
            _signal: signal,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.terminal.process_bytes(bytes);
    }

    pub fn drain_events(&self) -> Vec<TerminalEvent> {
        self.events.drain().events
    }
}

#[derive(Debug)]
pub(crate) enum ReadStep {
    Data(Vec<u8>),
    Error(io::ErrorKind),
    Sleep(Duration),
    Eof,
}

pub(crate) struct ScriptedReader {
    steps: VecDeque<ReadStep>,
}

impl ScriptedReader {
    pub fn new(steps: impl IntoIterator<Item = ReadStep>) -> Self {
        Self {
            steps: steps.into_iter().collect(),
        }
    }
}

impl Read for ScriptedReader {
    fn read(&mut self, target: &mut [u8]) -> io::Result<usize> {
        match self.steps.pop_front().unwrap_or(ReadStep::Eof) {
            ReadStep::Data(mut bytes) => {
                let len = bytes.len().min(target.len());
                target[..len].copy_from_slice(&bytes[..len]);
                if len < bytes.len() {
                    bytes.drain(..len);
                    self.steps.push_front(ReadStep::Data(bytes));
                }
                Ok(len)
            }
            ReadStep::Error(kind) => Err(io::Error::from(kind)),
            ReadStep::Sleep(duration) => {
                std::thread::sleep(duration);
                self.read(target)
            }
            ReadStep::Eof => Ok(0),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum WriteStep {
    Accept(usize),
    Interrupted,
    WouldBlock,
    Zero,
    Error(io::ErrorKind),
}

#[derive(Clone, Default)]
pub(crate) struct RecordingWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    steps: Arc<Mutex<VecDeque<WriteStep>>>,
    flushes: Arc<AtomicUsize>,
}

impl RecordingWriter {
    pub fn scripted(steps: impl IntoIterator<Item = WriteStep>) -> Self {
        Self {
            steps: Arc::new(Mutex::new(steps.into_iter().collect())),
            ..Self::default()
        }
    }

    pub fn bytes(&self) -> Vec<u8> {
        self.bytes.lock().clone()
    }

    pub fn flushes(&self) -> usize {
        self.flushes.load(Ordering::Relaxed)
    }
}

impl Write for RecordingWriter {
    fn write(&mut self, input: &[u8]) -> io::Result<usize> {
        match self.steps.lock().pop_front() {
            Some(WriteStep::Interrupted) => Err(io::Error::from(io::ErrorKind::Interrupted)),
            Some(WriteStep::WouldBlock) => Err(io::Error::from(io::ErrorKind::WouldBlock)),
            Some(WriteStep::Zero) => Ok(0),
            Some(WriteStep::Error(kind)) => Err(io::Error::from(kind)),
            Some(WriteStep::Accept(limit)) => {
                let len = limit.min(input.len());
                self.bytes.lock().extend_from_slice(&input[..len]);
                Ok(len)
            }
            None => {
                self.bytes.lock().extend_from_slice(input);
                Ok(input.len())
            }
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        self.flushes.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

#[derive(Clone, Default)]
pub(crate) struct FakeReaperState {
    killed: Arc<AtomicUsize>,
    waited: Arc<AtomicUsize>,
}

impl FakeReaperState {
    pub fn record_kill(&self) {
        self.killed.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_wait(&self) {
        self.waited.fetch_add(1, Ordering::Relaxed);
    }

    pub fn counts(&self) -> (usize, usize) {
        (
            self.killed.load(Ordering::Relaxed),
            self.waited.load(Ordering::Relaxed),
        )
    }
}

#[test]
fn adapter_fixture_collects_terminal_events() {
    let mut fixture = TerminalFixture::new(8, 2);
    fixture.feed(b"\x1b]2;fixture-title\x1b\\");
    assert!(
        fixture
            .drain_events()
            .iter()
            .any(|event| matches!(event, TerminalEvent::Title(title) if title == "fixture-title"))
    );
}
