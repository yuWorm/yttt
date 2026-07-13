use std::{
    cell::RefCell,
    io::{self, Write},
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
use yttt_terminal::input::keystroke_to_bytes;

fn keystroke_with_platform_key(parse: &str, key: &str) -> Keystroke {
    let mut keystroke = Keystroke::parse(parse).unwrap();
    keystroke.key = key.to_string();
    keystroke
}

#[test]
fn terminal_input_accepts_platform_special_key_names() {
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("tab", "Tab"),
            Default::default()
        ),
        Some(b"\t".to_vec())
    );
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("enter", "Enter"),
            Default::default()
        ),
        Some(b"\r".to_vec())
    );
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("escape", "Escape"),
            Default::default()
        ),
        Some(b"\x1b".to_vec())
    );
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("up", "ArrowUp"),
            Default::default()
        ),
        Some(b"\x1b[A".to_vec())
    );
}

#[test]
fn terminal_input_accepts_uppercase_control_key_names() {
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("ctrl-c", "C"),
            Default::default()
        ),
        Some(vec![0x03])
    );
}

#[test]
fn terminal_input_ignores_platform_shortcuts() {
    let mut keystroke = keystroke_with_platform_key("cmd-s", "s");
    keystroke.key_char = None;

    assert_eq!(keystroke_to_bytes(&keystroke, Default::default()), None);
    assert_eq!(
        keystroke_to_bytes(
            &keystroke_with_platform_key("cmd-tab", "Tab"),
            Default::default()
        ),
        None
    );
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
    let input_blocked = Arc::new(AtomicBool::new(false));
    let input_blocked_for_window = input_blocked.clone();
    let terminal_slot = Rc::new(RefCell::new(None));
    let terminal_slot_for_window = terminal_slot.clone();
    let (_root, cx) = cx.add_window_view(move |window, cx| {
        let terminal = cx.new(|cx| {
            yttt_terminal::TerminalView::new(
                RecordingWriter { writes },
                io::empty(),
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
    assert_eq!(recorded_writes.try_recv(), Ok(b"\t".to_vec()));
    cx.update(|window, cx| {
        assert!(
            terminal.read(cx).focus_handle().is_focused(window),
            "Tab must not move focus to the next application tab stop"
        );
    });

    cx.simulate_keystrokes("x");
    assert_eq!(recorded_writes.try_recv(), Ok(b"x".to_vec()));

    cx.simulate_keystrokes("shift-tab");
    assert_eq!(recorded_writes.try_recv(), Ok(b"\x1b[Z".to_vec()));
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
