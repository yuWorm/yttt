use std::{
    collections::{BTreeMap, hash_map::DefaultHasher},
    hash::{Hash, Hasher},
};

use alacritty_terminal::{
    event::EventListener,
    grid::Dimensions as _,
    index::{Column, Line, Point},
    term::{
        self, Term, TermMode,
        cell::{Cell, Flags},
    },
    vte::ansi::{Color, CursorShape as AlacrittyCursorShape, NamedColor},
};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::terminal::SemanticColor::{Indexed, Named, Rgb as SemanticRgb};
use yttt_protocol::terminal::TerminalStreamUpdate::{Delta, Snapshot};
use yttt_protocol::terminal::{
    CursorShape, DynamicColor, SemanticColor, SemanticCursor, SemanticDelta, SemanticRow,
    SemanticSpan, SemanticStyle, SemanticViewport, TerminalCheckpoint, TerminalGeometry,
    TerminalModes, TerminalPalette, TerminalProcessState, TerminalSearchMatch,
    TerminalSearchResults, TerminalStreamUpdate, TerminalViewportAnchor,
};

use crate::TerminalState;

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SemanticCaptureContext {
    pub geometry: TerminalGeometry,
    pub geometry_epoch: u64,
    pub palette_revision: u64,
    pub title: Option<String>,
    pub cwd: Option<String>,
    pub process_state: TerminalProcessState,
}

impl SemanticCaptureContext {
    pub fn running(cols: u16, rows: u16, geometry_epoch: u64) -> Self {
        Self {
            geometry: TerminalGeometry {
                cols,
                rows,
                cell_width: 0,
                cell_height: 0,
            },
            geometry_epoch,
            palette_revision: 0,
            title: None,
            cwd: None,
            process_state: TerminalProcessState::Running,
        }
    }
}

pub struct SemanticSnapshotter {
    session_id: TerminalSessionId,
    session_epoch: u64,
    sequence: u64,
    scrollback_epoch: u64,
    logical_line_offset: i64,
    next_line_id: u64,
    line_ids: BTreeMap<i64, u64>,
    previous: Option<SemanticViewport>,
    previous_history_size: usize,
    previous_bottom_fingerprints: Vec<u64>,
    previous_geometry_epoch: Option<u64>,
    previous_alt_screen: Option<bool>,
}

impl SemanticSnapshotter {
    pub fn new(session_id: TerminalSessionId, session_epoch: u64) -> Self {
        Self {
            session_id,
            session_epoch,
            sequence: 0,
            scrollback_epoch: 1,
            logical_line_offset: 0,
            next_line_id: 1,
            line_ids: BTreeMap::new(),
            previous: None,
            previous_history_size: 0,
            previous_bottom_fingerprints: Vec::new(),
            previous_geometry_epoch: None,
            previous_alt_screen: None,
        }
    }

    pub fn capture<L: EventListener>(
        &mut self,
        state: &TerminalState<L>,
        context: &SemanticCaptureContext,
    ) -> TerminalStreamUpdate {
        let mut captured = state.with_term_mut(capture_term);
        self.sequence = self.sequence.saturating_add(1);
        let geometry_changed = self
            .previous_geometry_epoch
            .is_some_and(|epoch| epoch != context.geometry_epoch);
        let alternate_changed = self
            .previous_alt_screen
            .is_some_and(|alternate| alternate != captured.alt_screen);
        let history_reset = captured.history_size < self.previous_history_size;
        if geometry_changed || alternate_changed || history_reset {
            self.scrollback_epoch = self.scrollback_epoch.saturating_add(1);
            self.line_ids.clear();
            self.logical_line_offset = captured.history_size as i64;
        } else if self.previous.is_none() {
            self.logical_line_offset = captured.history_size as i64;
        } else if captured.history_size > self.previous_history_size {
            self.logical_line_offset += captured
                .history_size
                .saturating_sub(self.previous_history_size)
                as i64;
        } else if captured.display_offset == 0
            && self
                .previous
                .as_ref()
                .is_some_and(|frame| frame.display_offset == 0)
            && let Some(shift) =
                detect_upward_shift(&self.previous_bottom_fingerprints, &captured.fingerprints)
        {
            self.logical_line_offset += shift as i64;
        }

        let history_size = captured.history_size;
        let display_offset = captured.display_offset;
        let fingerprints = std::mem::take(&mut captured.fingerprints);
        let alt_screen = captured.alt_screen;
        let viewport = self.viewport_from_captured(captured, context, self.sequence);

        let update = match self.previous.as_ref() {
            Some(previous)
                if previous.geometry_epoch == viewport.geometry_epoch
                    && previous.scrollback_epoch == viewport.scrollback_epoch =>
            {
                Delta(delta_between(previous, &viewport))
            }
            _ => Snapshot(viewport.clone()),
        };
        self.previous_history_size = history_size;
        if display_offset == 0 {
            self.previous_bottom_fingerprints = fingerprints;
        }
        self.previous_geometry_epoch = Some(context.geometry_epoch);
        self.previous_alt_screen = Some(alt_screen);
        self.previous = Some(viewport);
        update
    }

    pub fn latest_viewport(&self) -> Option<&SemanticViewport> {
        self.previous.as_ref()
    }

    pub fn checkpoint(
        &self,
        raw_replay_tail: Vec<u8>,
        raw_tail_start_sequence: u64,
    ) -> Option<TerminalCheckpoint> {
        self.previous.clone().map(|viewport| TerminalCheckpoint {
            viewport,
            raw_replay_tail,
            raw_tail_start_sequence,
        })
    }
    pub fn read_viewport<L: EventListener>(
        &mut self,
        state: &TerminalState<L>,
        context: &SemanticCaptureContext,
        scrollback_epoch: u64,
        anchor: TerminalViewportAnchor,
    ) -> Result<SemanticViewport, SemanticAccessError> {
        self.validate_scrollback_epoch(scrollback_epoch)?;
        let history_size = self.previous.as_ref().map_or(0, |frame| frame.history_size);
        let display_offset = match anchor {
            TerminalViewportAnchor::Bottom => 0,
            TerminalViewportAnchor::DisplayOffset(offset) => offset.min(history_size),
            TerminalViewportAnchor::LineId(line_id) => {
                let logical_line = self
                    .line_ids
                    .iter()
                    .find_map(|(logical_line, candidate)| {
                        (*candidate == line_id).then_some(*logical_line)
                    })
                    .ok_or(SemanticAccessError::UnknownLineId(line_id))?;
                u64::try_from(self.logical_line_offset.saturating_sub(logical_line))
                    .unwrap_or_default()
                    .min(history_size)
            }
        };
        let captured = state.with_term_mut(|term| {
            capture_term_with_offset(
                term,
                Some(usize::try_from(display_offset).unwrap_or(usize::MAX)),
            )
        });
        Ok(self.viewport_from_captured(captured, context, self.sequence))
    }

    pub fn search<L: EventListener>(
        &mut self,
        state: &TerminalState<L>,
        scrollback_epoch: u64,
        generation: u64,
        query: &str,
        case_sensitive: bool,
        max_results: u16,
    ) -> Result<TerminalSearchResults, SemanticAccessError> {
        self.validate_scrollback_epoch(scrollback_epoch)?;
        if query.is_empty() || max_results == 0 {
            return Err(SemanticAccessError::InvalidSearch);
        }
        let needle = if case_sensitive {
            query.to_string()
        } else {
            query.to_ascii_lowercase()
        };
        let limit = usize::from(max_results);
        let mut matches = Vec::with_capacity(limit.min(64));
        let mut truncated = false;
        state.with_term_mut(|term| {
            let columns = term.columns();
            let screen_lines = term.screen_lines();
            let history_size = term.grid().total_lines().saturating_sub(screen_lines);
            for grid_line in -(history_size as i32)..screen_lines as i32 {
                let text = plain_text_row(term, Line(grid_line), columns);
                let haystack = if case_sensitive {
                    text.clone()
                } else {
                    text.to_ascii_lowercase()
                };
                for (byte_start, found) in haystack.match_indices(&needle) {
                    if matches.len() == limit {
                        truncated = true;
                        return;
                    }
                    let byte_end = byte_start.saturating_add(found.len());
                    let start_column =
                        u16::try_from(text[..byte_start].chars().count()).unwrap_or(u16::MAX);
                    let end_column =
                        u16::try_from(text[..byte_end].chars().count()).unwrap_or(u16::MAX);
                    let logical_line = self.logical_line_offset + i64::from(grid_line);
                    matches.push(TerminalSearchMatch {
                        line_id: self.line_id_for_key(logical_line),
                        start_column,
                        end_column,
                    });
                }
            }
        });
        Ok(TerminalSearchResults {
            session_id: self.session_id.clone(),
            session_epoch: self.session_epoch,
            scrollback_epoch,
            generation,
            matches,
            truncated,
        })
    }

    fn validate_scrollback_epoch(&self, received: u64) -> Result<(), SemanticAccessError> {
        if received == self.scrollback_epoch {
            Ok(())
        } else {
            Err(SemanticAccessError::StaleScrollback {
                received,
                current: self.scrollback_epoch,
            })
        }
    }

    fn line_id_for_key(&mut self, key: i64) -> u64 {
        match self.line_ids.get(&key).copied() {
            Some(line_id) => line_id,
            None => {
                let line_id = self.next_line_id;
                self.next_line_id = self.next_line_id.saturating_add(1);
                self.line_ids.insert(key, line_id);
                line_id
            }
        }
    }

    fn viewport_from_captured(
        &mut self,
        captured: CapturedTerminal,
        context: &SemanticCaptureContext,
        sequence: u64,
    ) -> SemanticViewport {
        let mut rows = Vec::with_capacity(captured.rows.len());
        for raw in captured.rows {
            let key = self.logical_line_offset + i64::from(raw.grid_line);
            rows.push(SemanticRow {
                line_id: self.line_id_for_key(key),
                viewport_row: raw.viewport_row,
                spans: raw.spans,
            });
        }
        self.prune_line_ids(captured.history_size, captured.screen_lines);

        let mut geometry = context.geometry;
        geometry.cols = u16::try_from(captured.columns).unwrap_or(u16::MAX);
        geometry.rows = u16::try_from(captured.screen_lines).unwrap_or(u16::MAX);
        SemanticViewport {
            session_id: self.session_id.clone(),
            session_epoch: self.session_epoch,
            sequence,
            geometry,
            geometry_epoch: context.geometry_epoch,
            scrollback_epoch: self.scrollback_epoch,
            history_size: captured.history_size as u64,
            display_offset: captured.display_offset as u64,
            rows,
            cursor: captured.cursor,
            modes: TerminalModes {
                bits: captured.mode_bits,
                title: context.title.clone(),
                cwd: context.cwd.clone(),
            },
            palette: TerminalPalette {
                colors: captured.palette,
                revision: context.palette_revision,
            },
            process_state: context.process_state,
        }
    }

    pub fn session_epoch(&self) -> u64 {
        self.session_epoch
    }

    fn prune_line_ids(&mut self, history_size: usize, screen_lines: usize) {
        let minimum = self.logical_line_offset - history_size as i64 - screen_lines as i64;
        let maximum = self.logical_line_offset + screen_lines as i64;
        self.line_ids
            .retain(|line, _| *line >= minimum && *line <= maximum);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SemanticAccessError {
    StaleScrollback { received: u64, current: u64 },
    UnknownLineId(u64),
    InvalidSearch,
}

struct CapturedTerminal {
    columns: usize,
    screen_lines: usize,
    history_size: usize,
    display_offset: usize,
    rows: Vec<RawSemanticRow>,
    fingerprints: Vec<u64>,
    cursor: SemanticCursor,
    mode_bits: u32,
    alt_screen: bool,
    palette: Vec<DynamicColor>,
}

struct RawSemanticRow {
    grid_line: i32,
    viewport_row: u16,
    spans: Vec<SemanticSpan>,
}

fn capture_term<L: EventListener>(term: &mut Term<L>) -> CapturedTerminal {
    capture_term_with_offset(term, None)
}

fn capture_term_with_offset<L: EventListener>(
    term: &mut Term<L>,
    requested_display_offset: Option<usize>,
) -> CapturedTerminal {
    let columns = term.columns();
    let screen_lines = term.screen_lines();
    let (canonical_display_offset, cursor, colors, mode) = {
        let content = term.renderable_content();
        (
            content.display_offset,
            content.cursor,
            *content.colors,
            content.mode,
        )
    };
    let history_size = term.grid().total_lines().saturating_sub(screen_lines);
    let display_offset = requested_display_offset
        .unwrap_or(canonical_display_offset)
        .min(history_size);
    let cursor_viewport = term::point_to_viewport(display_offset, cursor.point);
    let cursor = SemanticCursor {
        row: cursor_viewport
            .map(|point| u16::try_from(point.line).unwrap_or(u16::MAX))
            .unwrap_or(0),
        column: cursor_viewport
            .map(|point| u16::try_from(point.column.0).unwrap_or(u16::MAX))
            .unwrap_or(0),
        shape: semantic_cursor_shape(cursor.shape),
        visible: cursor_viewport.is_some() && cursor.shape != AlacrittyCursorShape::Hidden,
        blinking: false,
    };

    let mut rows = Vec::with_capacity(screen_lines);
    let mut fingerprints = Vec::with_capacity(screen_lines);
    let grid = term.grid();
    for viewport_row in 0..screen_lines {
        let line = Line(viewport_row as i32 - display_offset as i32);
        let mut spans = Vec::new();
        for column in 0..columns {
            let point = Point::new(line, Column(column));
            let cell = &grid[point];
            if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                continue;
            }
            append_semantic_cell(&mut spans, column, cell);
        }
        let fingerprint = fingerprint_spans(&spans);
        fingerprints.push(fingerprint);
        rows.push(RawSemanticRow {
            grid_line: line.0,
            viewport_row: u16::try_from(viewport_row).unwrap_or(u16::MAX),
            spans,
        });
    }

    CapturedTerminal {
        columns,
        screen_lines,
        history_size,
        display_offset,
        rows,
        fingerprints,
        cursor,
        mode_bits: mode.bits(),
        alt_screen: mode.contains(TermMode::ALT_SCREEN),
        palette: dynamic_palette(&colors),
    }
}

fn plain_text_row<L: EventListener>(term: &Term<L>, line: Line, columns: usize) -> String {
    let grid = term.grid();
    let mut text = String::with_capacity(columns);
    for column in 0..columns {
        let cell = &grid[Point::new(line, Column(column))];
        if cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
            continue;
        }
        append_cell_text(&mut text, cell);
    }
    text
}

fn append_semantic_cell(spans: &mut Vec<SemanticSpan>, column: usize, cell: &Cell) {
    let width = if cell.flags.contains(Flags::WIDE_CHAR) {
        2
    } else {
        1
    };
    let style = semantic_style(cell);
    let hyperlink = cell.hyperlink();
    let hyperlink_uri = hyperlink.as_ref().map(|hyperlink| hyperlink.uri());
    if let Some(previous) = spans.last_mut()
        && can_merge(previous, column, &style, hyperlink_uri)
    {
        append_cell_text(&mut previous.text, cell);
        previous.width = previous.width.saturating_add(width);
        return;
    }

    let mut text = String::new();
    append_cell_text(&mut text, cell);
    spans.push(SemanticSpan {
        start_column: u16::try_from(column).unwrap_or(u16::MAX),
        text,
        width,
        style,
        hyperlink: hyperlink_uri.map(str::to_owned),
    });
}

fn append_cell_text(text: &mut String, cell: &Cell) {
    if cell.flags.contains(Flags::HIDDEN) || cell.c == '\0' {
        text.push(' ');
    } else {
        text.push(cell.c);
        if let Some(zerowidth) = cell.zerowidth() {
            text.extend(zerowidth.iter());
        }
    }
}

fn semantic_style(cell: &Cell) -> SemanticStyle {
    SemanticStyle {
        foreground: semantic_color(cell.fg),
        background: semantic_color(cell.bg),
        flags: semantic_flags(cell.flags),
        underline_color: cell
            .underline_color()
            .map(semantic_color)
            .unwrap_or_else(|| semantic_color(cell.fg)),
    }
}

fn can_merge(
    previous: &SemanticSpan,
    column: usize,
    style: &SemanticStyle,
    hyperlink: Option<&str>,
) -> bool {
    previous.style == *style
        && previous.hyperlink.as_deref() == hyperlink
        && usize::from(previous.start_column.saturating_add(previous.width)) == column
}

fn semantic_flags(flags: Flags) -> u16 {
    let mut semantic = 0;
    for (flag, value) in [
        (Flags::INVERSE, STYLE_INVERSE),
        (Flags::BOLD, STYLE_BOLD),
        (Flags::ITALIC, STYLE_ITALIC),
        (Flags::UNDERLINE, STYLE_UNDERLINE),
        (Flags::DIM, STYLE_DIM),
        (Flags::HIDDEN, STYLE_HIDDEN),
        (Flags::STRIKEOUT, STYLE_STRIKEOUT),
        (Flags::DOUBLE_UNDERLINE, STYLE_DOUBLE_UNDERLINE),
        (Flags::UNDERCURL, STYLE_UNDERCURL),
        (Flags::DOTTED_UNDERLINE, STYLE_DOTTED_UNDERLINE),
        (Flags::DASHED_UNDERLINE, STYLE_DASHED_UNDERLINE),
    ] {
        if flags.contains(flag) {
            semantic |= value;
        }
    }
    semantic
}

fn semantic_color(color: Color) -> SemanticColor {
    match color {
        Color::Named(named) => Named(named as u16),
        Color::Indexed(index) => Indexed(index),
        Color::Spec(rgb) => SemanticRgb {
            red: rgb.r,
            green: rgb.g,
            blue: rgb.b,
        },
    }
}

fn semantic_cursor_shape(shape: AlacrittyCursorShape) -> CursorShape {
    match shape {
        AlacrittyCursorShape::Block | AlacrittyCursorShape::Hidden => CursorShape::Block,
        AlacrittyCursorShape::Underline => CursorShape::Underline,
        AlacrittyCursorShape::Beam => CursorShape::Beam,
        AlacrittyCursorShape::HollowBlock => CursorShape::HollowBlock,
    }
}

fn dynamic_palette(colors: &alacritty_terminal::term::color::Colors) -> Vec<DynamicColor> {
    let mut dynamic = Vec::new();
    for index in 0..=NamedColor::DimForeground as usize {
        if let Some(rgb) = colors[index] {
            dynamic.push(DynamicColor {
                index: u16::try_from(index).unwrap_or(u16::MAX),
                red: rgb.r,
                green: rgb.g,
                blue: rgb.b,
            });
        }
    }
    dynamic
}

fn fingerprint_spans(spans: &[SemanticSpan]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for span in spans {
        span.start_column.hash(&mut hasher);
        span.text.hash(&mut hasher);
        span.width.hash(&mut hasher);
        color_hash(span.style.foreground, &mut hasher);
        color_hash(span.style.background, &mut hasher);
        span.style.flags.hash(&mut hasher);
        color_hash(span.style.underline_color, &mut hasher);
        span.hyperlink.hash(&mut hasher);
    }
    hasher.finish()
}

fn color_hash(color: SemanticColor, hasher: &mut impl Hasher) {
    match color {
        Named(index) => {
            0_u8.hash(hasher);
            index.hash(hasher);
        }
        Indexed(index) => {
            1_u8.hash(hasher);
            index.hash(hasher);
        }
        SemanticRgb { red, green, blue } => {
            2_u8.hash(hasher);
            red.hash(hasher);
            green.hash(hasher);
            blue.hash(hasher);
        }
    }
}

fn detect_upward_shift(previous: &[u64], current: &[u64]) -> Option<usize> {
    if previous.len() != current.len() || previous == current {
        return None;
    }
    (1..previous.len()).find(|shift| previous[*shift..] == current[..current.len() - *shift])
}

fn delta_between(previous: &SemanticViewport, current: &SemanticViewport) -> SemanticDelta {
    let changed_rows = current
        .rows
        .iter()
        .filter(|row| {
            previous
                .rows
                .iter()
                .find(|previous_row| previous_row.viewport_row == row.viewport_row)
                != Some(*row)
        })
        .cloned()
        .collect();
    SemanticDelta {
        session_id: current.session_id.clone(),
        session_epoch: current.session_epoch,
        base_sequence: previous.sequence,
        sequence: current.sequence,
        geometry_epoch: current.geometry_epoch,
        scrollback_epoch: current.scrollback_epoch,
        history_size: current.history_size,
        display_offset: current.display_offset,
        changed_rows,
        cursor: (previous.cursor != current.cursor).then_some(current.cursor),
        modes: (previous.modes != current.modes).then(|| current.modes.clone()),
        palette: (previous.palette != current.palette).then(|| current.palette.clone()),
        process_state: (previous.process_state != current.process_state)
            .then_some(current.process_state),
    }
}
