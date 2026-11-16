//! The apps' copy mode: a terminal's text, with the Host's history on the
//! primary screen, and the keys that page a program that scrolls itself.

use yttt_protocol::terminal::{SemanticRow, SemanticSpan};

use crate::vt::wire::{STYLE_WRAPLINE, TermMode};

/// Plain text of a terminal for copying.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TerminalText {
    /// Logical lines, oldest first: soft-wrapped rows joined, trailing
    /// blanks trimmed.
    pub lines: Vec<String>,
    /// The last line goes on in the first line of the newer text after it.
    pub last_line_continues: bool,
    /// Where `older_text` reads on; `None` at the start of the history and
    /// on the alternate screen, which has none.
    pub older: Option<TextCursor>,
    /// The program draws on the alternate screen and scrolls by itself.
    pub alternate_screen: bool,
    /// For each screen row, top first, the index in `lines` of the line it
    /// shows; empty for history alone.
    pub row_lines: Vec<u32>,
}

/// What paging a program up one screen added above the text read so far.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct PagedText {
    /// Lines that go above the text so far, oldest first.
    pub above: Vec<String>,
    /// The fixed lines at the top of the program's screen (a title, say),
    /// which the text so far still starts with after the first page.
    pub header: u32,
    /// The pages did not overlap: something may be missing between them.
    pub gap: bool,
    /// The program did not scroll: its start is reached.
    pub at_top: bool,
    /// The screen now; pass it back with the next page.
    pub screen: Vec<String>,
}

/// The top of the text read so far.
#[derive(Clone, Debug, PartialEq, Eq, uniffi::Record)]
pub struct TextCursor {
    pub session_epoch: u64,
    pub scrollback_epoch: u64,
    pub top_line_id: u64,
}

/// Whether the mode bits put the program on the alternate screen.
pub(crate) fn alternate_screen(mode_bits: u32) -> bool {
    TermMode::from_bits_truncate(mode_bits).contains(TermMode::ALT_SCREEN)
}

/// One row's text, its spans placed at their columns, and whether it wraps
/// into the next row.
pub(crate) fn row_text(row: &SemanticRow) -> (String, bool) {
    let mut spans: Vec<&SemanticSpan> = row.spans.iter().collect();
    spans.sort_by_key(|span| span.start_column);
    let mut text = String::new();
    let mut column = 0u16;
    let mut wraps = false;
    for span in spans {
        if span.start_column > column {
            text.extend(std::iter::repeat_n(
                ' ',
                usize::from(span.start_column - column),
            ));
        }
        text.push_str(&span.text);
        column = column.max(span.start_column.saturating_add(span.width));
        wraps |= span.style.flags & STYLE_WRAPLINE != 0;
    }
    (text, wraps)
}

/// Rows, oldest first, as logical lines; also whether the last one wraps.
pub(crate) fn lines(rows: &[SemanticRow]) -> (Vec<String>, bool) {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut continues = false;
    for row in rows {
        let (text, wraps) = row_text(row);
        if wraps {
            // A wrapped row is full width: its trailing blanks are text.
            current.push_str(&text);
        } else {
            current.push_str(text.trim_end());
            lines.push(std::mem::take(&mut current));
        }
        continues = wraps;
    }
    if continues {
        lines.push(current);
    }
    (lines, continues)
}

/// For each row, the index of the logical line `lines` makes it part of.
pub(crate) fn row_lines(rows: &[SemanticRow]) -> Vec<u32> {
    let mut line = 0u32;
    rows.iter()
        .map(|row| {
            let index = line;
            if !row_text(row).1 {
                line += 1;
            }
            index
        })
        .collect()
}

/// Stitches the screen a program shows after paging up (`older`) above the
/// one before it (`newer`). The content moved down by some rows: the shift
/// that lines up the most non-blank lines. Lines equal in place at the top
/// are the program's frame (a title); its bottom (an input box, a status
/// line) may change from page to page and is not trusted.
pub fn stitch(older: &[String], newer: &[String]) -> Stitch {
    let blank = |line: &String| line.trim().is_empty();
    // Only a status line or two changed: the program did not scroll.
    let changed_rows = (0..older.len().max(newer.len()))
        .filter(|&row| older.get(row) != newer.get(row))
        .count();
    if changed_rows <= (newer.len() / 8).max(1) {
        return Stitch::unchanged();
    }
    let header = older.iter().zip(newer).take_while(|(a, b)| a == b).count();
    // Rows moved down by `shift`: older[row + shift] shows newer[row].
    let score = |shift: usize| {
        (header..newer.len())
            .filter(|&row| {
                older
                    .get(row + shift)
                    .is_some_and(|line| *line == newer[row] && !blank(line))
            })
            .count()
    };
    let best = (1..older.len())
        .map(|shift| (score(shift), shift))
        .max_by(|a, b| a.0.cmp(&b.0).then(b.1.cmp(&a.1)));
    match best {
        Some((matched, shift)) if matched > 0 => Stitch {
            header: header as u32,
            above: older[header..(header + shift).min(older.len())].to_vec(),
            gap: false,
            unchanged: false,
        },
        // No line in common: the program moved a whole screen or more.
        // Its frame's bottom is whatever stays equal at the end.
        _ => {
            let footer = older
                .iter()
                .rev()
                .zip(newer.iter().rev())
                .take_while(|(a, b)| a == b)
                .count();
            let end = older.len().saturating_sub(footer).max(header);
            Stitch {
                header: header as u32,
                above: older[header..end].to_vec(),
                gap: true,
                unchanged: false,
            }
        }
    }
}

/// The result of `stitch`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Stitch {
    pub header: u32,
    pub above: Vec<String>,
    pub gap: bool,
    pub unchanged: bool,
}

impl Stitch {
    fn unchanged() -> Self {
        Self {
            header: 0,
            above: Vec::new(),
            gap: false,
            unchanged: true,
        }
    }
}

/// Drops the blank lines at the bottom of a screen.
pub(crate) fn trim_trailing_blank(lines: &mut Vec<String>) {
    while lines.last().is_some_and(|line| line.trim().is_empty()) {
        lines.pop();
    }
}

/// The rows of a block read with the previous top line at its bottom: the
/// ones above that line. `None` when the line is not in the block.
pub(crate) fn rows_above(rows: &[SemanticRow], top_line_id: u64) -> Option<&[SemanticRow]> {
    let index = rows.iter().position(|row| row.line_id == top_line_id)?;
    Some(&rows[..index])
}

/// The bytes that scroll a program by `pages` steps, negative up: wheel
/// reports at the middle of the screen when it tracks the mouse, enough for
/// about half a screen at the usual three lines a notch, so pages overlap;
/// Page Up / Page Down otherwise.
pub fn scroll_bytes(mode_bits: u32, cols: u16, rows: u16, pages: i32) -> Vec<u8> {
    let modes = TermMode::from_bits_truncate(mode_bits);
    let up = pages < 0;
    let count = pages.unsigned_abs() as usize;
    let mut out = Vec::new();
    if mode_bits & TermMode::MOUSE_MODE.bits() != 0 {
        let notches = count * usize::from((rows / 6).max(1));
        let button: u16 = if up { 64 } else { 65 };
        let (x, y) = (cols / 2 + 1, rows / 2 + 1);
        for _ in 0..notches {
            if modes.contains(TermMode::SGR_MOUSE) {
                out.extend_from_slice(format!("\x1b[<{button};{x};{y}M").as_bytes());
            } else {
                // The legacy encoding tops out at column and row 223.
                let byte = |value: u16| (32 + value.min(223)) as u8;
                out.extend_from_slice(b"\x1b[M");
                out.extend_from_slice(&[byte(button), byte(x), byte(y)]);
            }
        }
    } else {
        let key: &[u8] = if up { b"\x1b[5~" } else { b"\x1b[6~" };
        for _ in 0..count {
            out.extend_from_slice(key);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_protocol::terminal::{SemanticColor, SemanticStyle};

    fn span(start: u16, text: &str, width: u16, flags: u16) -> SemanticSpan {
        SemanticSpan {
            start_column: start,
            text: text.into(),
            width,
            style: SemanticStyle {
                foreground: SemanticColor::Named(256),
                background: SemanticColor::Named(257),
                flags,
                underline_color: SemanticColor::Named(256),
            },
            hyperlink: None,
        }
    }

    fn row(line_id: u64, spans: Vec<SemanticSpan>) -> SemanticRow {
        SemanticRow {
            line_id,
            viewport_row: 0,
            spans,
            graphics: Vec::new(),
        }
    }

    #[test]
    fn spans_land_at_their_columns() {
        let (text, wraps) = row_text(&row(1, vec![span(4, "b", 1, 0), span(0, "中a", 3, 0)]));
        assert_eq!(text, "中a b");
        assert!(!wraps);
    }

    #[test]
    fn wrapped_rows_join_and_blanks_trim() {
        let rows = [
            row(1, vec![span(0, "abc ", 4, STYLE_WRAPLINE)]),
            row(2, vec![span(0, "def   ", 6, 0)]),
            row(3, vec![]),
            row(4, vec![span(0, "x", 1, 0)]),
        ];
        let (lines, continues) = lines(&rows);
        assert_eq!(lines, ["abc def", "", "x"]);
        assert!(!continues);
    }

    #[test]
    fn a_block_ending_in_a_wrap_says_so() {
        let rows = [row(1, vec![span(0, "abc", 3, STYLE_WRAPLINE)])];
        assert_eq!(lines(&rows), (vec!["abc".to_string()], true));
    }

    #[test]
    fn the_block_above_stops_before_the_known_top() {
        let rows = [row(7, vec![]), row(8, vec![]), row(9, vec![])];
        let above = rows_above(&rows, 9).unwrap();
        assert_eq!(
            above.iter().map(|row| row.line_id).collect::<Vec<_>>(),
            [7, 8]
        );
        assert!(rows_above(&rows, 42).is_none());
    }

    #[test]
    fn rows_map_to_their_lines() {
        let rows = [
            row(1, vec![span(0, "a", 1, STYLE_WRAPLINE)]),
            row(2, vec![span(0, "b", 1, 0)]),
            row(3, vec![span(0, "c", 1, 0)]),
        ];
        assert_eq!(row_lines(&rows), [0, 0, 1]);
    }

    fn strings(lines: &[&str]) -> Vec<String> {
        lines.iter().map(|line| line.to_string()).collect()
    }

    #[test]
    fn pages_stitch_inside_the_programs_frame() {
        // A title on top, an input box at the bottom, content scrolling
        // between them by three lines.
        let newer = strings(&["Title", "c4", "c5", "c6", "c7", "> input", "status"]);
        let older = strings(&["Title", "c1", "c2", "c3", "c4", "> input", "status"]);
        let stitched = stitch(&older, &newer);
        assert_eq!(stitched.header, 1);
        assert_eq!(stitched.above, strings(&["c1", "c2", "c3"]));
        assert!(!stitched.gap);
        assert!(!stitched.unchanged);
    }

    #[test]
    fn pages_without_overlap_leave_a_gap() {
        let newer = strings(&["Title", "c9", "c10", "> input"]);
        let older = strings(&["Title", "c1", "c2", "> input"]);
        let stitched = stitch(&older, &newer);
        assert_eq!(stitched.above, strings(&["c1", "c2"]));
        assert!(stitched.gap);
    }

    #[test]
    fn blank_lines_alone_do_not_count_as_overlap() {
        let newer = strings(&["", "x", "y"]);
        let older = strings(&["a", "b", ""]);
        assert!(stitch(&older, &newer).gap);
    }

    #[test]
    fn a_changing_status_line_is_not_content() {
        let newer = strings(&["c5", "c6", "c7", "c8", "(END)"]);
        let older = strings(&["c3", "c4", "c5", "c6", ":"]);
        let stitched = stitch(&older, &newer);
        assert_eq!(stitched.above, strings(&["c3", "c4"]));
        assert!(!stitched.gap);
    }

    #[test]
    fn a_screen_that_only_changed_its_status_is_the_start() {
        let newer = strings(&["c1", "c2", "c3", "c4", ":"]);
        let older = strings(&["c1", "c2", "c3", "c4", "set mark:"]);
        assert!(stitch(&older, &newer).unchanged);
    }

    #[test]
    fn an_unmoved_screen_is_the_start() {
        let screen = strings(&["a", "b"]);
        assert!(stitch(&screen, &screen).unchanged);
    }

    #[test]
    fn screen_trailing_blank_lines_go() {
        let mut lines = vec!["a".to_string(), "".into(), "  ".into()];
        trim_trailing_blank(&mut lines);
        assert_eq!(lines, ["a"]);
    }

    #[test]
    fn alternate_screen_comes_from_the_mode_bits() {
        assert!(alternate_screen(TermMode::ALT_SCREEN.bits()));
        assert!(!alternate_screen(TermMode::APP_CURSOR.bits()));
    }

    #[test]
    fn a_program_without_the_mouse_gets_page_keys() {
        assert_eq!(scroll_bytes(0, 80, 24, -2), b"\x1b[5~\x1b[5~");
        assert_eq!(scroll_bytes(0, 80, 24, 1), b"\x1b[6~");
    }

    #[test]
    fn a_mouse_program_gets_wheel_notches() {
        let sgr = (TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE).bits();
        // 18 rows: three notches a step, at the middle cell.
        assert_eq!(scroll_bytes(sgr, 80, 18, -1), b"\x1b[<64;41;10M".repeat(3));
        assert_eq!(scroll_bytes(sgr, 80, 18, 1), b"\x1b[<65;41;10M".repeat(3));
        let legacy = TermMode::MOUSE_DRAG.bits();
        assert_eq!(scroll_bytes(legacy, 80, 3, -1), b"\x1b[M\x60\x49\x22");
    }
}
