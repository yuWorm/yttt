use std::sync::Arc;

use alacritty_terminal::event::{Event, EventListener};
use parking_lot::Mutex;
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::terminal::{
    TerminalStreamApply, TerminalStreamDamage,
    TerminalStreamUpdate::{Delta, Snapshot},
};
use yttt_terminal_core::{
    TerminalState,
    semantic::{STYLE_WRAPLINE, SemanticCaptureContext, SemanticSnapshotter},
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

const ONE_PIXEL_RED_SIXEL: &[u8] = b"\x1bPq\"1;1;1;1#0;2;100;0;0@\x1b\\";
const ONE_PIXEL_RED_KITTY: &[u8] = b"\x1b_Ga=T,f=32,s=1,v=1,i=7,c=1,r=1;/wAA/w==\x1b\\";
const ONE_PIXEL_BLUE_KITTY_FRAME: &[u8] = b"\x1b_Ga=f,i=7,f=32,s=1,v=1;AAD//w==\x1b\\";
const SELECT_KITTY_SECOND_FRAME: &[u8] = b"\x1b_Ga=a,i=7,c=2;\x1b\\";

fn image_ids(viewport: &yttt_protocol::terminal::SemanticViewport) -> Vec<u64> {
    viewport.images.iter().map(|image| image.id).collect()
}

fn image_ids_from_assets(images: &[Arc<yttt_protocol::terminal::TerminalImage>]) -> Vec<u64> {
    images.iter().map(|image| image.id).collect()
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
fn semantic_snapshots_preserve_wrapped_line_boundaries() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new(4, 2, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-wrap"), 1);
    terminal.process_bytes(b"abcde");

    let snapshot = match snapshots.capture(&terminal, &SemanticCaptureContext::running(4, 2, 1)) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected initial snapshot, got {update:?}"),
    };

    assert!(
        snapshot.rows[0]
            .spans
            .iter()
            .any(|span| span.style.flags & STYLE_WRAPLINE != 0),
        "semantic rows must retain soft-wrap boundaries for copied selections"
    );
}

#[test]
fn damage_capture_emits_only_rows_touched_since_the_previous_frame() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new(80, 24, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-damage"), 1);
    let context = SemanticCaptureContext::running(80, 24, 1);

    let first = match snapshots.capture_damage(&terminal, &context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected initial snapshot, got {update:?}"),
    };
    terminal.process_bytes(b"x");
    let delta = match snapshots.capture_damage(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected damage delta, got {update:?}"),
    };

    assert_eq!(delta.base_sequence, first.sequence);
    assert_eq!(delta.changed_rows.len(), 1);
    assert!(row_text(&delta.changed_rows[0]).starts_with('x'));
    assert_eq!(
        snapshots.latest_viewport().unwrap().rows[0],
        delta.changed_rows[0]
    );
}

#[test]
fn image_only_delta_reconstructs_a_renderable_sixel_without_pty_replay() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new(8, 2, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-image"), 1);
    let context = SemanticCaptureContext::running(8, 2, 1);
    let initial = match snapshots.capture(&terminal, &context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected initial snapshot, got {update:?}"),
    };

    terminal.process_bytes(ONE_PIXEL_RED_SIXEL);
    let delta = match snapshots.capture_damage(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected image delta, got {update:?}"),
    };
    let assets = delta
        .images
        .as_ref()
        .expect("image insertion updates assets");
    assert_eq!(assets.len(), 1);
    assert_eq!(assets[0].width, 1);
    assert_eq!(assets[0].height, 1);
    assert_eq!(assets[0].rgba.len(), 4);

    let mut rendered = initial;
    assert_eq!(
        rendered.apply_stream_update(Delta(delta)),
        TerminalStreamApply::Updated(TerminalStreamDamage::Full)
    );
    let graphic = rendered
        .rows
        .iter()
        .flat_map(|row| row.graphics.iter())
        .next()
        .expect("image cells are present in the semantic stream");
    assert_eq!(image_ids(&rendered), vec![graphic.image_id]);
}

#[test]
fn kitty_animation_frame_switches_change_only_the_scene_and_keep_all_frame_assets() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new(8, 2, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-kitty"), 1);
    let context = SemanticCaptureContext::running(8, 2, 1);
    let initial = match snapshots.capture_damage(&terminal, &context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected initial snapshot, got {update:?}"),
    };

    terminal.process_bytes(ONE_PIXEL_RED_KITTY);
    let root = match snapshots.capture_damage(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected Kitty placement delta, got {update:?}"),
    };
    let root_placement = root
        .placements
        .as_ref()
        .and_then(|placements| placements.first())
        .expect("Kitty transmit-and-display creates a placement")
        .clone();
    let root_assets = root
        .images
        .as_ref()
        .expect("Kitty placement includes its base frame asset");
    assert_eq!(root_assets.len(), 1);
    assert!(image_ids_from_assets(root_assets).contains(&root_placement.image_id));

    terminal.process_bytes(ONE_PIXEL_BLUE_KITTY_FRAME);
    let frame_loaded = match snapshots.capture_damage(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected animation-frame delta, got {update:?}"),
    };
    let all_frame_assets = frame_loaded
        .images
        .as_ref()
        .expect("a visible animation retains every frame resource");
    assert_eq!(all_frame_assets.len(), 2);
    let all_frame_ids = image_ids_from_assets(all_frame_assets);

    terminal.process_bytes(SELECT_KITTY_SECOND_FRAME);
    let switched = match snapshots.capture_damage(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected frame-switch delta, got {update:?}"),
    };
    assert!(switched.changed_rows.is_empty());
    assert!(
        switched.images.is_none(),
        "switching frames must not resend an unchanged animation asset set"
    );
    let switched_placement = switched
        .placements
        .as_ref()
        .and_then(|placements| placements.first())
        .expect("the scene must identify the newly current frame")
        .clone();
    assert_ne!(switched_placement.image_id, root_placement.image_id);
    assert!(all_frame_ids.contains(&switched_placement.image_id));

    let checkpoint = snapshots
        .checkpoint(Vec::new(), 0)
        .expect("latest Kitty scene checkpoints");
    assert_eq!(image_ids(&checkpoint.viewport), all_frame_ids);
    assert_eq!(
        checkpoint.viewport.placements[0].image_id,
        switched_placement.image_id
    );

    let mut rendered = initial;
    assert_eq!(
        rendered.apply_stream_update(Delta(root)),
        TerminalStreamApply::Updated(TerminalStreamDamage::Full)
    );
    assert_eq!(
        rendered.apply_stream_update(Delta(frame_loaded)),
        TerminalStreamApply::Updated(TerminalStreamDamage::Full)
    );
    assert_eq!(
        rendered.apply_stream_update(Delta(switched)),
        TerminalStreamApply::Updated(TerminalStreamDamage::Full)
    );
    assert_eq!(
        rendered.placements[0].image_id, switched_placement.image_id,
        "a placement-only update replaces the complete remote scene"
    );
    assert_eq!(image_ids(&rendered), all_frame_ids);
}

#[test]
fn kitty_scene_survives_partial_text_capture_without_retransmitting_assets() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new(8, 2, events);
    let mut snapshots =
        SemanticSnapshotter::new(TerminalSessionId::new("terminal-kitty-partial"), 1);
    let context = SemanticCaptureContext::running(8, 2, 1);
    snapshots.capture_damage(&terminal, &context);

    terminal.process_bytes(ONE_PIXEL_RED_KITTY);
    let placed = match snapshots.capture_damage(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected Kitty placement delta, got {update:?}"),
    };
    let expected_scene = placed.placements.expect("placement scene");

    terminal.process_bytes(b"text");
    let text_delta = match snapshots.capture_damage(&terminal, &context) {
        Delta(delta) => delta,
        update => panic!("expected partial text delta, got {update:?}"),
    };
    assert!(text_delta.placements.is_none());
    assert!(text_delta.images.is_none());
    assert_eq!(
        snapshots.latest_viewport().unwrap().placements,
        expected_scene,
        "partial capture retains the complete unchanged scene"
    );
}

#[test]
fn history_viewport_retains_kitty_placements_and_their_assets() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new_with_scrollback(8, 2, 20, events);
    let mut snapshots =
        SemanticSnapshotter::new(TerminalSessionId::new("terminal-kitty-history"), 1);
    let context = SemanticCaptureContext::running(8, 2, 1);

    terminal.process_bytes(ONE_PIXEL_RED_KITTY);
    terminal.process_bytes(b"\r\none\r\ntwo\r\nthree");
    let bottom = match snapshots.capture(&terminal, &context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected initial snapshot, got {update:?}"),
    };
    let history = snapshots
        .read_viewport(
            &terminal,
            &context,
            bottom.scrollback_epoch,
            yttt_protocol::terminal::TerminalViewportAnchor::DisplayOffset(bottom.history_size),
        )
        .expect("scrollback viewport");

    assert!(!history.placements.is_empty());
    assert!(
        history
            .placements
            .iter()
            .all(|placement| image_ids(&history).contains(&placement.image_id))
    );
}

#[test]
fn semantic_text_hides_kitty_unicode_placeholder_and_its_diacritics() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new(8, 2, events);
    let mut snapshots =
        SemanticSnapshotter::new(TerminalSessionId::new("terminal-kitty-placeholder"), 1);

    terminal.process_bytes("\u{10EEEE}\u{0305}".as_bytes());
    let snapshot = match snapshots.capture(&terminal, &SemanticCaptureContext::running(8, 2, 1)) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected initial snapshot, got {update:?}"),
    };
    let text = row_text(&snapshot.rows[0]);
    assert!(!text.contains('\u{10EEEE}'));
    assert!(!text.contains('\u{0305}'));
    assert!(text.starts_with(' '));
}

#[test]
fn history_viewport_retains_assets_for_scrolled_sixels() {
    let events = EventCollector::default();
    let mut terminal = TerminalState::new_with_scrollback(8, 2, 20, events);
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("terminal-history"), 1);
    let context = SemanticCaptureContext::running(8, 2, 1);

    terminal.process_bytes(ONE_PIXEL_RED_SIXEL);
    terminal.process_bytes(b"\r\none\r\ntwo\r\nthree");
    let bottom = match snapshots.capture(&terminal, &context) {
        Snapshot(snapshot) => snapshot,
        update => panic!("expected bottom snapshot, got {update:?}"),
    };
    let history = snapshots
        .read_viewport(
            &terminal,
            &context,
            bottom.scrollback_epoch,
            yttt_protocol::terminal::TerminalViewportAnchor::DisplayOffset(bottom.history_size),
        )
        .expect("scrollback viewport");

    let visible_graphics = history
        .rows
        .iter()
        .flat_map(|row| row.graphics.iter())
        .collect::<Vec<_>>();
    assert!(!visible_graphics.is_empty());
    assert!(
        visible_graphics
            .iter()
            .all(|graphic| image_ids(&history).contains(&graphic.image_id))
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

#[test]
fn phone_screenshot_survives_chunked_upload_and_semantic_capture() {
    let mut terminal = TerminalState::new(100, 100, EventCollector::default());
    let pixels = "/wAA".repeat(882 * 1568);
    for image_id in 1..=2 {
        terminal.process_bytes(format!("\x1b[1;{}H", 1 + (image_id - 1) * 50).as_bytes());
        let chunks = pixels.as_bytes().chunks(4096);
        let count = chunks.len();
        for (index, chunk) in chunks.enumerate() {
            let more = u8::from(index + 1 < count);
            let header = if index == 0 {
                format!("\x1b_Ga=T,f=24,s=882,v=1568,i={image_id},c=40,r=70,C=1,m={more};")
            } else {
                format!("\x1b_Gm={more};")
            };
            terminal.process_bytes(header.as_bytes());
            terminal.process_bytes(chunk);
            terminal.process_bytes(b"\x1b\\");
        }
    }
    let mut snapshots = SemanticSnapshotter::new(TerminalSessionId::new("phone-image"), 1);
    snapshots.capture(&terminal, &SemanticCaptureContext::running(100, 100, 1));
    let viewport = snapshots.latest_viewport().unwrap();
    assert_eq!(
        viewport.placements.len(),
        2,
        "both phone screenshots must be visible"
    );
    assert_eq!(viewport.images.len(), 2);
    assert_eq!(
        (viewport.images[0].width, viewport.images[0].height),
        (882, 1568)
    );
    assert_eq!(viewport.images[0].rgba.len(), 882 * 1568 * 4);
    let checkpoint = snapshots.checkpoint(Vec::new(), 0).unwrap();
    let encoded = yttt_protocol::encode_message(yttt_protocol::FrameKind::Control, &checkpoint)
        .expect("a reconnect checkpoint must carry both full-resolution images");
    let decoded: yttt_protocol::terminal::TerminalCheckpoint =
        yttt_protocol::decode_message(&yttt_protocol::decode_frame(&encoded).unwrap()).unwrap();
    assert_eq!(decoded.viewport, checkpoint.viewport);
}
