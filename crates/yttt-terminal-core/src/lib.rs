#![forbid(unsafe_code)]
mod graphics;
pub mod semantic;

use std::{
    collections::BTreeMap,
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

use yttt_protocol::terminal::TerminalImage;

const DEFAULT_CELL_WIDTH: u16 = 8;
const DEFAULT_CELL_HEIGHT: u16 = 16;

fn nonzero_cell_size(cell_size: u16, default: u16) -> u16 {
    if cell_size == 0 { default } else { cell_size }
}

struct TermDimensions {
    columns: usize,
    screen_lines: usize,
    cell_width: u16,
    cell_height: u16,
}

impl TermDimensions {
    fn new(columns: usize, screen_lines: usize) -> Self {
        Self::with_cell_size(
            columns,
            screen_lines,
            DEFAULT_CELL_WIDTH,
            DEFAULT_CELL_HEIGHT,
        )
    }

    fn with_cell_size(
        columns: usize,
        screen_lines: usize,
        cell_width: u16,
        cell_height: u16,
    ) -> Self {
        Self {
            columns,
            screen_lines,
            cell_width,
            cell_height,
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

    fn cell_width(&self) -> f32 {
        f32::from(self.cell_width)
    }

    fn cell_height(&self) -> f32 {
        f32::from(self.cell_height)
    }
}

pub struct TerminalState<L: EventListener> {
    term: Arc<FairMutex<Term<L>>>,
    parser: Processor<StdSyncHandler>,
    images: graphics::SharedImages,
    cols: usize,
    rows: usize,
    cell_width: u16,
    cell_height: u16,
}

impl<L: EventListener> Clone for TerminalState<L> {
    fn clone(&self) -> Self {
        Self {
            term: Arc::clone(&self.term),
            parser: Processor::new(),
            images: Arc::clone(&self.images),
            cols: self.cols,
            rows: self.rows,
            cell_width: self.cell_width,
            cell_height: self.cell_height,
        }
    }
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
            images: Arc::new(parking_lot::Mutex::new(graphics::ImageStore::default())),
            cols,
            rows,
            cell_width: DEFAULT_CELL_WIDTH,
            cell_height: DEFAULT_CELL_HEIGHT,
        }
    }

    pub fn process_bytes(&mut self, bytes: &[u8]) {
        let mut term = self.term.lock();
        self.parser.advance(&mut *term, bytes);
        let mut images = self.images.lock();
        graphics::drain(&mut term, &mut images);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.resize_with_cell_size(cols, rows, self.cell_width, self.cell_height);
    }

    pub fn resize_with_cell_size(
        &mut self,
        cols: usize,
        rows: usize,
        cell_width: u16,
        cell_height: u16,
    ) {
        self.cols = cols;
        self.rows = rows;
        self.cell_width = nonzero_cell_size(cell_width, DEFAULT_CELL_WIDTH);
        self.cell_height = nonzero_cell_size(cell_height, DEFAULT_CELL_HEIGHT);
        let dimensions =
            TermDimensions::with_cell_size(cols, rows, self.cell_width, self.cell_height);
        self.with_term_mut(|term| term.resize(dimensions));
    }

    pub fn parser(&self) -> TerminalParser<L> {
        TerminalParser {
            term: Arc::clone(&self.term),
            images: Arc::clone(&self.images),
            processor: Processor::new(),
        }
    }

    pub fn set_kitty_file_transfers(&self, allowed: bool) {
        self.with_term_mut(|term| term.set_kitty_file_transfers(allowed));
    }

    pub fn graphics_deadline(&self) -> Option<Instant> {
        self.with_term(Term::kitty_next_deadline)
    }

    pub fn advance_graphics(&self, now: Instant) -> bool {
        let mut term = self.term.lock();
        let changed = term.kitty_tick(now);
        graphics::drain(&mut term, &mut self.images.lock());
        changed
    }

    pub fn freeze_graphics(&self) {
        self.with_term_mut(Term::freeze_graphics);
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

    pub fn with_render_state<R>(
        &self,
        f: impl FnOnce(&mut Term<L>, &BTreeMap<u64, Arc<TerminalImage>>) -> R,
    ) -> R {
        let mut term = self.term.lock();
        term.kitty_tick(Instant::now());
        let mut images = self.images.lock();
        graphics::drain(&mut term, &mut images);
        f(&mut term, images.images())
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
    images: graphics::SharedImages,
    processor: Processor<StdSyncHandler>,
}

impl<L: EventListener> TerminalParser<L> {
    pub fn sync_deadline(&self) -> Option<Instant> {
        self.processor.sync_timeout().sync_timeout()
    }

    pub fn graphics_deadline(&self) -> Option<Instant> {
        self.term.lock().kitty_next_deadline()
    }

    pub fn advance_graphics(&self, now: Instant) -> bool {
        let mut term = self.term.lock();
        let changed = term.kitty_tick(now);
        graphics::drain(&mut term, &mut self.images.lock());
        changed
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
        let mut images = self.images.lock();
        graphics::drain(&mut term, &mut images);
        ParserBatchStats {
            lock_wait,
            advance: advance_started.elapsed(),
            unsynchronized_output: self.processor.sync_bytes_count() < bytes.len(),
        }
    }

    pub fn stop_sync(&mut self) {
        let mut term = self.term.lock();
        self.processor.stop_sync(&mut *term);
        let mut images = self.images.lock();
        graphics::drain(&mut term, &mut images);
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
        let mut parser = terminal.parser();
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

    #[test]
    fn parser_retains_sixel_asset_for_render_state() {
        use alacritty_terminal::index::{Column, Line};

        let mut terminal = TerminalState::new(8, 2, EventCollector::default());
        terminal.process_bytes(b"\x1bPq#1;2;100;0;0~\x1b\\");

        terminal.with_render_state(|term, images| {
            let graphic = &term.grid()[Line(0)][Column(0)].graphics().unwrap()[0];
            let image = images.get(&graphic.graphic_id().get()).unwrap();
            assert_eq!((image.width, image.height), (1, 6));
            assert_eq!(image.rgba.len(), 4 * 6);
        });
    }

    #[test]
    fn kitty_query_replies_before_device_attributes_without_storing_pixels() {
        let events = EventCollector::default();
        let observed = events.clone();
        let mut terminal = TerminalState::new(8, 2, events);
        terminal.process_bytes(b"\x1b_Ga=q,i=7,f=24,s=1,v=1;AQID\x1b\\\x1b[c");
        let events = observed.0.lock();
        let replies: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                Event::PtyWrite(reply) => Some(reply.as_str()),
                _ => None,
            })
            .collect();
        assert_eq!(replies.len(), 2);
        assert_eq!(replies[0], "\x1b_Gi=7;OK\x1b\\");
        assert!(replies[1].starts_with("\x1b[?") && replies[1].ends_with('c'));
        terminal.with_render_state(|term, images| {
            assert!(images.is_empty());
            assert!(term.kitty_scene(0).placements.is_empty());
        });
    }

    #[test]
    fn local_copy_omits_placeholder_metadata_but_preserves_real_combining_text() {
        use alacritty_terminal::index::{Column, Line};

        let mut terminal = TerminalState::new(8, 2, EventCollector::default());
        terminal.process_bytes("a\u{10eeee}\u{305}\u{305}e\u{301}".as_bytes());
        terminal.set_simple_selection(
            Point::new(Line(0), Column(0)),
            Point::new(Line(0), Column(2)),
        );
        assert_eq!(
            terminal.selection_to_string().as_deref(),
            Some("a e\u{301}")
        );
    }

    #[test]
    fn frozen_final_graphics_do_not_change_on_later_checkpoint_capture() {
        let mut terminal = TerminalState::new(8, 4, EventCollector::default());
        terminal.process_bytes(
            concat!(
                "\x1b_Ga=T,i=8,f=32,s=1,v=1,C=1,q=2;/wAA/w==\x1b\\",
                "\x1b_Ga=f,i=8,f=32,s=1,v=1,z=40,q=2;AP8A/w==\x1b\\",
                "\x1b_Ga=a,i=8,r=1,z=40,s=3,v=1,q=2\x1b\\",
            )
            .as_bytes(),
        );
        assert!(terminal.graphics_deadline().is_some());
        terminal.freeze_graphics();
        let final_image =
            terminal.with_render_state(|term, _| term.kitty_scene(0).placements[0].image_id);
        assert!(!terminal.advance_graphics(Instant::now() + Duration::from_secs(10)));
        assert!(terminal.graphics_deadline().is_none());
        terminal.with_render_state(|term, _| {
            assert_eq!(term.kitty_scene(0).placements[0].image_id, final_image);
        });
    }

    #[test]
    fn alternate_screen_resize_preserves_primary_scrollback_graphics() {
        let mut terminal = TerminalState::new_with_scrollback(8, 4, 16, EventCollector::default());
        terminal
            .process_bytes(b"\x1b_Ga=T,i=9,f=32,s=1,v=1,c=2,r=1,C=1,q=2;/wAA/w==\x1b\\\n\n\n\n\n");
        terminal.process_bytes(b"\x1b[?1049h");
        terminal.resize_with_cell_size(8, 3, 12, 24);
        terminal.with_render_state(|term, _| assert!(term.kitty_scene(0).placements.is_empty()));
        terminal.process_bytes(b"\x1b[?1049l");
        terminal.scroll_display(Scroll::Top);
        terminal.with_render_state(|term, images| {
            let scene = term.kitty_scene(term.grid().display_offset());
            assert_eq!(scene.placements.len(), 1);
            let placement = &scene.placements[0];
            assert_eq!((placement.width, placement.height), (24, 24));
            assert_eq!(placement.y, 0);
            assert_eq!(
                images[&placement.image_id].rgba.as_slice(),
                &[255, 0, 0, 255]
            );
        });
    }

    #[test]
    fn partial_margin_scroll_keeps_moving_the_surviving_image_fragment() {
        let mut terminal = TerminalState::new(8, 5, EventCollector::default());
        terminal.process_bytes(
            concat!(
                "\x1b[2;1H\x1b_Ga=T,i=9,f=32,s=1,v=1,c=2,r=2,C=1,q=2;/wAA/w==\x1b\\",
                "\x1b[2;4r\x1b[S",
            )
            .as_bytes(),
        );
        terminal.with_render_state(|term, _| {
            let scene = term.kitty_scene(0);
            assert_eq!(scene.placements.len(), 1);
            let placement = &scene.placements[0];
            assert_eq!(
                (placement.y, placement.clip_y, placement.clip_height),
                (0, 16, 16)
            );
        });
        terminal.process_bytes(b"\x1b[S");
        terminal.with_render_state(|term, _| assert!(term.kitty_scene(0).placements.is_empty()));
    }

    #[test]
    fn top_page_margin_clips_graphics_instead_of_preserving_them_in_history() {
        let mut terminal = TerminalState::new(8, 5, EventCollector::default());
        terminal.process_bytes(
            concat!(
                "\x1b_Ga=T,i=9,f=32,s=1,v=1,c=2,r=1,C=1,q=2;/wAA/w==\x1b\\",
                "\x1b[1;3r\x1b[S",
            )
            .as_bytes(),
        );
        terminal.scroll_display(Scroll::Top);
        terminal.with_render_state(|term, _| {
            assert!(
                term.kitty_scene(term.grid().display_offset())
                    .placements
                    .is_empty()
            );
        });
    }

    #[test]
    fn enormous_placement_is_clipped_and_can_still_be_cleared() {
        let mut terminal = TerminalState::new(8, 4, EventCollector::default());
        terminal.process_bytes(
            b"\x1b_Ga=T,i=9,f=32,s=1,v=1,c=4294967295,r=4294967295,Y=15,C=1,q=2;/wAA/w==\x1b\\",
        );
        terminal.with_render_state(|term, _| {
            let scene = term.kitty_scene(0);
            assert_eq!(scene.placements.len(), 1);
            assert_eq!(
                (
                    scene.placements[0].clip_width,
                    scene.placements[0].clip_height
                ),
                (64, 49)
            );
        });
        terminal.process_bytes(b"\x1b[2J");
        terminal.with_render_state(|term, _| assert!(term.kitty_scene(0).placements.is_empty()));
    }

    #[test]
    fn quiet_graphics_never_leak_success_or_error_responses_into_pty_input() {
        let events = EventCollector::default();
        let observed = events.clone();
        let mut terminal = TerminalState::new(8, 4, events);
        terminal.process_bytes(
            concat!(
                "\x1b_Ga=T,i=18,f=32,s=1,v=1,C=1,q=2;AQIDBA==\x1b\\",
                "\x1b_Ga=p,i=999,q=2\x1b\\",
                "\x1b_Ga=p,i=18,C=1,q=1\x1b\\",
            )
            .as_bytes(),
        );
        assert!(
            !observed
                .0
                .lock()
                .iter()
                .any(|event| matches!(event, Event::PtyWrite(_)))
        );
        terminal.with_render_state(|term, images| {
            let scene = term.kitty_scene(0);
            assert_eq!(scene.placements.len(), 2);
            assert_eq!(
                images[&scene.placements[0].image_id].rgba.as_slice(),
                &[1, 2, 3, 4]
            );
        });
        terminal.process_bytes(b"\x1b_Ga=p,i=999,q=1\x1b\\");
        assert!(observed.0.lock().iter().any(|event| matches!(
            event, Event::PtyWrite(reply) if reply.starts_with("\x1b_Gi=999;ENOENT")
        )));
    }
}
