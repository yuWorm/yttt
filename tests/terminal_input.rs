use std::time::Duration;
use std::{
    cell::RefCell,
    io::{self, Read, Write},
    rc::Rc,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use gpui::{
    AppContext, Context, Entity, FocusHandle, InteractiveElement, IntoElement, Keystroke,
    ParentElement, Render, Styled, TestAppContext, Window, div,
};
use yttt_terminal::input::{KeyState, TerminalKeyEvent, encode_key};

fn keystroke_with_platform_key(parse: &str, key: &str) -> Keystroke {
    let mut keystroke = Keystroke::parse(parse).unwrap();
    keystroke.key = key.to_string();
    keystroke
}

fn encoded(keystroke: &Keystroke) -> Option<Vec<u8>> {
    let event = TerminalKeyEvent::from_gpui_keystroke(keystroke, KeyState::Pressed, false)?;
    encode_key(&event, Default::default()).map(|bytes| bytes.to_vec())
}

#[test]
fn terminal_input_accepts_platform_special_key_names() {
    assert_eq!(
        encoded(&keystroke_with_platform_key("tab", "Tab")),
        Some(b"\t".to_vec())
    );
    assert_eq!(
        encoded(&keystroke_with_platform_key("enter", "Enter")),
        Some(b"\r".to_vec())
    );
    assert_eq!(
        encoded(&keystroke_with_platform_key("escape", "Escape")),
        Some(b"\x1b".to_vec())
    );
    assert_eq!(
        encoded(&keystroke_with_platform_key("up", "ArrowUp")),
        Some(b"\x1b[A".to_vec())
    );
}

#[test]
fn terminal_input_accepts_uppercase_control_key_names() {
    assert_eq!(
        encoded(&keystroke_with_platform_key("ctrl-c", "C")),
        Some(vec![0x03])
    );
}

#[test]
fn terminal_input_preserves_platform_modifier_for_routing() {
    let keystroke = keystroke_with_platform_key("cmd-s", "s");
    let event =
        TerminalKeyEvent::from_gpui_keystroke(&keystroke, KeyState::Pressed, false).unwrap();
    assert!(event.modifiers.super_key);
    assert_eq!(encode_key(&event, Default::default()), None);
}

struct BlockingReader {
    release: mpsc::Receiver<()>,
}

impl Read for BlockingReader {
    fn read(&mut self, _bytes: &mut [u8]) -> io::Result<usize> {
        let _ = self.release.recv();
        Ok(0)
    }
}

struct RecordingWriter {
    writes: mpsc::Sender<Vec<u8>>,
}

impl Write for RecordingWriter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes
            .send(bytes.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "test receiver dropped"))?;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct TerminalTabTestHost {
    terminal: Entity<yttt_terminal::TerminalView>,
    next_focus: FocusHandle,
}

impl Render for TerminalTabTestHost {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .tab_group()
            .child(self.terminal.clone())
            .child(div().track_focus(&self.next_focus).tab_index(0))
    }
}

#[gpui::test]
fn terminal_tab_bindings_override_component_root_focus_navigation(cx: &mut TestAppContext) {
    cx.update(|cx| {
        gpui_component::init(cx);
        yttt_terminal::init(cx);
    });

    let (writes, recorded_writes) = mpsc::channel();
    let (reader_release, reader_wait) = mpsc::channel();
    std::mem::forget(reader_release);
    let input_blocked = Arc::new(AtomicBool::new(false));
    let input_blocked_for_window = input_blocked.clone();
    let terminal_slot = Rc::new(RefCell::new(None));
    let terminal_slot_for_window = terminal_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, cx| {
        let terminal = cx.new(|cx| {
            yttt_terminal::TerminalView::new(
                RecordingWriter { writes },
                BlockingReader {
                    release: reader_wait,
                },
                yttt_terminal::TerminalConfig::default(),
                cx,
            )
            .with_key_handler(move |_| input_blocked_for_window.load(Ordering::SeqCst))
        });
        let focus_handle = terminal.read(cx).focus_handle().clone();
        focus_handle.focus(window, cx);
        *terminal_slot_for_window.borrow_mut() = Some(terminal.clone());
        let host = cx.new(|cx| TerminalTabTestHost {
            terminal,
            next_focus: cx.focus_handle(),
        });
        gpui_component::Root::new(host, window, cx)
    });
    let terminal = terminal_slot.borrow_mut().take().unwrap();
    cx.run_until_parked();

    cx.simulate_keystrokes("tab");
    assert_eq!(
        recorded_writes.recv_timeout(Duration::from_secs(1)),
        Ok(b"\t".to_vec())
    );
    cx.update(|window, cx| {
        assert!(
            terminal.read(cx).focus_handle().is_focused(window),
            "Tab must not move focus to the next application tab stop"
        );
    });

    cx.simulate_keystrokes("x");
    assert_eq!(
        recorded_writes.recv_timeout(Duration::from_secs(1)),
        Ok(b"x".to_vec())
    );

    cx.simulate_keystrokes("ctrl-c");
    assert_eq!(
        recorded_writes.recv_timeout(Duration::from_secs(1)),
        Ok(vec![0x03])
    );

    #[cfg(target_os = "macos")]
    cx.simulate_keystrokes("cmd-c");
    #[cfg(not(target_os = "macos"))]
    cx.simulate_keystrokes("ctrl-shift-c");
    assert_eq!(
        recorded_writes.recv_timeout(Duration::from_millis(20)),
        Err(mpsc::RecvTimeoutError::Timeout),
        "Copy without a selection must be a consumed no-op"
    );

    cx.simulate_keystrokes("shift-tab");
    assert_eq!(
        recorded_writes.recv_timeout(Duration::from_secs(1)),
        Ok(b"\x1b[Z".to_vec())
    );
    input_blocked.store(true, Ordering::SeqCst);
    cx.simulate_keystrokes("tab");
    assert_eq!(recorded_writes.try_recv(), Err(mpsc::TryRecvError::Empty));
    cx.update(|window, cx| {
        assert!(
            terminal.read(cx).focus_handle().is_focused(window),
            "terminal Tab handling must retain terminal focus"
        );
    });
}
