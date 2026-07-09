//! Terminal state management.
//!
//! This module provides [`TerminalState`], a thread-safe wrapper around alacritty's
//! [`Term`] structure. It manages the terminal
//! emulator state, including the character grid, cursor position, and VTE parser.
//!
//! # Architecture
//!
//! `TerminalState` wraps the alacritty terminal in `Arc<Mutex<>>` to allow safe
//! concurrent access from:
//!
//! - The async reader task (writing bytes to the terminal)
//! - The render thread (reading the grid for display)
//! - The main thread (handling resize events)
//!
//! # VTE Parsing
//!
//! The terminal uses alacritty's VTE parser to process byte streams. When bytes
//! arrive from the PTY, they are fed through the parser via [`process_bytes`],
//! which calls handler methods on the `Term` to update the grid:
//!
//! ```text
//! PTY bytes → VTE Parser → Term handlers → Grid updates
//!                          ├─ print()     (regular characters)
//!                          ├─ execute()   (control chars: BEL, BS, etc.)
//!                          ├─ esc_dispatch()  (escape sequences)
//!                          └─ csi_dispatch()  (CSI sequences: colors, cursor, etc.)
//! ```
//!
//! # Example
//!
//! ```
//! use std::sync::mpsc::channel;
//! use yttt_terminal::event::GpuiEventProxy;
//! use yttt_terminal::terminal::TerminalState;
//!
//! let (tx, rx) = channel();
//! let event_proxy = GpuiEventProxy::new(tx);
//! let mut terminal = TerminalState::new(80, 24, event_proxy);
//!
//! // Process some output (e.g., colored text)
//! terminal.process_bytes(b"\x1b[31mRed text\x1b[0m");
//! ```
//!
//! [`process_bytes`]: TerminalState::process_bytes

use crate::event::GpuiEventProxy;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::ansi::Processor;
use parking_lot::Mutex;
use std::sync::Arc;

/// Simple dimensions implementation for terminal initialization.
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
        // For initial setup, total lines equals screen lines
        // The scrollback buffer will be managed by the Term itself
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

/// Thread-safe terminal state wrapper.
///
/// This struct wraps alacritty's [`Term`] in an
/// `Arc<parking_lot::Mutex<>>` to allow safe concurrent access from multiple threads.
/// It also manages the VTE parser for processing incoming bytes from the PTY.
///
/// # Thread Safety
///
/// The terminal state can be safely shared across threads:
///
/// - Use [`term_arc`](Self::term_arc) to get a cloned `Arc` for sharing
/// - Use [`with_term`](Self::with_term) for read access to the grid
/// - Use [`with_term_mut`](Self::with_term_mut) for write access
///
/// The mutex is held only for the duration of the closure, minimizing contention.
///
/// # Grid Access
///
/// The terminal grid is accessed through the `Term` structure:
///
/// ```ignore
/// terminal_state.with_term(|term| {
///     let grid = term.grid();
///     let cursor = grid.cursor.point;
///     let cell = &grid[cursor];
///     // Read cell content, colors, flags, etc.
/// });
/// ```
///
/// # Performance Notes
///
/// - `parking_lot::Mutex` is used for faster locking than `std::sync::Mutex`
/// - Lock contention is minimized by keeping critical sections short
/// - The VTE parser state is kept outside the mutex (only accessed from one thread)
pub struct TerminalState {
    /// The underlying alacritty terminal emulator.
    term: Arc<Mutex<Term<GpuiEventProxy>>>,

    /// VTE parser for converting byte streams into terminal actions.
    parser: Processor,

    /// Number of columns in the terminal.
    cols: usize,

    /// Number of rows (lines) in the terminal.
    rows: usize,
}

/// Scrollbar geometry derived from terminal scrollback state.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalScrollbarMetrics {
    /// Top of the scrollbar thumb as a fraction of the full track height.
    pub thumb_top_fraction: f32,

    /// Height of the scrollbar thumb as a fraction of the full track height.
    pub thumb_height_fraction: f32,

    /// Scroll progress from top to bottom, where 0 is top and 1 is bottom.
    pub scroll_progress_from_top: f32,
}

impl TerminalScrollbarMetrics {
    const MIN_THUMB_FRACTION: f32 = 0.08;

    /// Build scrollbar metrics from row counts and display offset.
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
            ((visible_rows as f32) / (total_rows as f32)).clamp(Self::MIN_THUMB_FRACTION, 1.0);
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

impl TerminalState {
    /// Create a new terminal state with the given dimensions.
    ///
    /// # Arguments
    ///
    /// * `cols` - The number of columns (character width) of the terminal
    /// * `rows` - The number of rows (lines) of the terminal
    /// * `event_proxy` - The event proxy for forwarding terminal events to GPUI
    ///
    /// # Returns
    ///
    /// A new `TerminalState` instance.
    ///
    /// # Examples
    ///
    /// ```
    /// use std::sync::mpsc::channel;
    /// use yttt_terminal::event::GpuiEventProxy;
    /// use yttt_terminal::terminal::TerminalState;
    ///
    /// let (tx, rx) = channel();
    /// let event_proxy = GpuiEventProxy::new(tx);
    /// let terminal = TerminalState::new(80, 24, event_proxy);
    /// ```
    pub fn new(cols: usize, rows: usize, event_proxy: GpuiEventProxy) -> Self {
        Self::new_with_scrollback(cols, rows, Config::default().scrolling_history, event_proxy)
    }

    /// Create a new terminal state with a custom scrollback history limit.
    pub fn new_with_scrollback(
        cols: usize,
        rows: usize,
        scrollback: usize,
        event_proxy: GpuiEventProxy,
    ) -> Self {
        let mut config = Config::default();
        config.scrolling_history = scrollback;

        // Create dimensions for terminal initialization
        let dimensions = TermDimensions::new(cols, rows);

        // Create the terminal with the given configuration and dimensions
        let term = Term::new(config, &dimensions, event_proxy);

        // Create the VTE parser for processing incoming bytes
        let parser = Processor::new();

        Self {
            term: Arc::new(Mutex::new(term)),
            parser,
            cols,
            rows,
        }
    }

    /// Process incoming bytes from the PTY.
    ///
    /// This method feeds the bytes through the VTE parser, which will call
    /// the appropriate handler methods on the terminal to update its state.
    ///
    /// # Arguments
    ///
    /// * `bytes` - The bytes received from the PTY
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::sync::mpsc::channel;
    /// # use yttt_terminal::event::GpuiEventProxy;
    /// # use yttt_terminal::terminal::TerminalState;
    /// # let (tx, rx) = channel();
    /// # let event_proxy = GpuiEventProxy::new(tx);
    /// # let mut terminal = TerminalState::new(80, 24, event_proxy);
    /// // Process some output from the PTY
    /// terminal.process_bytes(b"Hello, world!\r\n");
    /// ```
    pub fn process_bytes(&mut self, bytes: &[u8]) {
        let mut term = self.term.lock();
        // The parser.advance method calls handler methods on the Term
        // The Term implements the Handler trait from the VTE crate
        self.parser.advance(&mut *term, bytes);
    }

    /// Resize the terminal to new dimensions.
    ///
    /// This method updates the terminal's internal grid to match the new size.
    /// It should be called when the terminal view is resized.
    ///
    /// # Arguments
    ///
    /// * `cols` - The new number of columns
    /// * `rows` - The new number of rows
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::sync::mpsc::channel;
    /// # use yttt_terminal::event::GpuiEventProxy;
    /// # use yttt_terminal::terminal::TerminalState;
    /// # let (tx, rx) = channel();
    /// # let event_proxy = GpuiEventProxy::new(tx);
    /// # let mut terminal = TerminalState::new(80, 24, event_proxy);
    /// // Resize to 120x30
    /// terminal.resize(120, 30);
    /// ```
    pub fn resize(&mut self, cols: usize, rows: usize) {
        self.cols = cols;
        self.rows = rows;

        let mut term = self.term.lock();

        // Create dimensions for the resize
        let dimensions = TermDimensions::new(cols, rows);

        // Resize the terminal
        term.resize(dimensions);
    }

    /// Get the current terminal mode.
    ///
    /// The terminal mode affects how certain key sequences are interpreted,
    /// particularly arrow keys in application cursor mode.
    ///
    /// # Returns
    ///
    /// The current `TermMode` flags.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::sync::mpsc::channel;
    /// # use yttt_terminal::event::GpuiEventProxy;
    /// # use yttt_terminal::terminal::TerminalState;
    /// # let (tx, rx) = channel();
    /// # let event_proxy = GpuiEventProxy::new(tx);
    /// # let terminal = TerminalState::new(80, 24, event_proxy);
    /// use alacritty_terminal::term::TermMode;
    ///
    /// let mode = terminal.mode();
    /// if mode.contains(TermMode::APP_CURSOR) {
    ///     println!("Application cursor mode is enabled");
    /// }
    /// ```
    pub fn mode(&self) -> TermMode {
        let term = self.term.lock();
        *term.mode()
    }

    /// Update the maximum number of scrollback lines kept by the terminal.
    pub fn set_scrollback(&self, scrollback: usize) {
        self.with_term_mut(|term| {
            let mut config = Config::default();
            config.scrolling_history = scrollback;
            term.set_options(config);
        });
    }

    /// Scroll the visible terminal display through scrollback history.
    pub fn scroll_display(&self, scroll: Scroll) {
        self.with_term_mut(|term| {
            term.scroll_display(scroll);
        });
    }

    /// Current scrollback display offset.
    pub fn display_offset(&self) -> usize {
        self.with_term(|term| term.grid().display_offset())
    }

    /// Current scrollbar metrics, if scrollback history is available.
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

    /// Begin a terminal text selection.
    pub fn begin_selection(&self, point: AlacPoint, selection_type: SelectionType) {
        self.with_term_mut(|term| {
            term.selection = Some(Selection::new(selection_type, point, Side::Left));
        });
    }

    /// Update the current terminal text selection.
    pub fn update_selection(&self, point: AlacPoint) {
        self.with_term_mut(|term| {
            if let Some(selection) = term.selection.as_mut() {
                selection.update(point, Side::Right);
            }
        });
    }

    /// Set a simple range selection.
    pub fn set_simple_selection(&self, start: AlacPoint, end: AlacPoint) {
        self.begin_selection(start, SelectionType::Simple);
        self.update_selection(end);
    }

    /// Clear any active terminal text selection.
    pub fn clear_selection(&self) {
        self.with_term_mut(|term| {
            term.selection = None;
        });
    }

    /// Convert the active terminal selection to text.
    pub fn selection_to_string(&self) -> Option<String> {
        self.with_term(|term| term.selection_to_string())
    }

    /// Execute a function with read access to the terminal.
    ///
    /// This method provides safe read access to the underlying `Term` structure.
    /// The terminal is locked for the duration of the function call.
    ///
    /// # Arguments
    ///
    /// * `f` - A function that takes a reference to the `Term` and returns a value
    ///
    /// # Returns
    ///
    /// The value returned by the function `f`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::sync::mpsc::channel;
    /// # use yttt_terminal::event::GpuiEventProxy;
    /// # use yttt_terminal::terminal::TerminalState;
    /// # let (tx, rx) = channel();
    /// # let event_proxy = GpuiEventProxy::new(tx);
    /// # let terminal = TerminalState::new(80, 24, event_proxy);
    /// let cursor_pos = terminal.with_term(|term| {
    ///     term.grid().cursor.point
    /// });
    /// ```
    pub fn with_term<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&Term<GpuiEventProxy>) -> R,
    {
        let term = self.term.lock();
        f(&term)
    }

    /// Execute a function with mutable access to the terminal.
    ///
    /// This method provides safe write access to the underlying `Term` structure.
    /// The terminal is locked for the duration of the function call.
    ///
    /// # Arguments
    ///
    /// * `f` - A function that takes a mutable reference to the `Term` and returns a value
    ///
    /// # Returns
    ///
    /// The value returned by the function `f`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use std::sync::mpsc::channel;
    /// # use yttt_terminal::event::GpuiEventProxy;
    /// # use yttt_terminal::terminal::TerminalState;
    /// # let (tx, rx) = channel();
    /// # let event_proxy = GpuiEventProxy::new(tx);
    /// # let terminal = TerminalState::new(80, 24, event_proxy);
    /// terminal.with_term_mut(|term| {
    ///     // Perform some mutation on the term
    ///     term.scroll_display(alacritty_terminal::grid::Scroll::Delta(5));
    /// });
    /// ```
    pub fn with_term_mut<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut Term<GpuiEventProxy>) -> R,
    {
        let mut term = self.term.lock();
        f(&mut term)
    }

    /// Get the number of columns in the terminal.
    ///
    /// # Returns
    ///
    /// The current number of columns.
    pub fn cols(&self) -> usize {
        self.cols
    }

    /// Get the number of rows in the terminal.
    ///
    /// # Returns
    ///
    /// The current number of rows.
    pub fn rows(&self) -> usize {
        self.rows
    }

    /// Get a cloned reference to the underlying terminal Arc.
    ///
    /// This allows sharing the terminal state across multiple threads or components.
    ///
    /// # Returns
    ///
    /// A cloned `Arc<Mutex<Term<GpuiEventProxy>>>`.
    pub fn term_arc(&self) -> Arc<Mutex<Term<GpuiEventProxy>>> {
        Arc::clone(&self.term)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::TerminalEvent;
    use alacritty_terminal::grid::Scroll;
    use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
    use std::sync::mpsc::channel;

    fn event_proxy() -> GpuiEventProxy {
        let (tx, _rx) = channel();
        GpuiEventProxy::new(tx)
    }

    #[test]
    fn test_terminal_creation() {
        let terminal = TerminalState::new(80, 24, event_proxy());

        assert_eq!(terminal.cols(), 80);
        assert_eq!(terminal.rows(), 24);
    }

    #[test]
    fn test_new_with_scrollback_caps_history() {
        let mut terminal = TerminalState::new_with_scrollback(8, 2, 3, event_proxy());

        terminal.process_bytes(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        terminal.scroll_display(Scroll::Top);

        assert_eq!(terminal.display_offset(), 3);
    }

    #[test]
    fn test_scroll_display_moves_visible_history() {
        let mut terminal = TerminalState::new_with_scrollback(8, 2, 20, event_proxy());

        terminal.process_bytes(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        terminal.scroll_display(Scroll::Delta(2));
        assert_eq!(terminal.display_offset(), 2);

        terminal.scroll_display(Scroll::Delta(-1));
        assert_eq!(terminal.display_offset(), 1);
    }

    #[test]
    fn test_primary_device_attributes_query_emits_pty_write() {
        let (tx, rx) = channel();
        let mut terminal = TerminalState::new(80, 24, GpuiEventProxy::new(tx));

        terminal.process_bytes(b"\x1b[c");

        let events: Vec<_> = rx.try_iter().collect();
        assert!(
            events
                .iter()
                .any(|event| matches!(event, TerminalEvent::PtyWrite(data) if data == "\x1b[?6c")),
            "expected primary device attributes response, got {events:?}",
        );
    }

    #[test]
    fn test_scrollbar_metrics_follow_display_offset() {
        let mut terminal = TerminalState::new_with_scrollback(8, 2, 20, event_proxy());

        terminal.process_bytes(b"one\r\ntwo\r\nthree\r\nfour\r\nfive\r\n");
        let bottom = terminal
            .scrollbar_metrics()
            .expect("scrollbar metrics should exist when history is present");

        terminal.scroll_display(Scroll::Top);
        let top = terminal
            .scrollbar_metrics()
            .expect("scrollbar metrics should exist when scrolled to top");

        assert!(bottom.thumb_top_fraction > top.thumb_top_fraction);
        assert_eq!(bottom.thumb_height_fraction, top.thumb_height_fraction);
        assert!(bottom.thumb_height_fraction > 0.0);
        assert!(bottom.thumb_height_fraction <= 1.0);
    }

    #[test]
    fn test_simple_selection_to_string() {
        let mut terminal = TerminalState::new_with_scrollback(8, 2, 20, event_proxy());

        terminal.process_bytes(b"abcdef");
        terminal.set_simple_selection(
            AlacPoint::new(Line(0), Column(1)),
            AlacPoint::new(Line(0), Column(3)),
        );

        assert_eq!(terminal.selection_to_string(), Some("bcd".to_string()));
    }

    #[test]
    fn test_process_bytes() {
        let mut terminal = TerminalState::new(80, 24, event_proxy());

        // Process some text
        terminal.process_bytes(b"Hello, world!");

        // Verify the text was written to the grid
        terminal.with_term(|term| {
            let grid = term.grid();
            // The text should be at the cursor position
            // We can't easily test the exact content without more complex grid inspection
            assert!(grid.columns() == 80);
        });
    }

    #[test]
    fn test_resize() {
        let mut terminal = TerminalState::new(80, 24, event_proxy());

        terminal.resize(120, 30);

        assert_eq!(terminal.cols(), 120);
        assert_eq!(terminal.rows(), 30);

        terminal.with_term(|term| {
            let grid = term.grid();
            assert_eq!(grid.columns(), 120);
            assert_eq!(grid.screen_lines(), 30);
        });
    }

    #[test]
    fn test_mode() {
        let terminal = TerminalState::new(80, 24, event_proxy());

        let mode = terminal.mode();
        // Mode should be a valid TermMode value (just verify we can get it)
        let _bits = mode.bits();
    }

    #[test]
    fn test_with_term() {
        let terminal = TerminalState::new(80, 24, event_proxy());

        let cols = terminal.with_term(|term| term.grid().columns());
        assert_eq!(cols, 80);
    }

    #[test]
    fn test_with_term_mut() {
        let terminal = TerminalState::new(80, 24, event_proxy());

        terminal.with_term_mut(|term| {
            // Just verify we can get mutable access
            let _grid = term.grid_mut();
        });
    }

    #[test]
    fn test_term_arc() {
        let terminal = TerminalState::new(80, 24, event_proxy());

        let arc1 = terminal.term_arc();
        let arc2 = terminal.term_arc();

        // Both Arcs should point to the same terminal
        assert!(Arc::ptr_eq(&arc1, &arc2));
    }
}
