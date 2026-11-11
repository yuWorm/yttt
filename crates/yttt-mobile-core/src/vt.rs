//! Re-encodes a Host terminal viewport as VT repaint sequences for a native
//! terminal component (SwiftTerm, Termux). The Host parses the PTY stream
//! once; the component only parses the plain repaints written here.
//!
//! The repaint never relies on the component's own wrapping or erase rules:
//! autowrap stays off, every span is placed with its own `CUP`, and a row is
//! cleared with `CSI 2K` from column 1 before it is written, never with `EL`
//! after text (a full row leaves the cursor in the pending-wrap state, where
//! xterm-style components would erase the last column).

use std::{
    collections::hash_map::DefaultHasher,
    hash::{Hash, Hasher},
    io::Write as _,
};

use wire::{
    STYLE_BOLD, STYLE_DASHED_UNDERLINE, STYLE_DIM, STYLE_DOTTED_UNDERLINE, STYLE_DOUBLE_UNDERLINE,
    STYLE_HIDDEN, STYLE_INVERSE, STYLE_ITALIC, STYLE_STRIKEOUT, STYLE_UNDERCURL, STYLE_UNDERLINE,
    TermMode,
};
use yttt_protocol::terminal::{
    CursorShape, DynamicColor, SemanticColor, SemanticCursor, SemanticSpan, SemanticViewport,
};

/// Bits the Host sends as they are: the style flags of `yttt-terminal-core`'s
/// semantic capture and alacritty's `TermMode`. Named here so the mobile core
/// needs neither crate (alacritty does not build for Android); a test checks
/// they still match.
mod wire {
    pub const STYLE_INVERSE: u16 = 1 << 0;
    pub const STYLE_BOLD: u16 = 1 << 1;
    pub const STYLE_ITALIC: u16 = 1 << 2;
    pub const STYLE_UNDERLINE: u16 = 1 << 3;
    pub const STYLE_DIM: u16 = 1 << 4;
    pub const STYLE_HIDDEN: u16 = 1 << 5;
    pub const STYLE_STRIKEOUT: u16 = 1 << 6;
    pub const STYLE_DOUBLE_UNDERLINE: u16 = 1 << 7;
    pub const STYLE_UNDERCURL: u16 = 1 << 8;
    pub const STYLE_DOTTED_UNDERLINE: u16 = 1 << 9;
    pub const STYLE_DASHED_UNDERLINE: u16 = 1 << 10;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub struct TermMode(u32);

    impl TermMode {
        pub const APP_CURSOR: Self = Self(1 << 1);
        pub const APP_KEYPAD: Self = Self(1 << 2);
        pub const MOUSE_REPORT_CLICK: Self = Self(1 << 3);
        pub const BRACKETED_PASTE: Self = Self(1 << 4);
        pub const SGR_MOUSE: Self = Self(1 << 5);
        pub const MOUSE_MOTION: Self = Self(1 << 6);
        pub const FOCUS_IN_OUT: Self = Self(1 << 11);
        pub const MOUSE_DRAG: Self = Self(1 << 13);
        pub const MOUSE_MODE: Self =
            Self(Self::MOUSE_REPORT_CLICK.0 | Self::MOUSE_MOTION.0 | Self::MOUSE_DRAG.0);

        pub const fn from_bits_truncate(bits: u32) -> Self {
            Self(bits)
        }

        pub const fn bits(self) -> u32 {
            self.0
        }

        pub const fn contains(self, other: Self) -> bool {
            self.0 & other.0 == other.0
        }
    }

    impl std::ops::BitOr for TermMode {
        type Output = Self;
        fn bitor(self, other: Self) -> Self {
            Self(self.0 | other.0)
        }
    }

    impl std::ops::BitAnd for TermMode {
        type Output = Self;
        fn bitand(self, other: Self) -> Self {
            Self(self.0 & other.0)
        }
    }

    impl std::ops::BitOrAssign for TermMode {
        fn bitor_assign(&mut self, other: Self) {
            self.0 |= other.0;
        }
    }
}

/// Named color numbers of alacritty's `NamedColor`, which the Host sends as is.
const NAMED_FOREGROUND: u16 = 256;
const NAMED_BACKGROUND: u16 = 257;
const NAMED_CURSOR: u16 = 258;
const NAMED_DIM_BLACK: u16 = 259;
const NAMED_DIM_WHITE: u16 = 266;

const ANY_UNDERLINE: u16 = STYLE_UNDERLINE
    | STYLE_DOUBLE_UNDERLINE
    | STYLE_UNDERCURL
    | STYLE_DOTTED_UNDERLINE
    | STYLE_DASHED_UNDERLINE;

/// Modes that change how the component encodes keys, always synced.
const INPUT_MODES: [(TermMode, &str, &str); 2] = [
    (TermMode::APP_CURSOR, "\x1b[?1h", "\x1b[?1l"),
    (TermMode::BRACKETED_PASTE, "\x1b[?2004h", "\x1b[?2004l"),
];
/// Reporting modes, synced only while the user controls the terminal.
/// Resets come before sets: the mouse tracking modes replace one another.
const REPORT_MODES: [(TermMode, &str, &str); 5] = [
    (TermMode::MOUSE_REPORT_CLICK, "\x1b[?1000h", "\x1b[?1000l"),
    (TermMode::MOUSE_DRAG, "\x1b[?1002h", "\x1b[?1002l"),
    (TermMode::MOUSE_MOTION, "\x1b[?1003h", "\x1b[?1003l"),
    (TermMode::SGR_MOUSE, "\x1b[?1006h", "\x1b[?1006l"),
    (TermMode::FOCUS_IN_OUT, "\x1b[?1004h", "\x1b[?1004l"),
];

#[derive(Clone, Copy, PartialEq, Eq)]
struct Frame {
    session_epoch: u64,
    geometry_epoch: u64,
    cols: u16,
    rows: u16,
}

/// Turns successive viewports of one terminal into the bytes that repaint
/// what changed.
#[derive(Default)]
pub struct VtEncoder {
    frame: Option<Frame>,
    /// Content fingerprint of each component row as last written.
    rows: Vec<Option<u64>>,
    /// Mode bits as last written; `None` until the first full sync.
    modes: Option<u32>,
    palette: Vec<DynamicColor>,
    cursor: Option<SemanticCursor>,
    interactive: bool,
}

impl VtEncoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the user controls the terminal; mouse and focus reporting are
    /// synced only then, so taps while observing stay local.
    pub fn set_interactive(&mut self, interactive: bool) {
        self.interactive = interactive;
    }

    /// Forgets what the component shows, so the next `encode` repaints all of it.
    pub fn reset(&mut self) {
        *self = Self {
            interactive: self.interactive,
            ..Self::default()
        };
    }

    /// The bytes that bring the component from the last encoded viewport to
    /// `viewport`; empty when nothing changed.
    pub fn encode(&mut self, viewport: &SemanticViewport) -> Vec<u8> {
        let mut out = Vec::new();
        let frame = Frame {
            session_epoch: viewport.session_epoch,
            geometry_epoch: viewport.geometry_epoch,
            cols: viewport.geometry.cols,
            rows: viewport.geometry.rows,
        };
        if self.frame != Some(frame) {
            self.reset();
            self.frame = Some(frame);
            self.rows = vec![None; usize::from(frame.rows)];
            out.extend_from_slice(b"\x1b[0m\x1b[H\x1b[2J");
        }
        self.sync_palette(&viewport.palette.colors, &mut out);
        self.sync_modes(viewport.modes.bits, &mut out);
        let mut written = vec![false; self.rows.len()];
        for row in &viewport.rows {
            let index = usize::from(row.viewport_row);
            let Some(slot) = self.rows.get_mut(index) else {
                continue;
            };
            written[index] = true;
            let fingerprint = fingerprint(&row.spans);
            if *slot != Some(fingerprint) {
                *slot = Some(fingerprint);
                write_row(index, &row.spans, &mut out);
            }
        }
        // A row the viewport leaves out is blank.
        for (index, slot) in self.rows.iter_mut().enumerate() {
            if !written[index] && *slot != Some(BLANK_ROW) {
                *slot = Some(BLANK_ROW);
                write_row(index, &[], &mut out);
            }
        }
        if !out.is_empty() || self.cursor != Some(viewport.cursor) {
            self.cursor = Some(viewport.cursor);
            write_cursor(viewport.cursor, &mut out);
        }
        if out.is_empty() {
            return out;
        }
        let mut framed = b"\x1b[?7l".to_vec();
        framed.extend_from_slice(&out);
        framed
    }

    fn sync_palette(&mut self, colors: &[DynamicColor], out: &mut Vec<u8>) {
        if self.palette.as_slice() == colors {
            return;
        }
        for old in &self.palette {
            if !colors.iter().any(|color| color.index == old.index) {
                match old.index {
                    0..=255 => {
                        let _ = write!(out, "\x1b]104;{}\x1b\\", old.index);
                    }
                    NAMED_FOREGROUND => out.extend_from_slice(b"\x1b]110\x1b\\"),
                    NAMED_BACKGROUND => out.extend_from_slice(b"\x1b]111\x1b\\"),
                    NAMED_CURSOR => out.extend_from_slice(b"\x1b]112\x1b\\"),
                    _ => {}
                }
            }
        }
        for color in colors {
            let rgb = format!(
                "rgb:{:02x}/{:02x}/{:02x}",
                color.red, color.green, color.blue
            );
            let _ = match color.index {
                0..=255 => write!(out, "\x1b]4;{};{rgb}\x1b\\", color.index),
                NAMED_FOREGROUND => write!(out, "\x1b]10;{rgb}\x1b\\"),
                NAMED_BACKGROUND => write!(out, "\x1b]11;{rgb}\x1b\\"),
                NAMED_CURSOR => write!(out, "\x1b]12;{rgb}\x1b\\"),
                _ => Ok(()),
            };
        }
        self.palette = colors.to_vec();
    }

    fn sync_modes(&mut self, bits: u32, out: &mut Vec<u8>) {
        let host = TermMode::from_bits_truncate(bits);
        let mut wanted =
            host & (TermMode::APP_CURSOR | TermMode::BRACKETED_PASTE | TermMode::APP_KEYPAD);
        if self.interactive {
            wanted |= host & (TermMode::MOUSE_MODE | TermMode::SGR_MOUSE | TermMode::FOCUS_IN_OUT);
        }
        let previous = self.modes.map(TermMode::from_bits_truncate);
        if previous == Some(wanted) {
            return;
        }
        let changed =
            |mode: TermMode| previous.is_none_or(|old| old.contains(mode) != wanted.contains(mode));
        for (mode, set, reset) in INPUT_MODES {
            if changed(mode) {
                out.extend_from_slice(if wanted.contains(mode) { set } else { reset }.as_bytes());
            }
        }
        if changed(TermMode::APP_KEYPAD) {
            out.extend_from_slice(if wanted.contains(TermMode::APP_KEYPAD) {
                b"\x1b="
            } else {
                b"\x1b>"
            });
        }
        for (mode, _, reset) in REPORT_MODES {
            if changed(mode) && !wanted.contains(mode) {
                out.extend_from_slice(reset.as_bytes());
            }
        }
        for (mode, set, _) in REPORT_MODES {
            if changed(mode) && wanted.contains(mode) {
                out.extend_from_slice(set.as_bytes());
            }
        }
        self.modes = Some(wanted.bits());
    }
}

const BLANK_ROW: u64 = 0;

fn fingerprint(spans: &[SemanticSpan]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for span in spans {
        span.start_column.hash(&mut hasher);
        span.text.hash(&mut hasher);
        span.width.hash(&mut hasher);
        color_key(span.style.foreground).hash(&mut hasher);
        color_key(span.style.background).hash(&mut hasher);
        color_key(span.style.underline_color).hash(&mut hasher);
        span.style.flags.hash(&mut hasher);
        span.hyperlink.hash(&mut hasher);
    }
    // Never collide with the blank marker.
    hasher.finish() | 1
}

fn color_key(color: SemanticColor) -> (u8, u32) {
    match color {
        SemanticColor::Named(named) => (0, u32::from(named)),
        SemanticColor::Indexed(index) => (1, u32::from(index)),
        SemanticColor::Rgb { red, green, blue } => (
            2,
            u32::from(red) << 16 | u32::from(green) << 8 | u32::from(blue),
        ),
    }
}

fn write_row(index: usize, spans: &[SemanticSpan], out: &mut Vec<u8>) {
    let _ = write!(out, "\x1b[{};1H\x1b[0m\x1b[2K", index + 1);
    for span in spans {
        let _ = write!(
            out,
            "\x1b[{};{}H",
            index + 1,
            usize::from(span.start_column) + 1
        );
        write_sgr(span, out);
        match &span.hyperlink {
            Some(uri) => {
                let _ = write!(out, "\x1b]8;;{uri}\x1b\\{}\x1b]8;;\x1b\\", span.text);
            }
            None => out.extend_from_slice(span.text.as_bytes()),
        }
    }
    out.extend_from_slice(b"\x1b[0m");
}

#[derive(Clone, Copy)]
enum Layer {
    Foreground,
    Background,
    Underline,
}

fn write_sgr(span: &SemanticSpan, out: &mut Vec<u8>) {
    let style = span.style;
    let mut params = vec!["0".to_string()];
    for (flag, param) in [
        (STYLE_BOLD, "1"),
        (STYLE_DIM, "2"),
        (STYLE_ITALIC, "3"),
        (STYLE_INVERSE, "7"),
        (STYLE_HIDDEN, "8"),
        (STYLE_STRIKEOUT, "9"),
    ] {
        if style.flags & flag != 0 {
            params.push(param.into());
        }
    }
    let underline = [
        (STYLE_DOUBLE_UNDERLINE, "4:2"),
        (STYLE_UNDERCURL, "4:3"),
        (STYLE_DOTTED_UNDERLINE, "4:4"),
        (STYLE_DASHED_UNDERLINE, "4:5"),
        (STYLE_UNDERLINE, "4"),
    ]
    .into_iter()
    .find(|(flag, _)| style.flags & flag != 0);
    if let Some((_, param)) = underline {
        params.push(param.into());
    }
    params.extend(color_param(style.foreground, Layer::Foreground));
    params.extend(color_param(style.background, Layer::Background));
    if style.flags & ANY_UNDERLINE != 0 && style.underline_color != style.foreground {
        params.extend(color_param(style.underline_color, Layer::Underline));
    }
    let _ = write!(out, "\x1b[{}m", params.join(";"));
}

fn color_param(color: SemanticColor, layer: Layer) -> Option<String> {
    let (base, bright, extended) = match layer {
        Layer::Foreground => (30, 90, 38),
        Layer::Background => (40, 100, 48),
        Layer::Underline => (0, 0, 58),
    };
    match color {
        SemanticColor::Named(named) => {
            let index = match named {
                0..=15 => named,
                NAMED_DIM_BLACK..=NAMED_DIM_WHITE => named - NAMED_DIM_BLACK,
                // The default colors and anything without an SGR form.
                _ => return None,
            };
            match layer {
                Layer::Underline => Some(format!("58;5;{index}")),
                _ if index < 8 => Some((base + index).to_string()),
                _ => Some((bright + index - 8).to_string()),
            }
        }
        SemanticColor::Indexed(index) => Some(format!("{extended};5;{index}")),
        SemanticColor::Rgb { red, green, blue } => {
            Some(format!("{extended};2;{red};{green};{blue}"))
        }
    }
}

fn write_cursor(cursor: SemanticCursor, out: &mut Vec<u8>) {
    let shape = match (cursor.shape, cursor.blinking) {
        (CursorShape::Block | CursorShape::HollowBlock, true) => 1,
        (CursorShape::Block | CursorShape::HollowBlock, false) => 2,
        (CursorShape::Underline, true) => 3,
        (CursorShape::Underline, false) => 4,
        (CursorShape::Beam, true) => 5,
        (CursorShape::Beam, false) => 6,
    };
    let visibility = if cursor.visible { 'h' } else { 'l' };
    let _ = write!(
        out,
        "\x1b[{};{}H\x1b[{shape} q\x1b[?25{visibility}",
        usize::from(cursor.row) + 1,
        usize::from(cursor.column) + 1
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::{event::VoidListener, term::TermMode};
    use yttt_core::model::ids::TerminalSessionId;
    use yttt_protocol::terminal::TerminalStreamUpdate;
    use yttt_terminal_core::{
        TerminalState,
        semantic::{STYLE_WRAPLINE, SemanticCaptureContext, SemanticSnapshotter},
    };

    const COLS: usize = 20;
    const ROWS: usize = 6;

    fn capture_sized(bytes: &[u8], cols: usize, rows: usize, epoch: u64) -> SemanticViewport {
        let mut state = TerminalState::new(cols, rows, VoidListener);
        state.process_bytes(bytes);
        let mut snapshotter = SemanticSnapshotter::new(TerminalSessionId::new("t"), 1);
        let context = SemanticCaptureContext::running(cols as u16, rows as u16, epoch);
        match snapshotter.capture(&state, &context) {
            TerminalStreamUpdate::Snapshot(viewport) => viewport,
            other => panic!("expected a snapshot, got {other:?}"),
        }
    }

    fn capture(bytes: &[u8]) -> SemanticViewport {
        capture_sized(bytes, COLS, ROWS, 1)
    }

    /// The parts of a viewport a repaint must reproduce.
    fn visible(viewport: &SemanticViewport, synced: u32) -> String {
        let mut out = String::new();
        for row in &viewport.rows {
            out.push_str(&format!("row {}:", row.viewport_row));
            for span in &row.spans {
                let mut style = span.style;
                style.flags &= !STYLE_WRAPLINE;
                out.push_str(&format!(
                    " [{} w{} {:?} {:?} {:?}]",
                    span.start_column, span.width, span.text, style, span.hyperlink
                ));
            }
            out.push('\n');
        }
        let cursor = viewport.cursor;
        out.push_str(&format!(
            "cursor {},{} {:?} visible={}\n",
            cursor.row, cursor.column, cursor.shape, cursor.visible
        ));
        out.push_str(&format!("modes {:#x}\n", viewport.modes.bits & synced));
        out.push_str(&format!("palette {:?}\n", viewport.palette.colors));
        out
    }

    fn base_modes() -> u32 {
        (TermMode::APP_CURSOR | TermMode::BRACKETED_PASTE | TermMode::APP_KEYPAD).bits()
    }

    fn assert_round_trip(source: &[u8]) -> Vec<u8> {
        let original = capture(source);
        let bytes = VtEncoder::new().encode(&original);
        let repainted = capture(&bytes);
        assert_eq!(
            visible(&repainted, base_modes()),
            visible(&original, base_modes()),
            "repaint bytes: {:?}",
            String::from_utf8_lossy(&bytes)
        );
        bytes
    }

    /// Rows named by every `CSI row;col H` in `bytes`, in order.
    fn cup_rows(bytes: &[u8]) -> Vec<u16> {
        let text = String::from_utf8_lossy(bytes);
        let mut rows = Vec::new();
        let mut rest = text.as_ref();
        while let Some(start) = rest.find("\x1b[") {
            rest = &rest[start + 2..];
            let end = rest
                .find(|c: char| !(c.is_ascii_digit() || c == ';'))
                .unwrap_or(rest.len());
            if rest[end..].starts_with('H') {
                let params = &rest[..end];
                let row = params.split(';').next().unwrap_or("");
                rows.push(row.parse().unwrap_or(1));
            }
        }
        rows
    }

    #[test]
    fn wire_bits_match_the_host() {
        use yttt_terminal_core::semantic as host;
        for (local, host) in [
            (wire::STYLE_INVERSE, host::STYLE_INVERSE),
            (wire::STYLE_BOLD, host::STYLE_BOLD),
            (wire::STYLE_ITALIC, host::STYLE_ITALIC),
            (wire::STYLE_UNDERLINE, host::STYLE_UNDERLINE),
            (wire::STYLE_DIM, host::STYLE_DIM),
            (wire::STYLE_HIDDEN, host::STYLE_HIDDEN),
            (wire::STYLE_STRIKEOUT, host::STYLE_STRIKEOUT),
            (wire::STYLE_DOUBLE_UNDERLINE, host::STYLE_DOUBLE_UNDERLINE),
            (wire::STYLE_UNDERCURL, host::STYLE_UNDERCURL),
            (wire::STYLE_DOTTED_UNDERLINE, host::STYLE_DOTTED_UNDERLINE),
            (wire::STYLE_DASHED_UNDERLINE, host::STYLE_DASHED_UNDERLINE),
        ] {
            assert_eq!(local, host);
        }
        for (local, host) in [
            (wire::TermMode::APP_CURSOR, TermMode::APP_CURSOR),
            (wire::TermMode::APP_KEYPAD, TermMode::APP_KEYPAD),
            (
                wire::TermMode::MOUSE_REPORT_CLICK,
                TermMode::MOUSE_REPORT_CLICK,
            ),
            (wire::TermMode::BRACKETED_PASTE, TermMode::BRACKETED_PASTE),
            (wire::TermMode::SGR_MOUSE, TermMode::SGR_MOUSE),
            (wire::TermMode::MOUSE_MOTION, TermMode::MOUSE_MOTION),
            (wire::TermMode::FOCUS_IN_OUT, TermMode::FOCUS_IN_OUT),
            (wire::TermMode::MOUSE_DRAG, TermMode::MOUSE_DRAG),
            (wire::TermMode::MOUSE_MODE, TermMode::MOUSE_MODE),
        ] {
            assert_eq!(local.bits(), host.bits());
        }
    }

    #[test]
    fn plain_rows_round_trip() {
        assert_round_trip(b"hello\r\nworld\r\n\r\nlast line here");
    }

    #[test]
    fn a_full_width_row_keeps_its_last_column() {
        let bytes = assert_round_trip(b"abcdefghijklmnopqrst\r\nnext");
        let text = String::from_utf8_lossy(&bytes);
        assert!(!text.contains("\x1b[K") && !text.contains("\x1b[0K"));
    }

    #[test]
    fn stale_cells_are_cleared_when_a_row_shrinks() {
        let mut encoder = VtEncoder::new();
        let long = capture(b"a long first line");
        let short = capture(b"short");
        let mut component = TerminalState::new(COLS, ROWS, VoidListener);
        component.process_bytes(&encoder.encode(&long));
        component.process_bytes(&encoder.encode(&short));
        let mut snapshotter = SemanticSnapshotter::new(TerminalSessionId::new("t"), 1);
        let context = SemanticCaptureContext::running(COLS as u16, ROWS as u16, 1);
        let TerminalStreamUpdate::Snapshot(repainted) = snapshotter.capture(&component, &context)
        else {
            panic!("expected a snapshot");
        };
        assert_eq!(
            visible(&repainted, base_modes()),
            visible(&short, base_modes())
        );
    }

    #[test]
    fn colors_round_trip() {
        assert_round_trip(
            b"\x1b[31mred\x1b[0m \x1b[92;44mbright\x1b[0m \x1b[38;5;208;48;5;17mindexed\x1b[0m\r\n\
              \x1b[38;2;10;20;30;48;2;200;100;50mrgb\x1b[0m default",
        );
    }

    #[test]
    fn style_flags_round_trip() {
        assert_round_trip(
            b"\x1b[1mbold\x1b[0m \x1b[2mdim\x1b[0m \x1b[3mit\x1b[0m \x1b[4mul\x1b[0m\r\n\
              \x1b[4:2mdbl\x1b[0m \x1b[4:3mcurl\x1b[0m \x1b[4:4mdot\x1b[0m \x1b[4:5mdash\x1b[0m\r\n\
              \x1b[7minv\x1b[0m \x1b[8mhid\x1b[0m \x1b[9mstrike\x1b[0m",
        );
    }

    #[test]
    fn underline_color_round_trips_and_is_omitted_by_default() {
        assert_round_trip(b"\x1b[4;58;2;255;0;0mred line\x1b[0m \x1b[4;58;5;33mblue\x1b[0m");
        let bytes = assert_round_trip(b"\x1b[4;31mplain underline\x1b[0m");
        assert!(!String::from_utf8_lossy(&bytes).contains("58;"));
    }

    #[test]
    fn wide_and_combining_characters_round_trip() {
        assert_round_trip("中文宽字符 ok\r\ne\u{301}t\u{e9} 😀!".as_bytes());
    }

    #[test]
    fn cursor_position_shape_and_visibility_round_trip() {
        assert_round_trip(b"prompt $ \x1b[3;5H\x1b[6 q");
        assert_round_trip(b"busy\x1b[?25l");
        assert_round_trip(b"\x1b[4 qunder");
    }

    #[test]
    fn input_modes_round_trip() {
        assert_round_trip(b"\x1b[?1h\x1b[?2004h\x1b=app modes");
    }

    #[test]
    fn mouse_and_focus_modes_wait_for_control() {
        let source = b"\x1b[?1002h\x1b[?1006h\x1b[?1004hmouse";
        let original = capture(source);
        let mut encoder = VtEncoder::new();
        let observing = String::from_utf8_lossy(&encoder.encode(&original)).into_owned();
        for mode in ["?1000", "?1002", "?1003", "?1006", "?1004"] {
            assert!(
                !observing.contains(&format!("{mode}h")),
                "{mode} set while observing"
            );
        }
        encoder.set_interactive(true);
        let controlling = encoder.encode(&original);
        let mut component = TerminalState::new(COLS, ROWS, VoidListener);
        component.process_bytes(observing.as_bytes());
        component.process_bytes(&controlling);
        let mouse = TermMode::MOUSE_DRAG | TermMode::SGR_MOUSE | TermMode::FOCUS_IN_OUT;
        assert!(component.mode().contains(mouse));
        encoder.set_interactive(false);
        component.process_bytes(&encoder.encode(&original));
        assert!(
            !component
                .mode()
                .intersects(TermMode::MOUSE_MODE | TermMode::FOCUS_IN_OUT)
        );
    }

    #[test]
    fn palette_overrides_round_trip() {
        assert_round_trip(b"\x1b]4;1;rgb:ff/80/00\x1b\\\x1b]11;rgb:10/10/10\x1b\\\x1b[31mtinted");
    }

    #[test]
    fn hyperlinks_round_trip() {
        assert_round_trip(b"see \x1b]8;;https://example.com/a\x1b\\link\x1b]8;;\x1b\\ here");
    }

    #[test]
    fn output_disables_autowrap_first() {
        let bytes = VtEncoder::new().encode(&capture(b"x"));
        assert!(bytes.starts_with(b"\x1b[?7l"));
    }

    #[test]
    fn an_unchanged_viewport_encodes_nothing() {
        let viewport = capture(b"same");
        let mut encoder = VtEncoder::new();
        assert!(!encoder.encode(&viewport).is_empty());
        assert!(encoder.encode(&viewport).is_empty());
    }

    #[test]
    fn a_one_row_change_repaints_only_that_row() {
        let mut encoder = VtEncoder::new();
        encoder.encode(&capture(b"one\r\ntwo\r\nthree"));
        let bytes = encoder.encode(&capture(b"one\r\nTWO!\r\nthree"));
        let mut rows = cup_rows(&bytes);
        rows.pop(); // the trailing cursor position
        rows.dedup();
        assert_eq!(rows, vec![2], "{:?}", String::from_utf8_lossy(&bytes));
    }

    #[test]
    fn size_and_epoch_changes_clear_the_screen() {
        let mut encoder = VtEncoder::new();
        assert!(contains(&encoder.encode(&capture(b"a")), b"\x1b[2J"));
        assert!(!contains(&encoder.encode(&capture(b"ab")), b"\x1b[2J"));
        assert!(contains(
            &encoder.encode(&capture_sized(b"ab", COLS, ROWS, 2)),
            b"\x1b[2J"
        ));
        assert!(contains(
            &encoder.encode(&capture_sized(b"ab", 30, ROWS, 2)),
            b"\x1b[2J"
        ));
        let mut next_session = capture_sized(b"ab", 30, ROWS, 2);
        next_session.session_epoch += 1;
        assert!(contains(&encoder.encode(&next_session), b"\x1b[2J"));
    }

    fn contains(haystack: &[u8], needle: &[u8]) -> bool {
        haystack
            .windows(needle.len())
            .any(|window| window == needle)
    }
}
