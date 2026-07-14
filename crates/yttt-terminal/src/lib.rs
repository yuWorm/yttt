//! # `yttt-terminal`
//!
//! An embeddable GPUI terminal adapter built on
//! [`alacritty_terminal`](https://docs.rs/alacritty_terminal).
//!
//! [`TerminalView`] owns GPUI focus, input, callbacks, interaction state, and painting. Alacritty
//! remains authoritative for VT parsing, grid state, terminal modes, selection, search, dynamic
//! colors, cursor style, and protocol negotiation.
//!
//! ## Execution model
//!
//! A bounded three-worker driver keeps blocking PTY operations away from GPUI:
//!
//! 1. A reader fills a preallocated pool of fixed-size buffers and submits at most eight batches.
//! 2. A parser coordinator owns the VTE processor, applies bytes under the `Term` lock, and
//!    returns buffers to the pool.
//! 3. An ordered writer processes input, query replies, resize callbacks, and shutdown.
//!
//! Alacritty events pass through a bounded, coalescing mailbox. GPUI receives one capacity-one
//! signal and drains titles, bells, clipboard requests, dynamic queries, cursor changes, I/O
//! failures, and lifecycle events in order. Keyboard and mouse handlers only enqueue protocol
//! bytes; they never block on the PTY writer.
//!
//! ## Rendering model
//!
//! The terminal snapshots Alacritty's renderable content and damage while holding the `Term` lock,
//! then releases it before font shaping or canvas painting. An authoritative visible-row cache
//! merges damaged rows. Canvas prepaint shapes contiguous same-style cells of equal terminal width
//! into row runs, preserving explicit two-column glyph offsets for wide cells. Paint reuses those
//! runs without touching the terminal or shaping text. Selection, cursor, IME, search, hints, and
//! hyperlink hover contribute overlay damage without mutating the grid.
//!
//! ## Protocol and interaction coverage
//!
//! - Legacy and Kitty keyboard encoding, including application cursor/keypad modes,
//!   press/repeat/release events, associated text, and pure IME text events.
//! - X10, UTF-8, SGR, and SGR-pixel mouse reports; local selection, scrollback, and alternate
//!   scrolling.
//! - Bracketed paste normalization, GPUI clipboard and primary-selection integration, and
//!   focus-aware OSC 52 policy enforcement.
//! - Dynamic color and size queries, cursor style/blinking, IME preedit, search, URL/hyperlink
//!   hints, Vi navigation/selection, and an interactive scrollbar.
//!
//! ## Portable PTY quick start
//!
//! Register terminal actions once at application startup. Use [`spawn_portable_pty_session`] to
//! configure the child environment, retain process/master ownership, and obtain separate blocking
//! I/O handles:
//!
//! ```ignore
//! use yttt_terminal::{
//!     TerminalConfig, TerminalSpawnRequest, TerminalView, spawn_portable_pty_session,
//! };
//!
//! yttt_terminal::init(cx);
//!
//! let mut session = spawn_portable_pty_session(
//!     TerminalSpawnRequest::for_shell("shell", "/bin/zsh", "")
//!         .cwd(project_directory),
//! )?;
//! let io = session.take_io().expect("PTY I/O can only be taken once");
//! let resize = session.resize_handle();
//!
//! let terminal = cx.new(|cx| {
//!     TerminalView::new(io.writer, io.reader, TerminalConfig::default(), cx)
//!         .with_resize_callback(move |cols, rows| {
//!             resize
//!                 .resize(cols as usize, rows as usize)
//!                 .map_err(|error| error.to_string())
//!         })
//!         .with_title_callback(|_cx, title| {
//!             eprintln!("title: {title}");
//!         })
//!         .with_io_error_callback(|_cx, operation, message, fatal| {
//!             eprintln!("{operation:?} (fatal={fatal}): {message}");
//!         })
//!         .with_exit_callback(|_cx, reason| {
//!             eprintln!("terminal exited: {reason:?}");
//!         })
//! });
//! terminal.read(cx).focus_handle().focus(window, cx);
//!
//! // Retain `session`, then reap it from a background executor when the view exits.
//! ```
//!
//! [`spawn_portable_pty_session`] applies [`pty::configure_terminal_environment`] automatically.
//! It advertises `TERM=xterm-256color`, true color, and the yttt terminal identity while
//! preserving explicit environment overrides.
//!
//! ## Generic stream cancellation
//!
//! [`TerminalView::new`] also accepts arbitrary [`std::io::Read`] and [`std::io::Write`] values.
//! Dropping the view stops parser and writer work, but Rust cannot forcibly interrupt an arbitrary
//! blocking `Read`. Custom embedders must close or otherwise unblock their reader during shutdown.
//! A reader that returns after shutdown is discarded without touching terminal or GPUI state.
//! [`PortablePtySession`] provides this cancellation and reaping contract for portable PTYs.
//!
//! ## Configuration
//!
//! [`TerminalConfig::default`] uses:
//!
//! - an 80 × 24 grid and 10,000 lines of scrollback;
//! - a visible scrollbar and Alacritty semantic-selection escapes;
//! - a block, non-blinking cursor with 750 ms interval, 5 s timeout, hollow unfocused rendering,
//!   and 0.15 beam/underline thickness;
//! - copy-only OSC 52 access, disabled Kitty keyboard negotiation, and URL hints;
//! - GPUI-managed normal clipboard access, plus primary selection on Linux/FreeBSD.
//!
//! Font, palette, core terminal options, and interaction groups can be updated with
//! [`TerminalView::update_config`]. Font changes invalidate metrics and shaping; palette changes
//! retain metrics; core changes call Alacritty's option update once.
//!
//! ## Callbacks
//!
//! - [`TerminalView::with_resize_callback`] receives `(cols, rows)` and returns
//!   `Result<(), String>`. A failure reports a nonfatal [`PtyIoOperation::Resize`] error while the
//!   local grid still updates.
//! - [`TerminalView::with_title_callback`] receives bounded OSC 0/2 title updates.
//! - [`TerminalView::with_bell_callback`] receives coalesced terminal bells.
//! - [`TerminalView::with_io_error_callback`] receives `(operation, message, fatal)`. Fatal
//!   read/write/mailbox errors are followed by exactly one exit callback.
//! - [`TerminalView::with_exit_callback`] receives [`ExitReason`] without requiring a visible
//!   window.
//! - [`TerminalView::with_key_handler`] may consume host shortcuts before terminal encoding.
//!
//! ## Modules
//!
//! - [`view`]: GPUI view, configuration, bounded driver, callbacks, and interactions.
//! - [`terminal`]: shared Alacritty terminal state and geometry adapters.
//! - [`render`]: immutable terminal snapshots, damage cache, font metrics, and painter.
//! - [`input`] and [`mouse`]: framework-neutral protocol encoders.
//! - [`event`]: bounded Alacritty-to-GPUI event bridge.
//! - [`pty`]: portable-PTY spawning, resize, environment, cancellation, and child reaping.
//! - [`colors`]: configurable ANSI, indexed, selection, search, hint, and cursor colors.

pub mod colors;
pub mod event;
pub mod input;
pub mod mouse;
mod perf;

pub mod pty;
pub mod render;
pub mod terminal;
#[cfg(test)]
pub(crate) mod test_support;
pub mod view;

// Re-export main types for convenience
pub use colors::{ColorPalette, ColorPaletteBuilder};
pub use event::{
    ClipboardFormatter, ColorFormatter, GpuiEventProxy, MAX_OSC52_BYTES, MAX_TITLE_BYTES,
    SizeFormatter, TerminalEvent,
};
pub use input::{
    KeyState, TerminalKey, TerminalKeyEvent, TerminalModifiers, TerminalNamedKey, encode_key, paste,
};
#[cfg(feature = "perf-metrics")]
pub use perf::{
    DurationDistribution, SlowFrameCounts, TerminalLatencyMetrics, TerminalPerformanceCounters,
    TerminalPerformanceDocument, TerminalPerformanceHandle, TerminalPerformanceReporter,
    TerminalPerformanceSemantics, TerminalPerformanceSnapshot,
};
pub use pty::{
    ExitReason, FakeTerminalRuntime, PortablePtyIo, PortablePtyResizeHandle, PortablePtyRuntime,
    PortablePtySession, ProcessHandle, ProcessStatus, PtyEvent, PtyIoOperation, TerminalExecution,
    TerminalRuntime, TerminalSpawnRequest, spawn_portable_pty_session,
};
pub use render::TerminalRenderer;
pub use terminal::TerminalState;
pub use view::{
    BellCallback, DEFAULT_TERMINAL_URL_REGEX, ExitCallback, IoErrorCallback, KeyHandler,
    ResizeCallback, TerminalConfig, TerminalCursorShape, TerminalHintAction, TerminalHintConfig,
    TerminalOsc52Policy, TerminalView, TitleCallback, init, is_valid_hint_alphabet,
};
