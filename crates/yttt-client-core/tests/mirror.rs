use yttt_client_core::{MirrorApply, TerminalMirror};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::terminal::{
    CursorShape, SemanticColor, SemanticCursor, SemanticDelta, SemanticRow, SemanticSpan,
    SemanticStyle, SemanticViewport, TerminalGeometry, TerminalModes, TerminalPalette,
    TerminalProcessState, TerminalStreamUpdate,
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
    }
}

#[test]
fn mirror_applies_contiguous_deltas_and_rejects_sequence_gaps() {
    let mut mirror = TerminalMirror::new(viewport());
    assert_eq!(
        mirror.apply(TerminalStreamUpdate::Delta(delta(4, 5, "after"))),
        MirrorApply::Updated
    );
    assert_eq!(mirror.viewport().sequence, 5);
    assert_eq!(mirror.viewport().rows[0].spans[0].text, "after");

    assert_eq!(
        mirror.apply(TerminalStreamUpdate::Delta(delta(3, 7, "corrupt"))),
        MirrorApply::SequenceGap
    );
    assert_eq!(mirror.viewport().sequence, 5);
    assert_eq!(mirror.viewport().rows[0].spans[0].text, "after");

    assert_eq!(
        mirror.apply(TerminalStreamUpdate::Delta(delta(4, 5, "duplicate"))),
        MirrorApply::Ignored
    );
}
