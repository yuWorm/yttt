use std::sync::Arc;

use yttt_client_core::{MirrorApply, TerminalMirror};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::terminal::{
    CursorShape, SemanticColor, SemanticCursor, SemanticDelta, SemanticGraphicCell,
    SemanticImagePlacement, SemanticRow, SemanticSpan, SemanticStyle, SemanticViewport,
    TerminalGeometry, TerminalImage, TerminalModes, TerminalPalette, TerminalProcessState,
    TerminalStreamUpdate,
};

fn row(text: &str) -> SemanticRow {
    SemanticRow {
        line_id: 1,
        viewport_row: 0,
        spans: vec![SemanticSpan {
            start_column: 0,
            text: text.to_string(),
            width: text.len() as u16,
            style: SemanticStyle {
                foreground: SemanticColor::Named(7),
                background: SemanticColor::Named(0),
                flags: 0,
                underline_color: SemanticColor::Named(7),
            },
            hyperlink: None,
        }],
        graphics: Vec::new(),
    }
}

fn viewport() -> SemanticViewport {
    SemanticViewport {
        session_id: TerminalSessionId::new("mirror"),
        session_epoch: 1,
        sequence: 4,
        geometry: TerminalGeometry {
            cols: 80,
            rows: 24,
            cell_width: 8,
            cell_height: 16,
        },
        geometry_epoch: 1,
        scrollback_epoch: 1,
        history_size: 0,
        display_offset: 0,
        rows: vec![row("before")],
        cursor: SemanticCursor {
            row: 0,
            column: 0,
            shape: CursorShape::Block,
            visible: true,
            blinking: false,
        },
        modes: TerminalModes {
            bits: 0,
            title: None,
            cwd: None,
        },
        palette: TerminalPalette {
            colors: Vec::new(),
            revision: 0,
        },
        process_state: TerminalProcessState::Running,
        images: Vec::new(),
        placements: Vec::new(),
    }
}

fn delta(base_sequence: u64, sequence: u64, text: &str) -> SemanticDelta {
    SemanticDelta {
        session_id: TerminalSessionId::new("mirror"),
        session_epoch: 1,
        base_sequence,
        sequence,
        geometry_epoch: 1,
        scrollback_epoch: 1,
        history_size: 0,
        display_offset: 0,
        changed_rows: vec![row(text)],
        cursor: None,
        modes: None,
        palette: None,
        process_state: None,
        images: None,
        placements: None,
    }
}

fn image(id: u64) -> Arc<TerminalImage> {
    Arc::new(TerminalImage {
        id,
        width: 1,
        height: 1,
        rgba: Arc::new(vec![255, 0, 0, 255]),
    })
}

fn placement(image_id: u64) -> SemanticImagePlacement {
    SemanticImagePlacement {
        image_id,
        x: 0,
        y: 0,
        width: 8,
        height: 16,
        source_x: 0,
        source_y: 0,
        source_width: 1,
        source_height: 1,
        clip_x: 0,
        clip_y: 0,
        clip_width: 8,
        clip_height: 16,
        z_index: 0,
        order: 0,
    }
}

#[test]
fn mirror_applies_contiguous_deltas_and_rejects_sequence_gaps() {
    let mut mirror = TerminalMirror::new(viewport());
    assert_eq!(
        mirror.apply(&TerminalStreamUpdate::Delta(delta(4, 5, "after"))),
        MirrorApply::Updated
    );
    assert_eq!(mirror.viewport().sequence, 5);
    assert_eq!(mirror.viewport().rows[0].spans[0].text, "after");

    assert_eq!(
        mirror.apply(&TerminalStreamUpdate::Delta(delta(3, 7, "corrupt"))),
        MirrorApply::SequenceGap
    );
    assert_eq!(mirror.viewport().sequence, 5);
    assert_eq!(mirror.viewport().rows[0].spans[0].text, "after");

    assert_eq!(
        mirror.apply(&TerminalStreamUpdate::Delta(delta(4, 5, "duplicate"))),
        MirrorApply::Ignored
    );
}

#[test]
fn reconnect_snapshot_reconstructs_visible_image_resources() {
    let mut mirror = TerminalMirror::new(viewport());
    let mut reconnected = viewport();
    reconnected.sequence = 5;
    reconnected.rows[0].graphics = vec![SemanticGraphicCell {
        column: 0,
        image_id: 8,
        offset_x: 0,
        offset_y: 0,
        cell_height: 16,
    }];
    reconnected.images = vec![image(8)];

    assert_eq!(
        mirror.apply(&TerminalStreamUpdate::Snapshot(reconnected)),
        MirrorApply::Updated
    );
    assert_eq!(
        mirror
            .viewport()
            .images
            .iter()
            .map(|image| image.id)
            .collect::<Vec<_>>(),
        vec![8]
    );
    assert_eq!(mirror.viewport().rows[0].graphics[0].image_id, 8);
}

#[test]
fn mirror_applies_placement_only_delta_and_clears_placements() {
    let mut initial = viewport();
    initial.images = vec![image(8)];
    initial.placements = vec![placement(8)];
    let mut mirror = TerminalMirror::new(initial);

    let mut replace = delta(4, 5, "before");
    replace.placements = Some(vec![placement(8)]);
    replace.placements.as_mut().unwrap()[0].x = 4;
    assert_eq!(
        mirror.apply(&TerminalStreamUpdate::Delta(replace)),
        MirrorApply::Updated
    );
    assert_eq!(mirror.viewport().placements[0].x, 4);

    let mut clear = delta(5, 6, "before");
    clear.placements = Some(Vec::new());
    assert_eq!(
        mirror.apply(&TerminalStreamUpdate::Delta(clear)),
        MirrorApply::Updated
    );
    assert!(mirror.viewport().placements.is_empty());
}
