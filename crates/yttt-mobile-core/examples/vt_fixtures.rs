//! Writes `mobile/fixtures/vt-fixtures.json`: repaint bytes from `VtEncoder`
//! and what a terminal component must show after feeding them. The iOS
//! (SwiftTerm) and Android (Termux) component tests read it.
//!
//! ```text
//! cargo run -p yttt-mobile-core --example vt_fixtures
//! ```

use alacritty_terminal::event::VoidListener;
use serde_json::{Value, json};
use yttt_core::model::ids::TerminalSessionId;
use yttt_mobile_core::vt::VtEncoder;
use yttt_protocol::terminal::{SemanticColor, SemanticViewport, TerminalStreamUpdate};
use yttt_terminal_core::{
    TerminalState,
    semantic::{
        STYLE_BOLD, STYLE_INVERSE, STYLE_ITALIC, STYLE_UNDERLINE, SemanticCaptureContext,
        SemanticSnapshotter,
    },
};

const COLS: usize = 20;
const ROWS: usize = 6;

const SOURCES: &[(&str, &str)] = &[
    ("plain", "hello\r\nworld\r\n\r\nlast line here"),
    ("full-width-row", "abcdefghijklmnopqrst\r\nnext"),
    (
        "colors",
        "\x1b[31mred\x1b[0m \x1b[92;44mbright\x1b[0m\r\n\x1b[38;5;208;48;5;17mindexed\x1b[0m\r\n\x1b[38;2;10;20;30;48;2;200;100;50mrgb\x1b[0m default",
    ),
    (
        "styles",
        "\x1b[1mbold\x1b[0m \x1b[3mit\x1b[0m \x1b[4mul\x1b[0m\r\n\x1b[7minv\x1b[0m plain",
    ),
    ("wide", "中文宽字符 ok\r\n😀 smile"),
    ("cursor", "prompt $ \x1b[3;5H"),
    (
        "hyperlink",
        "see \x1b]8;;https://example.com/a\x1b\\link\x1b]8;;\x1b\\ here",
    ),
];

fn capture(bytes: &[u8]) -> SemanticViewport {
    let mut state = TerminalState::new(COLS, ROWS, VoidListener);
    state.process_bytes(bytes);
    let mut snapshotter = SemanticSnapshotter::new(TerminalSessionId::new("fixture"), 1);
    let context = SemanticCaptureContext::running(COLS as u16, ROWS as u16, 1);
    match snapshotter.capture(&state, &context) {
        TerminalStreamUpdate::Snapshot(viewport) => viewport,
        other => panic!("expected a snapshot, got {other:?}"),
    }
}

fn color(color: SemanticColor) -> String {
    match color {
        SemanticColor::Named(index @ 0..=15) => format!("ansi:{index}"),
        SemanticColor::Named(index @ 259..=266) => format!("ansi:{}", index - 259),
        SemanticColor::Named(_) => "default".to_string(),
        SemanticColor::Indexed(index) => format!("ansi:{index}"),
        SemanticColor::Rgb { red, green, blue } => format!("rgb:{red},{green},{blue}"),
    }
}

fn expected(viewport: &SemanticViewport) -> Value {
    let mut rows = vec![String::new(); usize::from(viewport.geometry.rows)];
    let mut cells = Vec::new();
    for row in &viewport.rows {
        let text: String = row.spans.iter().map(|span| span.text.as_str()).collect();
        rows[usize::from(row.viewport_row)] = text.trim_end().to_string();
        for span in &row.spans {
            if span.text.trim().is_empty() {
                continue;
            }
            let flags = span.style.flags;
            cells.push(json!({
                "row": row.viewport_row,
                "col": span.start_column,
                "char": span.text.chars().next().map(String::from),
                "fg": color(span.style.foreground),
                "bg": color(span.style.background),
                "bold": flags & STYLE_BOLD != 0,
                "italic": flags & STYLE_ITALIC != 0,
                "underline": flags & STYLE_UNDERLINE != 0,
                "inverse": flags & STYLE_INVERSE != 0,
                "hyperlink": span.hyperlink,
            }));
        }
    }
    json!({
        "rows": rows,
        "cursor": {
            "row": viewport.cursor.row,
            "col": viewport.cursor.column,
            "visible": viewport.cursor.visible,
        },
        "cells": cells,
    })
}

fn main() {
    let fixtures: Vec<Value> = SOURCES
        .iter()
        .map(|(name, source)| {
            let viewport = capture(source.as_bytes());
            let bytes = VtEncoder::new().encode(&viewport);
            json!({
                "name": name,
                "cols": COLS,
                "rows": ROWS,
                "bytes": bytes,
                "expected": expected(&viewport),
            })
        })
        .collect();
    let path = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../mobile/fixtures/vt-fixtures.json"
    );
    let text = serde_json::to_string_pretty(&json!({ "fixtures": fixtures })).unwrap();
    std::fs::create_dir_all(std::path::Path::new(path).parent().unwrap()).unwrap();
    std::fs::write(path, text + "\n").unwrap();
    println!("wrote {path}");
}
