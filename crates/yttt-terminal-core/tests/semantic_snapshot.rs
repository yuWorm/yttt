use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener};
use parking_lot::Mutex;
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::terminal::TerminalStreamUpdate::{Delta, Snapshot};
use yttt_terminal_core::{
    TerminalState,
    semantic::{SemanticCaptureContext, SemanticSnapshotter},
};

#[derive(Clone, Default)]
struct EventCollector(Arc<Mutex<Vec<Event>>>);

impl EventListener for EventCollector {
    fn send_event(&self, event: Event) {
        self.0.lock().push(event);
    }
}

fn row_text(row: &yttt_protocol::terminal::SemanticRow) -> String {
    row.spans.iter().map(|span| span.text.as_str()).collect()
}

#[test]
fn semantic_snapshots_use_stable_line_ids_and_bounded_row_deltas() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new_with_scrollback(8, 2, 20, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-1"), 3);
    let context = SemanticCaptureContext::running(8, 2, 1);

    terminal.process_bytes(b"one\r\ntwo");
    let first = match snapshots.capture(&terminal, &context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected initial snapshot, got {update:?}"),
    };
    let two_id = first
        .rows
        .iter()
        .find(|row| row_text(row).contains("two"))
        .unwrap()
        .line_id;

    terminal.process_bytes(b"\r\nthree");
    let delta = match snapshots.capture(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected semantic delta, got {update:?}"),
    };
    assert_eq!(delta.base_sequence, first.sequence);
    assert!(delta.changed_rows.len() <= 2);
    let current = snapshots.latest_viewport().unwrap();
    assert_eq!(
        current
            .rows
            .iter()
            .find(|row| row_text(row).contains("two"))
            .unwrap()
            .line_id,
        two_id
    );
}

#[test]
fn geometry_and_alternate_screen_changes_force_resynchronizing_snapshots() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new(12, 3, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-2"), 1);
    let first_context = SemanticCaptureContext::running(12, 3, 1);
    terminal.process_bytes("plain 界🙂".as_bytes());
    let first = match snapshots.capture(&terminal, &first_context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected initial snapshot, got {update:?}"),
    };
    assert!(row_text(&first.rows[0]).contains("界🙂"));

    terminal.resize(16, 4);
    let resized_context = SemanticCaptureContext::running(16, 4, 2);
    let resized = match snapshots.capture(&terminal, &resized_context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected geometry snapshot, got {update:?}"),
    };
    assert!(resized.scrollback_epoch > first.scrollback_epoch);

    terminal.process_bytes(b"\x1b[?1049hfullscreen");
    let alternate = match snapshots.capture(&terminal, &resized_context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected alternate-screen snapshot, got {update:?}"),
    };
    assert!(alternate.scrollback_epoch > resized.scrollback_epoch);
}

#[test]
fn checkpoint_carries_semantic_viewport_and_raw_replay_tail() {
    let events = EventCollector::default();
    let observed = events.clone();
    let mut terminal = TerminalState::new(8, 2, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-3"), 1);
    terminal.process_bytes(b"hello\x1b[c");
    snapshots.capture(&terminal, &SemanticCaptureContext::running(8, 2, 1));

    let checkpoint = snapshots.checkpoint(b"raw-tail".to_vec(), 7).unwrap();
    assert_eq!(checkpoint.raw_replay_tail, b"raw-tail");
    assert_eq!(checkpoint.raw_tail_start_sequence, 7);
    assert_eq!(checkpoint.viewport.sequence, 1);
    assert_eq!(
        observed
            .0
            .lock()
            .iter()
            .filter(|event| matches!(event, Event::PtyWrite(_)))
            .count(),
        1,
        "snapshotting must not duplicate terminal query replies"
    );
}

#[test]
fn burst_output_never_emits_more_changed_rows_than_the_viewport() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new_with_scrollback(20, 4, 100, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-4"), 1);
    let context = SemanticCaptureContext::running(20, 4, 1);
    snapshots.capture(&terminal, &context);

    for line in 0..500 {
        terminal.process_bytes(format!("line-{line}\r\n").as_bytes());
    }
    let delta = match snapshots.capture(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected delta, got {update:?}"),
    };
    assert!(delta.changed_rows.len() <= context.geometry.rows as usize);
}
