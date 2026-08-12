#![forbid(unsafe_code)]
pub mod semantic;

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alacritty_terminal::{
    event::EventListener,
    grid::{Dimensions, Scroll},
    index::{Point, Side},
    selection::{Selection, SelectionType},
    sync::FairMutex,
    term::{Config, Term, TermMode, color::Colors},
    vte::ansi::{Processor, StdSyncHandler},
};

struct TermDimensions {
    columns: usize,
    screen_lines: usize,
}

impl TermDimensions {
    fn new(columns: usize, screen_lines: usize) -> Self {
        Self {
            columns,
            screen_lines,
        }
    }
}

impl Dimensions for TermDimensions {
    fn total_lines(&self) -> usize {
        self.screen_lines
    }

    fn screen_lines(&self) -> usize {
        self.screen_lines
    }

    fn columns(&self) -> usize {
        self.columns
    }

    fn last_column(&self) -> alacritty_terminal::index::Column {
        alacritty_terminal::index::Column(self.columns.saturating_sub(1))
    }
}

pub struct TerminalState<L: EventListener> {
    term: Arc<FairMutex<Term<L>>>,
    parser: Processor<StdSyncHandler>,
    cols: usize,
    rows: usize,
}

impl<L: EventListener> TerminalState<L> {
    pub fn new(cols: usize, rows: usize, event_listener: L) -> Self {
        Self::new_with_options(cols, rows, Config::default(), event_listener)
    }

    pub fn new_with_scrollback(
        cols: usize,
        rows: usize,
        scrollback: usize,
        event_listener: L,
    ) -> Self {
        let config = Config {
            scrolling_history: scrollback,
            ..Config::default()
        };
        Self::new_with_options(cols, rows, config, event_listener)
    }

    pub fn new_with_options(cols: usize, rows: usize, config: Config, event_listener: L) -> Self {
        let dimensions = TermDimensions::new(cols, rows);
        let term = Term::new(config, &dimensions, event_listener);
        Self {
            term: Arc::new(FairMutex::new(term)),
            parser: Processor::new(),
            cols,
            rows,
        }
    }

    pub fn process_bytes(&mut self, bytes: &[u8]) {
        let mut term = self.term.lock();
        self.parser.advance(&mut *term, bytes);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;
        self.with_term_mut(|term| term.resize(TermDimensions::new(cols, rows)));
    }

    pub fn mode(&self) -> TermMode {
        self.with_term(|term| *term.mode())
    }

    pub fn dynamic_colors(&self) -> Colors {
        self.with_term(|term| *term.colors())
    }

    pub fn set_options(&self, options: Config) {
        self.with_term_mut(|term| term.set_options(options));
    }

    pub fn scroll_display(&self, scroll: Scroll) {
        self.with_term_mut(|term| term.scroll_display(scroll));
    }

    pub fn display_offset(&self) -> usize {
        self.with_term(|term| term.grid().display_offset())
    }

    pub fn scrollbar_metrics(&self) -> Option<TerminalScrollbarMetrics> {
        self.with_term(|term| {
            let grid = term.grid();
            let history_size = grid.total_lines().saturating_sub(grid.screen_lines());
            TerminalScrollbarMetrics::from_rows(
                history_size,
                grid.screen_lines(),
                grid.display_offset(),
            )
        })
    }

    pub fn begin_selection(&self, point: Point, side: Side, selection_type: SelectionType) {
        self.with_term_mut(|term| {
            term.selection = Some(Selection::new(selection_type, point, side));
        });
    }

    pub fn update_selection(&self, point: Point, side: Side) {
        self.with_term_mut(|term| {
            if let Some(selection) = term.selection.as_mut() {
                selection.update(point, side);
            }
        });
    }

    pub fn set_simple_selection(&self, start: Point, end: Point) {
        self.begin_selection(start, Side::Left, SelectionType::Simple);
        self.update_selection(end, Side::Right);
    }

    pub fn clear_selection(&self) {
        self.with_term_mut(|term| term.selection = None);
    }

    pub fn selection_to_string(&self) -> Option<String> {
        self.with_term(|term| term.selection_to_string())
    }

    pub fn with_term<F, R>(&self, function: F) -> R
    where
        F: FnOnce(&Term<L>) -> R,
    {
        let term = self.term.lock();
        function(&term)
    }

    pub fn with_term_mut<F, R>(&self, function: F) -> R
    where
        F: FnOnce(&mut Term<L>) -> R,
    {
        let mut term = self.term.lock();
        function(&mut term)
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows
    }

    pub fn term_arc(&self) -> Arc<FairMutex<Term<L>>> {
        Arc::clone(&self.term)
    }
}

pub struct TerminalParser<L: EventListener> {
    term: Arc<FairMutex<Term<L>>>,
    processor: Processor<StdSyncHandler>,
}

impl<L: EventListener> TerminalParser<L> {
    pub fn new(term: Arc<FairMutex<Term<L>>>) -> Self {
        Self {
            term,
            processor: Processor::new(),
        }
    }

    pub fn sync_deadline(&self) -> Option<Instant> {
        self.processor.sync_timeout().sync_timeout()
    }

    pub fn sync_bytes_count(&self) -> usize {
        self.processor.sync_bytes_count()
    }

    pub fn advance(&mut self, bytes: &[u8]) -> ParserBatchStats {
        let lock_started = Instant::now();
        let mut term = self.term.lock();
        let lock_wait = lock_started.elapsed();
        let advance_started = Instant::now();
        self.processor.advance(&mut *term, bytes);
        ParserBatchStats {
            lock_wait,
            advance: advance_started.elapsed(),
            unsynchronized_output: self.processor.sync_bytes_count() < bytes.len(),
        }
    }

    pub fn stop_sync(&mut self) {
        let mut term = self.term.lock();
        self.processor.stop_sync(&mut *term);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ParserBatchStats {
    pub lock_wait: Duration,
    pub advance: Duration,
    pub unsynchronized_output: bool,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalScrollbarMetrics {
    pub thumb_top_fraction: f32,
    pub thumb_height_fraction: f32,
    pub scroll_progress_from_top: f32,
}

impl TerminalScrollbarMetrics {
    const MIN_THUMB_FRACTION: f32 = 0.08;

    pub fn from_rows(
        history_size: usize,
        visible_rows: usize,
        display_offset: usize,
    ) -> Option<Self> {
        if history_size == 0 || visible_rows == 0 {
            return None;
        }
        let total_rows = history_size + visible_rows;
        let thumb_height_fraction =
            (visible_rows as f32 / total_rows as f32).clamp(Self::MIN_THUMB_FRACTION, 1.0);
        let display_offset = display_offset.min(history_size);
        let scroll_progress_from_top = (history_size - display_offset) as f32 / history_size as f32;
        let thumb_top_fraction = scroll_progress_from_top * (1.0 - thumb_height_fraction);
        Some(Self {
            thumb_top_fraction,
            thumb_height_fraction,
            scroll_progress_from_top,
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use alacritty_terminal::event::{Event, EventListener};
    use parking_lot::Mutex;

    use super::*;

    #[derive(Clone, Default)]
    struct EventCollector(Arc<Mutex<Vec<Event>>>);

    impl EventListener for EventCollector {
        fn send_event(&self, event: Event) {
            self.0.lock().push(event);
        }
    }

    #[test]
    fn parses_vt_bytes_and_caps_scrollback_without_ui_dependencies() {
        let events = EventCollector::default();
        let mut terminal = TerminalState::new_with_scrollback(8, 2, 3, events);
        terminal.process_bytes(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        terminal.scroll_display(Scroll::Top);

        assert_eq!(terminal.display_offset(), 3);
        assert_eq!(terminal.cols(), 8);
        assert_eq!(terminal.rows(), 2);
    }

    #[test]
    fn parser_coordinator_preserves_split_escape_sequences() {
        let events = EventCollector::default();
        let observed = events.clone();
        let terminal = TerminalState::new(8, 2, events);
        let mut parser = TerminalParser::new(terminal.term_arc());
        parser.advance(b"\x1b]2;split");
        parser.advance(b" title\x07");

        assert!(
            observed
                .0
                .lock()
                .iter()
                .any(|event| matches!(event, Event::Title(title) if title == "split title"))
        );
    }
}
