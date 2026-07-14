//! Bounded bridge from Alacritty terminal events into GPUI.

use crate::perf::TerminalPerformanceHandle;

use crate::pty::{PtyEvent, PtyIoOperation};
use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::term::ClipboardType;
use alacritty_terminal::vte::ansi::Rgb;
use parking_lot::Mutex;
use std::collections::VecDeque;
use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU16, AtomicU64, Ordering};

pub const TERMINAL_EVENT_CAPACITY: usize = 256;
pub const MAX_TITLE_BYTES: usize = 4096;
pub const MAX_OSC52_BYTES: usize = 1024 * 1024;

pub type ClipboardFormatter = Arc<dyn Fn(&str) -> String + Send + Sync>;
pub type ColorFormatter = Arc<dyn Fn(Rgb) -> String + Send + Sync>;
pub type SizeFormatter = Arc<dyn Fn(WindowSize) -> String + Send + Sync>;

pub enum TerminalEvent {
    ClipboardStore(ClipboardType, String),
    ClipboardStoreRejected,
    ClipboardLoad(ClipboardType, ClipboardFormatter),
    ColorRequest(usize, ColorFormatter),
    TextAreaSizeRequest(SizeFormatter),
    PtyWrite(String),
    Pty(PtyEvent),
    MouseCursorDirty,
    CursorBlinkingChange,
    Title(String),
    ResetTitle,
    Bell,
    Wakeup,
    Exit,
    ChildExit(i32),
}

impl fmt::Debug for TerminalEvent {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ClipboardStore(target, _) => formatter
                .debug_tuple("ClipboardStore")
                .field(target)
                .field(&"<redacted>")
                .finish(),
            Self::ClipboardStoreRejected => formatter.write_str("ClipboardStoreRejected"),
            Self::ClipboardLoad(target, _) => formatter
                .debug_tuple("ClipboardLoad")
                .field(target)
                .field(&"<formatter>")
                .finish(),
            Self::ColorRequest(index, _) => formatter
                .debug_tuple("ColorRequest")
                .field(index)
                .field(&"<formatter>")
                .finish(),
            Self::TextAreaSizeRequest(_) => formatter.write_str("TextAreaSizeRequest(<formatter>)"),
            Self::PtyWrite(_) => formatter.write_str("PtyWrite(<redacted>)"),
            Self::Pty(_) => formatter.write_str("Pty(<event>)"),
            Self::MouseCursorDirty => formatter.write_str("MouseCursorDirty"),
            Self::CursorBlinkingChange => formatter.write_str("CursorBlinkingChange"),
            Self::Title(_) => formatter.write_str("Title(<redacted>)"),
            Self::ResetTitle => formatter.write_str("ResetTitle"),
            Self::Bell => formatter.write_str("Bell"),
            Self::Wakeup => formatter.write_str("Wakeup"),
            Self::Exit => formatter.write_str("Exit"),
            Self::ChildExit(_) => formatter.write_str("ChildExit(<status>)"),
        }
    }
}

#[derive(Default)]
struct GenerationGate {
    generation: AtomicU64,
    pending: AtomicBool,
}

impl GenerationGate {
    fn request(&self, signal: &async_channel::Sender<()>) -> bool {
        self.generation.fetch_add(1, Ordering::AcqRel);
        if !self.pending.swap(true, Ordering::AcqRel) {
            let _ = signal.try_send(());
            true
        } else {
            false
        }
    }

    fn pending(&self) -> bool {
        self.pending.load(Ordering::Acquire)
    }

    fn clear(&self, signal: &async_channel::Sender<()>) -> bool {
        let observed = self.generation.load(Ordering::Acquire);
        self.pending.store(false, Ordering::Release);
        if self.generation.load(Ordering::Acquire) != observed
            && !self.pending.swap(true, Ordering::AcqRel)
        {
            let _ = signal.try_send(());
            true
        } else {
            false
        }
    }
}

pub(crate) struct MailboxDrain {
    pub events: Vec<TerminalEvent>,
    pub redraw: bool,
    pub bells: u16,
}

pub struct TerminalEventMailbox {
    queue: Mutex<VecDeque<TerminalEvent>>,
    fatal: Mutex<Option<PtyEvent>>,
    bells: AtomicU16,
    event_gate: GenerationGate,
    redraw_gate: GenerationGate,
    failed: AtomicBool,
    signal: async_channel::Sender<()>,
    #[cfg(any(test, debug_assertions))]
    gpui_wakeups: AtomicU64,
    performance: TerminalPerformanceHandle,
}

impl TerminalEventMailbox {
    pub fn new() -> (Arc<Self>, async_channel::Receiver<()>) {
        Self::new_with_performance(TerminalPerformanceHandle::new())
    }

    pub(crate) fn new_with_performance(
        performance: TerminalPerformanceHandle,
    ) -> (Arc<Self>, async_channel::Receiver<()>) {
        let (signal, receiver) = async_channel::bounded(1);
        (
            Arc::new(Self {
                queue: Mutex::new(VecDeque::with_capacity(TERMINAL_EVENT_CAPACITY)),
                fatal: Mutex::new(None),
                bells: AtomicU16::new(0),
                event_gate: GenerationGate::default(),
                redraw_gate: GenerationGate::default(),
                failed: AtomicBool::new(false),
                signal,
                #[cfg(any(test, debug_assertions))]
                gpui_wakeups: AtomicU64::new(0),
                performance,
            }),
            receiver,
        )
    }

    pub fn request_redraw(&self) {
        let signaled = self.redraw_gate.request(&self.signal);
        if signaled {
            self.record_gpui_wakeup();
        }
        self.performance.record_redraw_request(signaled);
    }

    pub(crate) fn redraw_pending(&self) -> bool {
        self.redraw_gate.pending()
    }

    pub(crate) fn clear_redraw(&self) {
        if self.redraw_gate.clear(&self.signal) {
            self.record_gpui_wakeup();
            self.performance.record_redraw_signal();
        }
    }

    pub(crate) fn push_pty_event(&self, event: PtyEvent) {
        self.push(TerminalEvent::Pty(event));
    }

    pub(crate) fn push(&self, mut event: TerminalEvent) {
        if self.failed.load(Ordering::Acquire) {
            return;
        }
        match event {
            TerminalEvent::Wakeup => {
                self.request_redraw();
                return;
            }
            TerminalEvent::Bell => {
                let _ = self
                    .bells
                    .fetch_update(Ordering::AcqRel, Ordering::Acquire, |bells| {
                        Some(bells.saturating_add(1))
                    });
                if self.event_gate.request(&self.signal) {
                    self.record_gpui_wakeup();
                }

                return;
            }
            TerminalEvent::Title(ref mut title) => {
                truncate_utf8(title, MAX_TITLE_BYTES);
            }
            _ => {}
        }

        let required = matches!(
            event,
            TerminalEvent::ClipboardStoreRejected
                | TerminalEvent::ClipboardLoad(..)
                | TerminalEvent::ColorRequest(..)
                | TerminalEvent::TextAreaSizeRequest(_)
                | TerminalEvent::PtyWrite(_)
                | TerminalEvent::Exit
                | TerminalEvent::ChildExit(_)
                | TerminalEvent::Pty(_)
        );
        let mut queue = self.queue.lock();
        match &event {
            TerminalEvent::Title(_) | TerminalEvent::ResetTitle => {
                queue.retain(|queued| {
                    !matches!(queued, TerminalEvent::Title(_) | TerminalEvent::ResetTitle)
                });
            }
            TerminalEvent::CursorBlinkingChange => {
                if queue
                    .iter()
                    .any(|queued| matches!(queued, TerminalEvent::CursorBlinkingChange))
                {
                    return;
                }
            }
            TerminalEvent::MouseCursorDirty => {
                if queue
                    .iter()
                    .any(|queued| matches!(queued, TerminalEvent::MouseCursorDirty))
                {
                    return;
                }
            }
            _ => {}
        }

        if queue.len() >= TERMINAL_EVENT_CAPACITY {
            if required {
                queue.clear();
                self.failed.store(true, Ordering::Release);
                *self.fatal.lock() = Some(PtyEvent::IoError {
                    operation: PtyIoOperation::Mailbox,
                    message: "terminal event mailbox capacity exceeded".to_string(),
                    fatal: true,
                });
            } else {
                return;
            }
        } else {
            queue.push_back(event);
        }
        drop(queue);
        if self.event_gate.request(&self.signal) {
            self.record_gpui_wakeup();
        }
    }

    pub(crate) fn drain(&self) -> MailboxDrain {
        let mut events = {
            let mut queue = self.queue.lock();
            queue.drain(..).collect::<Vec<_>>()
        };
        if let Some(fatal) = self.fatal.lock().take() {
            events.push(TerminalEvent::Pty(fatal));
        }
        let bells = self.bells.swap(0, Ordering::AcqRel);
        let redraw = self.redraw_gate.pending();
        if self.event_gate.clear(&self.signal) {
            self.record_gpui_wakeup();
        }

        MailboxDrain {
            events,
            redraw,
            bells,
        }
    }

    #[inline]
    fn record_gpui_wakeup(&self) {
        #[cfg(any(test, debug_assertions))]
        self.gpui_wakeups.fetch_add(1, Ordering::Relaxed);
    }

    #[cfg(any(test, debug_assertions))]

    pub(crate) fn gpui_wakeups(&self) -> u64 {
        self.gpui_wakeups.load(Ordering::Relaxed)
    }

    #[cfg(test)]
    fn queue_len(&self) -> usize {
        self.queue.lock().len()
    }
}

fn truncate_utf8(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
}

#[derive(Clone)]
pub struct GpuiEventProxy {
    mailbox: Arc<TerminalEventMailbox>,
}

impl GpuiEventProxy {
    pub fn new(mailbox: Arc<TerminalEventMailbox>) -> Self {
        Self { mailbox }
    }

    pub fn mailbox(&self) -> &Arc<TerminalEventMailbox> {
        &self.mailbox
    }
}

impl EventListener for GpuiEventProxy {
    fn send_event(&self, event: Event) {
        let event = match event {
            Event::Wakeup => TerminalEvent::Wakeup,
            Event::Bell => TerminalEvent::Bell,
            Event::Title(title) => TerminalEvent::Title(title),
            Event::ResetTitle => TerminalEvent::ResetTitle,
            Event::ClipboardStore(target, text) => {
                if text.len() > MAX_OSC52_BYTES {
                    TerminalEvent::ClipboardStoreRejected
                } else {
                    TerminalEvent::ClipboardStore(target, text)
                }
            }
            Event::ClipboardLoad(target, formatter) => {
                TerminalEvent::ClipboardLoad(target, formatter)
            }
            Event::ColorRequest(index, formatter) => TerminalEvent::ColorRequest(index, formatter),
            Event::PtyWrite(data) => TerminalEvent::PtyWrite(data),
            Event::TextAreaSizeRequest(formatter) => TerminalEvent::TextAreaSizeRequest(formatter),
            Event::MouseCursorDirty => TerminalEvent::MouseCursorDirty,
            Event::CursorBlinkingChange => TerminalEvent::CursorBlinkingChange,
            Event::Exit => TerminalEvent::Exit,
            Event::ChildExit(code) => TerminalEvent::ChildExit(code),
        };
        self.mailbox.push(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::EventListener;

    fn event_proxy() -> (GpuiEventProxy, Arc<TerminalEventMailbox>) {
        let (mailbox, _) = TerminalEventMailbox::new();
        (GpuiEventProxy::new(mailbox.clone()), mailbox)
    }

    #[test]
    fn terminal_event_mailbox_is_bounded_and_preserves_replies() {
        let (proxy, mailbox) = event_proxy();
        for index in 0..TERMINAL_EVENT_CAPACITY * 2 {
            proxy.send_event(Event::Title(format!("title-{index}")));
            proxy.send_event(Event::MouseCursorDirty);
        }
        proxy.send_event(Event::PtyWrite("reply".to_string()));
        assert!(mailbox.queue_len() <= TERMINAL_EVENT_CAPACITY);
        let drained = mailbox.drain();
        assert!(
            drained
                .events
                .iter()
                .any(|event| matches!(event, TerminalEvent::PtyWrite(data) if data == "reply"))
        );
    }

    #[test]
    fn terminal_event_mailbox_coalesces_wakeup_and_title() {
        let (proxy, mailbox) = event_proxy();
        for _ in 0..100 {
            proxy.send_event(Event::Wakeup);
        }
        proxy.send_event(Event::Title("one".to_string()));
        proxy.send_event(Event::Title("two".to_string()));
        let drained = mailbox.drain();
        assert!(drained.redraw);
        assert_eq!(
            drained
                .events
                .iter()
                .filter(|event| matches!(event, TerminalEvent::Title(_)))
                .count(),
            1
        );
        assert!(mailbox.gpui_wakeups() <= 2);
    }

    #[test]
    fn required_mailbox_overflow_becomes_one_fatal_event() {
        let (_, mailbox) = event_proxy();
        for _ in 0..TERMINAL_EVENT_CAPACITY {
            mailbox.push(TerminalEvent::ClipboardStore(
                ClipboardType::Clipboard,
                "x".to_string(),
            ));
        }
        mailbox.push(TerminalEvent::PtyWrite("reply".to_string()));
        let drained = mailbox.drain();
        assert_eq!(
            drained
                .events
                .iter()
                .filter(|event| matches!(
                    event,
                    TerminalEvent::Pty(PtyEvent::IoError { fatal: true, .. })
                ))
                .count(),
            1
        );
    }

    #[test]
    fn terminal_bells_use_bounded_counter_instead_of_queue_entries() {
        let (_, mailbox) = event_proxy();
        for _ in 0..3 {
            mailbox.push(TerminalEvent::Bell);
        }
        let drained = mailbox.drain();
        assert_eq!(drained.bells, 3);
        assert!(drained.events.is_empty());
        assert_eq!(mailbox.drain().bells, 0);
    }

    #[test]
    fn terminal_title_is_bounded_and_runtime_only() {
        let (_, mailbox) = event_proxy();
        mailbox.push(TerminalEvent::Title("界".repeat(MAX_TITLE_BYTES)));
        let drained = mailbox.drain();
        let TerminalEvent::Title(title) = &drained.events[0] else {
            panic!("expected title event");
        };
        assert!(title.len() <= MAX_TITLE_BYTES);
        assert!(title.is_char_boundary(title.len()));
    }

    #[test]
    fn osc52_store_limit_rejects_payload_before_mailbox_retention() {
        let (proxy, mailbox) = event_proxy();
        proxy.send_event(Event::ClipboardStore(
            ClipboardType::Clipboard,
            "x".repeat(MAX_OSC52_BYTES + 1),
        ));
        let drained = mailbox.drain();
        assert!(matches!(
            drained.events.as_slice(),
            [TerminalEvent::ClipboardStoreRejected]
        ));
    }

    #[test]
    fn terminal_event_debug_redacts_sensitive_payloads() {
        let secret = "terminal-clipboard-secret";
        let store = format!(
            "{:?}",
            TerminalEvent::ClipboardStore(ClipboardType::Clipboard, secret.to_string())
        );
        let pty_write = format!("{:?}", TerminalEvent::PtyWrite(secret.to_string()));
        let title = format!("{:?}", TerminalEvent::Title(secret.to_string()));
        assert!(!store.contains(secret));
        assert!(!pty_write.contains(secret));
        assert!(!title.contains(secret));
    }
}
