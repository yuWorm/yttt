use std::collections::{BTreeMap, HashMap};

use alacritty_terminal::{
    index::{Column, Line, Point as AlacPoint, Side},
    selection::{SelectionRange, SelectionType},
};
use yttt_core::model::ids::TerminalSessionId;
use yttt_protocol::terminal::{SemanticRow, SemanticViewport};
use yttt_terminal_core::semantic::STYLE_WRAPLINE;

const BRACKET_PAIRS: [(char, char); 4] = [('(', ')'), ('[', ']'), ('{', '}'), ('<', '>')];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct SelectionAnchor {
    point: AlacPoint,
    side: Side,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum CachedCell {
    Text { text: String, width: usize },
    Spacer,
}

impl CachedCell {
    fn blank() -> Self {
        Self::Text {
            text: " ".to_string(),
            width: 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct CachedRow {
    line_id: u64,
    cells: Vec<CachedCell>,
    wrapped: bool,
}

impl CachedRow {
    fn from_semantic(row: &SemanticRow, columns: usize) -> Self {
        let mut cells = vec![CachedCell::blank(); columns];
        let wrapped = row
            .spans
            .iter()
            .any(|span| span.style.flags & STYLE_WRAPLINE != 0);

        for span in &row.spans {
            let mut column = usize::from(span.start_column).min(columns);
            let span_end = usize::from(span.start_column)
                .saturating_add(usize::from(span.width))
                .min(columns);
            let mut last_text_column = None;

            for character in span.text.chars() {
                let width = if character == '\t' {
                    1
                } else {
                    unicode_width::UnicodeWidthChar::width(character).unwrap_or(1)
                };
                if width == 0 {
                    if let Some(last_text_column) = last_text_column
                        && let Some(CachedCell::Text { text, .. }) = cells.get_mut(last_text_column)
                    {
                        text.push(character);
                    }
                    continue;
                }
                if column >= span_end {
                    break;
                }

                let width = width.min(span_end.saturating_sub(column)).max(1);
                cells[column] = CachedCell::Text {
                    text: character.to_string(),
                    width,
                };
                for spacer in 1..width {
                    cells[column + spacer] = CachedCell::Spacer;
                }
                last_text_column = Some(column);
                column += width;
            }
        }

        Self {
            line_id: row.line_id,
            cells,
            wrapped,
        }
    }

    fn columns(&self) -> usize {
        self.cells.len()
    }

    fn leading_column(&self, mut column: usize) -> usize {
        column = column.min(self.columns().saturating_sub(1));
        while column > 0 && matches!(self.cells[column], CachedCell::Spacer) {
            column -= 1;
        }
        column
    }

    fn cell_width(&self, column: usize) -> usize {
        match self.cells.get(self.leading_column(column)) {
            Some(CachedCell::Text { width, .. }) => *width,
            _ => 1,
        }
    }

    fn character(&self, column: usize) -> Option<char> {
        match self.cells.get(self.leading_column(column))? {
            CachedCell::Text { text, .. } => text.chars().next(),
            CachedCell::Spacer => None,
        }
    }

    fn text_for_columns(&self, start: usize, end: usize) -> String {
        if self.cells.is_empty() || start > end {
            return String::new();
        }

        let mut text = String::new();
        let mut column = self.leading_column(start);
        let end = end.min(self.columns() - 1);
        while column <= end {
            match &self.cells[column] {
                CachedCell::Text {
                    text: cell_text,
                    width,
                } => {
                    if column.saturating_add(*width).saturating_sub(1) >= start {
                        text.push_str(cell_text);
                    }
                    column = column.saturating_add(*width);
                }
                CachedCell::Spacer => column += 1,
            }
        }

        if end + 1 == self.columns() {
            text.truncate(text.trim_end_matches(' ').len());
        }
        text
    }
}

#[derive(Clone, Debug)]
struct SelectionContext {
    session_id: TerminalSessionId,
    session_epoch: u64,
    geometry_epoch: u64,
    scrollback_epoch: u64,
    columns: usize,
}

impl SelectionContext {
    fn new(viewport: &SemanticViewport) -> Self {
        Self {
            session_id: viewport.session_id.clone(),
            session_epoch: viewport.session_epoch,
            geometry_epoch: viewport.geometry_epoch,
            scrollback_epoch: viewport.scrollback_epoch,
            columns: usize::from(viewport.geometry.cols),
        }
    }

    fn matches(&self, viewport: &SemanticViewport) -> bool {
        self.session_id == viewport.session_id
            && self.session_epoch == viewport.session_epoch
            && self.geometry_epoch == viewport.geometry_epoch
            && self.scrollback_epoch == viewport.scrollback_epoch
            && self.columns == usize::from(viewport.geometry.cols)
    }
}

/// Client-local selection over Host semantic rows.
///
/// Selection points are stored in stable grid coordinates. `grid_line_offset` tracks terminal
/// rotation without rewriting a potentially large cached selection whenever new output arrives.
#[derive(Clone, Debug)]
pub(crate) struct SemanticSelection {
    ty: SelectionType,
    start: SelectionAnchor,
    end: SelectionAnchor,
    context: SelectionContext,
    rows: BTreeMap<i32, CachedRow>,
    lines_by_id: HashMap<u64, i32>,
    grid_line_offset: i32,
    last_display_offset: u64,
    last_sequence: u64,
}

impl SemanticSelection {
    pub(crate) fn new(
        viewport: &SemanticViewport,
        point: AlacPoint,
        side: Side,
        ty: SelectionType,
    ) -> Option<Self> {
        let mut selection = Self {
            ty,
            start: SelectionAnchor { point, side },
            end: SelectionAnchor { point, side },
            context: SelectionContext::new(viewport),
            rows: BTreeMap::new(),
            lines_by_id: HashMap::new(),
            grid_line_offset: 0,
            last_display_offset: viewport.display_offset,
            last_sequence: viewport.sequence,
        };
        selection.observe(viewport);
        let point = selection.current_to_stable(point)?;
        if !selection.rows.contains_key(&point.line.0) {
            return None;
        }
        selection.start.point = point;
        selection.end.point = point;
        Some(selection)
    }

    /// Merge a new authoritative viewport into the rows observed during this selection.
    /// Returns `false` when a session, geometry, or untraceable grid replacement invalidates it.
    pub(crate) fn observe(&mut self, viewport: &SemanticViewport) -> bool {
        if !self.context.matches(viewport) {
            return false;
        }

        let viewport_rows = viewport
            .rows
            .iter()
            .map(|row| {
                let grid_line = i32::from(row.viewport_row)
                    .saturating_sub(viewport.display_offset.min(i32::MAX as u64) as i32);
                (
                    grid_line,
                    CachedRow::from_semantic(row, self.context.columns),
                )
            })
            .collect::<Vec<_>>();

        let mut inferred_offset = None;
        for (grid_line, row) in &viewport_rows {
            let Some(stable_line) = self.lines_by_id.get(&row.line_id).copied() else {
                continue;
            };
            let candidate = grid_line.saturating_sub(stable_line);
            match inferred_offset {
                Some(current) if current != candidate => return false,
                Some(_) => {}
                None => inferred_offset = Some(candidate),
            }
        }

        if let Some(offset) = inferred_offset {
            self.grid_line_offset = offset;
        } else if !self.rows.is_empty()
            && viewport.display_offset == self.last_display_offset
            && viewport.sequence != self.last_sequence
        {
            return false;
        }

        let selected_lines = self.selected_line_bounds();
        for (grid_line, row) in viewport_rows {
            let stable_line = grid_line.saturating_sub(self.grid_line_offset);
            if let Some(previous) = self.rows.get(&stable_line)
                && previous.line_id != row.line_id
                && selected_lines
                    .is_some_and(|(start, end)| stable_line >= start && stable_line <= end)
            {
                return false;
            }
            if let Some(previous) = self.rows.insert(stable_line, row.clone()) {
                self.lines_by_id.remove(&previous.line_id);
            }
            self.lines_by_id.insert(row.line_id, stable_line);
        }

        self.last_display_offset = viewport.display_offset;
        self.last_sequence = viewport.sequence;
        true
    }

    pub(crate) fn update(
        &mut self,
        viewport: &SemanticViewport,
        point: AlacPoint,
        side: Side,
    ) -> bool {
        if !self.observe(viewport) {
            return false;
        }
        let Some(point) = self.current_to_stable(point) else {
            return false;
        };
        if !self.rows.contains_key(&point.line.0) {
            return false;
        }
        self.end = SelectionAnchor { point, side };
        true
    }

    pub(crate) fn render_range(&self, semantic_escape_chars: &str) -> Option<SelectionRange> {
        let mut range = self.stable_range(semantic_escape_chars)?;
        range.start.line.0 = range.start.line.0.checked_add(self.grid_line_offset)?;
        range.end.line.0 = range.end.line.0.checked_add(self.grid_line_offset)?;
        Some(range)
    }

    pub(crate) fn text(&self, semantic_escape_chars: &str) -> Option<String> {
        let range = self.stable_range(semantic_escape_chars)?;
        let mut text = String::new();
        for line in range.start.line.0..=range.end.line.0 {
            let row = self.rows.get(&line)?;
            let start = if line == range.start.line.0 {
                range.start.column.0
            } else {
                0
            };
            let end = if line == range.end.line.0 {
                range.end.column.0
            } else {
                self.context.columns.saturating_sub(1)
            };
            text.push_str(&row.text_for_columns(start, end));
            if end + 1 == self.context.columns && !row.wrapped {
                text.push('\n');
            }
        }

        if text.ends_with('\n') {
            text.pop();
        }
        if self.ty == SelectionType::Lines {
            text.push('\n');
        }
        Some(text)
    }

    fn current_to_stable(&self, mut point: AlacPoint) -> Option<AlacPoint> {
        point.line.0 = point.line.0.checked_sub(self.grid_line_offset)?;
        Some(point)
    }

    fn selected_line_bounds(&self) -> Option<(i32, i32)> {
        Some((
            self.start.point.line.0.min(self.end.point.line.0),
            self.start.point.line.0.max(self.end.point.line.0),
        ))
    }

    fn stable_range(&self, semantic_escape_chars: &str) -> Option<SelectionRange> {
        if self.context.columns == 0 {
            return None;
        }

        let (mut start, mut end) = (self.start, self.end);
        if start.point > end.point {
            std::mem::swap(&mut start, &mut end);
        }
        start.point.column.0 = start.point.column.0.min(self.context.columns - 1);
        end.point.column.0 = end.point.column.0.min(self.context.columns - 1);

        match self.ty {
            SelectionType::Simple => self.simple_range(start, end),
            SelectionType::Semantic => {
                if start.point == end.point
                    && let Some(matching) = self.matching_bracket(start.point)
                {
                    return Some(SelectionRange::new(
                        start.point.min(matching),
                        start.point.max(matching),
                        false,
                    ));
                }
                Some(SelectionRange::new(
                    self.semantic_search_left(start.point, semantic_escape_chars),
                    self.semantic_search_right(end.point, semantic_escape_chars),
                    false,
                ))
            }
            SelectionType::Lines => Some(SelectionRange::new(
                self.line_search_left(start.point),
                self.line_search_right(end.point),
                false,
            )),
            SelectionType::Block => None,
        }
    }

    fn simple_range(
        &self,
        mut start: SelectionAnchor,
        mut end: SelectionAnchor,
    ) -> Option<SelectionRange> {
        if start == end
            || (start.side == Side::Right
                && end.side == Side::Left
                && start.point.line == end.point.line
                && start.point.column + 1 == end.point.column)
        {
            return None;
        }

        if end.side == Side::Left && start.point != end.point {
            if end.point.column.0 == 0 {
                end.point.column = Column(self.context.columns - 1);
                end.point.line -= 1;
            } else {
                end.point.column -= 1;
            }
        }
        if start.side == Side::Right && start.point != end.point {
            start.point.column += 1;
            if start.point.column.0 == self.context.columns {
                start.point.column = Column(0);
                start.point.line += 1;
            }
        }
        (start.point <= end.point).then(|| SelectionRange::new(start.point, end.point, false))
    }

    fn semantic_search_left(&self, point: AlacPoint, escape_chars: &str) -> AlacPoint {
        let mut point = self.normalize_point(point);
        if self
            .character(point)
            .is_some_and(|character| escape_chars.contains(character))
        {
            return point;
        }
        while let Some(previous) = self.previous_wrapped_point(point) {
            if self
                .character(previous)
                .is_some_and(|character| escape_chars.contains(character))
            {
                break;
            }
            point = previous;
        }
        point
    }

    fn semantic_search_right(&self, point: AlacPoint, escape_chars: &str) -> AlacPoint {
        let mut point = self.normalize_point(point);
        if self
            .character(point)
            .is_some_and(|character| escape_chars.contains(character))
        {
            return point;
        }
        while let Some(next) = self.next_wrapped_point(point) {
            if self
                .character(next)
                .is_some_and(|character| escape_chars.contains(character))
            {
                break;
            }
            point = next;
        }
        let row = &self.rows[&point.line.0];
        point.column = Column(
            point
                .column
                .0
                .saturating_add(row.cell_width(point.column.0))
                .saturating_sub(1)
                .min(self.context.columns - 1),
        );
        point
    }

    fn line_search_left(&self, mut point: AlacPoint) -> AlacPoint {
        while self
            .rows
            .get(&point.line.0.saturating_sub(1))
            .is_some_and(|row| row.wrapped)
        {
            point.line -= 1;
        }
        point.column = Column(0);
        point
    }

    fn line_search_right(&self, mut point: AlacPoint) -> AlacPoint {
        while self.rows.get(&point.line.0).is_some_and(|row| row.wrapped)
            && self.rows.contains_key(&point.line.0.saturating_add(1))
        {
            point.line += 1;
        }
        point.column = Column(self.context.columns - 1);
        point
    }

    fn normalize_point(&self, mut point: AlacPoint) -> AlacPoint {
        if let Some(row) = self.rows.get(&point.line.0) {
            point.column = Column(row.leading_column(point.column.0));
        }
        point
    }

    fn character(&self, point: AlacPoint) -> Option<char> {
        self.rows.get(&point.line.0)?.character(point.column.0)
    }

    fn previous_wrapped_point(&self, point: AlacPoint) -> Option<AlacPoint> {
        let point = self.normalize_point(point);
        let row = self.rows.get(&point.line.0)?;
        if point.column.0 > 0 {
            let mut column = point.column.0 - 1;
            while column > 0 && matches!(row.cells[column], CachedCell::Spacer) {
                column -= 1;
            }
            return Some(AlacPoint::new(point.line, Column(column)));
        }

        let previous_line = point.line.0.checked_sub(1)?;
        let previous = self.rows.get(&previous_line)?;
        if !previous.wrapped {
            return None;
        }
        Some(AlacPoint::new(
            Line(previous_line),
            Column(previous.leading_column(self.context.columns - 1)),
        ))
    }

    fn next_wrapped_point(&self, point: AlacPoint) -> Option<AlacPoint> {
        let point = self.normalize_point(point);
        let row = self.rows.get(&point.line.0)?;
        let next_column = point
            .column
            .0
            .saturating_add(row.cell_width(point.column.0));
        if next_column < self.context.columns {
            return Some(AlacPoint::new(point.line, Column(next_column)));
        }
        if !row.wrapped {
            return None;
        }
        let next_line = point.line.0.checked_add(1)?;
        self.rows
            .contains_key(&next_line)
            .then(|| AlacPoint::new(Line(next_line), Column(0)))
    }

    fn previous_grid_point(&self, point: AlacPoint) -> Option<AlacPoint> {
        let point = self.normalize_point(point);
        let row = self.rows.get(&point.line.0)?;
        if point.column.0 > 0 {
            let mut column = point.column.0 - 1;
            while column > 0 && matches!(row.cells[column], CachedCell::Spacer) {
                column -= 1;
            }
            return Some(AlacPoint::new(point.line, Column(column)));
        }
        let previous_line = point.line.0.checked_sub(1)?;
        let previous = self.rows.get(&previous_line)?;
        Some(AlacPoint::new(
            Line(previous_line),
            Column(previous.leading_column(self.context.columns - 1)),
        ))
    }

    fn next_grid_point(&self, point: AlacPoint) -> Option<AlacPoint> {
        let point = self.normalize_point(point);
        let row = self.rows.get(&point.line.0)?;
        let next_column = point
            .column
            .0
            .saturating_add(row.cell_width(point.column.0));
        if next_column < self.context.columns {
            return Some(AlacPoint::new(point.line, Column(next_column)));
        }
        let next_line = point.line.0.checked_add(1)?;
        self.rows
            .contains_key(&next_line)
            .then(|| AlacPoint::new(Line(next_line), Column(0)))
    }

    fn matching_bracket(&self, point: AlacPoint) -> Option<AlacPoint> {
        let point = self.normalize_point(point);
        let character = self.character(point)?;
        let (forward, matching) = BRACKET_PAIRS.iter().find_map(|(open, close)| {
            if character == *open {
                Some((true, *close))
            } else if character == *close {
                Some((false, *open))
            } else {
                None
            }
        })?;

        let mut cursor = point;
        let mut nested = 0usize;
        loop {
            cursor = if forward {
                self.next_grid_point(cursor)?
            } else {
                self.previous_grid_point(cursor)?
            };
            let candidate = self.character(cursor)?;
            if candidate == matching {
                if nested == 0 {
                    return Some(cursor);
                }
                nested -= 1;
            } else if candidate == character {
                nested += 1;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::SemanticSelection;
    use alacritty_terminal::{
        index::{Column, Line, Point, Side},
        selection::SelectionType,
        vte::ansi::NamedColor,
    };
    use yttt_core::model::ids::TerminalSessionId;
    use yttt_protocol::terminal::{
        CursorShape, SemanticColor, SemanticCursor, SemanticRow, SemanticSpan, SemanticStyle,
        SemanticViewport, TerminalGeometry, TerminalModes, TerminalPalette, TerminalProcessState,
    };
    use yttt_terminal_core::semantic::STYLE_WRAPLINE;

    fn row(line_id: u64, viewport_row: u16, text: &str, wrapped: bool) -> SemanticRow {
        SemanticRow {
            line_id,
            viewport_row,
            spans: vec![SemanticSpan {
                start_column: 0,
                text: text.to_string(),
                width: unicode_width::UnicodeWidthStr::width(text) as u16,
                style: SemanticStyle {
                    foreground: SemanticColor::Named(NamedColor::Foreground as u16),
                    background: SemanticColor::Named(NamedColor::Background as u16),
                    flags: if wrapped { STYLE_WRAPLINE } else { 0 },
                    underline_color: SemanticColor::Named(NamedColor::Foreground as u16),
                },
                hyperlink: None,
            }],
        }
    }

    fn viewport(sequence: u64, display_offset: u64, rows: Vec<SemanticRow>) -> SemanticViewport {
        SemanticViewport {
            session_id: TerminalSessionId::new("semantic-selection"),
            session_epoch: 1,
            sequence,
            geometry: TerminalGeometry {
                cols: 8,
                rows: 2,
                cell_width: 0,
                cell_height: 0,
            },
            geometry_epoch: 1,
            scrollback_epoch: 1,
            history_size: 4,
            display_offset,
            rows,
            cursor: SemanticCursor {
                row: 0,
                column: 0,
                shape: CursorShape::Block,
                visible: false,
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

    #[test]
    fn simple_selection_extracts_wide_semantic_text() {
        let viewport = viewport(1, 0, vec![row(1, 0, "A界B", false), row(2, 1, "", false)]);
        let mut selection = SemanticSelection::new(
            &viewport,
            Point::new(Line(0), Column(1)),
            Side::Left,
            SelectionType::Simple,
        )
        .unwrap();

        assert!(selection.update(&viewport, Point::new(Line(0), Column(3)), Side::Right,));
        assert_eq!(selection.text(" "), Some("界B".to_string()));
        assert_eq!(
            selection.render_range(" ").unwrap(),
            alacritty_terminal::selection::SelectionRange::new(
                Point::new(Line(0), Column(1)),
                Point::new(Line(0), Column(3)),
                false,
            )
        );
    }

    #[test]
    fn selection_caches_scrolled_rows_and_preserves_wrapped_lines() {
        let bottom = viewport(
            1,
            0,
            vec![row(3, 0, "third", false), row(4, 1, "fourth", false)],
        );
        let mut selection = SemanticSelection::new(
            &bottom,
            Point::new(Line(1), Column(5)),
            Side::Right,
            SelectionType::Simple,
        )
        .unwrap();
        let history = viewport(
            2,
            2,
            vec![row(1, 0, "first---", true), row(2, 1, "second", false)],
        );

        assert!(selection.update(&history, Point::new(Line(-2), Column(0)), Side::Left,));
        assert_eq!(
            selection.text(" "),
            Some("first---second\nthird\nfourth".to_string())
        );
    }

    #[test]
    fn semantic_selection_expands_words_and_matching_brackets() {
        let viewport = viewport(
            1,
            0,
            vec![row(1, 0, "one two", false), row(2, 1, "(x[y])", false)],
        );
        let word = SemanticSelection::new(
            &viewport,
            Point::new(Line(0), Column(5)),
            Side::Left,
            SelectionType::Semantic,
        )
        .unwrap();
        let bracket = SemanticSelection::new(
            &viewport,
            Point::new(Line(1), Column(0)),
            Side::Left,
            SelectionType::Semantic,
        )
        .unwrap();

        assert_eq!(word.text(" "), Some("two".to_string()));
        assert_eq!(bracket.text(" "), Some("(x[y])".to_string()));
    }
}
