//! GPUI terminal view, bounded PTY driver, and interaction adapters.
//!
//! [`TerminalView`] accepts generic [`Read`]/[`Write`] streams while delegating VT parsing,
//! terminal modes, selection, search, and protocol semantics to `alacritty_terminal`.
//!
//! # Concurrency and rendering boundaries
//!
//! - A blocking reader fills a fixed pool of `u16::MAX` buffers and sends them through a
//!   capacity-eight queue.
//! - A parser coordinator owns the VTE processor and returns every buffer to the pool.
//! - An ordered writer handles input, terminal replies, resize callbacks, and shutdown.
//! - Alacritty events enter a bounded, coalescing mailbox; one GPUI task drains it.
//! - Render semantics and damage are snapshotted under the `Term` lock. Font shaping and canvas
//!   painting consume the snapshot after that lock is released.
//!
//! Keyboard, mouse, bracketed paste, OSC 52, IME, search, hints, scrollbar, hyperlink, and Vi
//! behavior all use the same terminal modes and visible-frame snapshot.
//!
//! # Cancellation
//!
//! The driver can stop processing immediately, but Rust cannot interrupt an arbitrary blocking
//! [`Read`]. Embedders using custom streams must close or otherwise unblock their reader during
//! shutdown. A read that returns after shutdown is discarded without touching terminal or GPUI
//! state. [`crate::pty::PortablePtySession`] supplies this lifecycle for portable PTYs.
//!
//! # Example
//!
//! ```ignore
//! yttt_terminal::init(cx);
//! let resize = session.resize_handle();
//! let terminal = cx.new(|cx| {
//!     TerminalView::new(pty_io.writer, pty_io.reader, TerminalConfig::default(), cx)
//!         .with_resize_callback(move |cols, rows| {
//!             resize
//!                 .resize(cols as usize, rows as usize)
//!                 .map_err(|error| error.to_string())
//!         })
//!         .with_io_error_callback(|_cx, operation, message, fatal| {
//!             eprintln!("{operation:?} (fatal={fatal}): {message}");
//!         })
//!         .with_exit_callback(|_cx, reason| {
//!             eprintln!("terminal exited: {reason:?}");
//!         })
//! });
//! terminal.read(cx).focus_handle().focus(window, cx);
//! ```

use crate::colors::ColorPalette;
use crate::event::{GpuiEventProxy, MAX_OSC52_BYTES, TerminalEvent, TerminalEventMailbox};
use crate::input::{KeyState, TerminalKey, TerminalKeyEvent, TerminalModifiers, encode_key, paste};
use crate::mouse::{
    MouseButtonState, TerminalMouseButton, TerminalMouseEvent, TerminalWheelDirection,
    encode_mouse, pixel_to_cell, pixel_to_cell_side, pixels_to_scroll_lines,
    selection_type_from_clicks,
};
use crate::pty::{ExitReason, PtyEvent, PtyIoDriver, PtyIoHandle, PtyIoOperation};
#[cfg(any(test, debug_assertions))]
use crate::render::TerminalDiagnosticsSnapshot;
use crate::render::{
    RenderOverlayState, TerminalRenderCache, TerminalRenderSnapshot, TerminalRenderer,
};
use crate::terminal::TerminalState;
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::index::{Boundary, Column, Direction, Line, Point as AlacPoint, Side};
use alacritty_terminal::selection::{Selection, SelectionType as AlacSelectionType};
use alacritty_terminal::term::cell::Hyperlink;
use alacritty_terminal::term::search::RegexSearch;
use alacritty_terminal::term::{
    ClipboardType, Config as TermConfig, Osc52, SEMANTIC_ESCAPE_CHARS, TermMode,
};
use alacritty_terminal::vi_mode::ViMotion;
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle as TermCursorStyle};
use bytes::{Bytes, BytesMut};
use gpui::prelude::FluentBuilder;
use gpui::{Edges, *};
use std::collections::{BTreeSet, HashSet, VecDeque};
use std::io::{Read, Write};
use std::ops::RangeInclusive;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

const TERMINAL_KEY_CONTEXT: &str = "YtttTerminal";
const TERMINAL_SEARCH_KEY_CONTEXT: &str = "YtttTerminalSearch";
const TERMINAL_HINT_KEY_CONTEXT: &str = "YtttTerminalHint";
const MAX_SEARCH_HISTORY: usize = 100;

/// Alacritty's default URL matcher used by terminal hint mode.
pub const DEFAULT_TERMINAL_URL_REGEX: &str = r#"(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file:|git://|ssh:|ftp://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>"\s{-}\^⟨⟩`\\]+"#;

/// Configured cursor shape.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalCursorShape {
    #[default]
    Block,
    Underline,
    Beam,
}

/// OSC 52 clipboard access policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalOsc52Policy {
    Disabled,
    #[default]
    CopyOnly,
    ReadWrite,
}

/// Action performed by a matched terminal hint.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TerminalHintAction {
    #[default]
    Open,
    Copy,
}

/// One terminal hint matcher.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct TerminalHintConfig {
    pub regex: Option<String>,
    pub hyperlinks: bool,
    pub action: TerminalHintAction,
}

impl Default for TerminalHintConfig {
    fn default() -> Self {
        Self {
            regex: Some(DEFAULT_TERMINAL_URL_REGEX.to_string()),
            hyperlinks: true,
            action: TerminalHintAction::Open,
        }
    }
}

impl TerminalHintConfig {
    pub fn is_valid(&self) -> bool {
        let regex_valid = self
            .regex
            .as_deref()
            .is_none_or(|regex| alacritty_terminal::term::search::RegexSearch::new(regex).is_ok());
        regex_valid && (self.regex.is_some() || self.hyperlinks)
    }
}

actions!(
    yttt_terminal,
    [
        SendTab,
        SendBacktab,
        Copy,
        Paste,
        StartSearch,
        SearchNext,
        SearchPrevious,
        SearchHistoryPrevious,
        SearchHistoryNext,
        CancelSearch,
        ToggleViMode,
        StartHintMode
    ]
);

/// Register terminal-specific key bindings.
pub fn init(cx: &mut App) {
    let mut bindings = vec![
        KeyBinding::new("tab", SendTab, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("shift-tab", SendBacktab, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-space", ToggleViMode, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("enter", SearchNext, Some(TERMINAL_SEARCH_KEY_CONTEXT)),
        KeyBinding::new(
            "shift-enter",
            SearchPrevious,
            Some(TERMINAL_SEARCH_KEY_CONTEXT),
        ),
        KeyBinding::new(
            "up",
            SearchHistoryPrevious,
            Some(TERMINAL_SEARCH_KEY_CONTEXT),
        ),
        KeyBinding::new("down", SearchHistoryNext, Some(TERMINAL_SEARCH_KEY_CONTEXT)),
        KeyBinding::new("escape", CancelSearch, Some(TERMINAL_SEARCH_KEY_CONTEXT)),
        KeyBinding::new("escape", CancelSearch, Some(TERMINAL_HINT_KEY_CONTEXT)),
    ];
    #[cfg(target_os = "macos")]
    bindings.extend([
        KeyBinding::new("cmd-c", Copy, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("cmd-v", Paste, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("cmd-f", StartSearch, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("cmd-shift-o", StartHintMode, Some(TERMINAL_KEY_CONTEXT)),
    ]);
    #[cfg(not(target_os = "macos"))]
    bindings.extend([
        KeyBinding::new("ctrl-shift-c", Copy, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-v", Paste, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-f", StartSearch, Some(TERMINAL_KEY_CONTEXT)),
        KeyBinding::new("ctrl-shift-o", StartHintMode, Some(TERMINAL_KEY_CONTEXT)),
    ]);
    cx.bind_keys(bindings);
}

fn tab_key_down_event(shift: bool) -> &'static KeyDownEvent {
    static TAB_EVENTS: LazyLock<[KeyDownEvent; 2]> = LazyLock::new(|| {
        [
            KeyDownEvent {
                keystroke: Keystroke::parse("tab").expect("static Tab keystroke must parse"),
                is_held: false,
                prefer_character_input: false,
            },
            KeyDownEvent {
                keystroke: Keystroke::parse("shift-tab")
                    .expect("static Shift-Tab keystroke must parse"),
                is_held: false,
                prefer_character_input: false,
            },
        ]
    });
    &TAB_EVENTS[usize::from(shift)]
}

/// Configuration for terminal creation and runtime updates.
///
/// This struct defines the terminal's appearance and behavior, including
/// grid dimensions, font settings, scrollback buffer, and color scheme.
///
/// # Default Values
///
/// | Field | Default |
/// |-------|---------|
/// | `cols` | 80 |
/// | `rows` | 24 |
/// | `font_family` | "monospace" |
/// | `font_size` | 14px |
/// | `scrollback` | 10000 |
/// | `line_height_multiplier` | 1.2 |
/// | `padding` | 0px all sides |
/// | `show_scrollbar` | true |
/// | `colors` | Default palette |
///
/// # Example
///
/// ```ignore
/// use gpui::{Edges, px};
/// use yttt_terminal::{ColorPalette, TerminalConfig};
///
/// let config = TerminalConfig {
///     cols: 120,
///     rows: 40,
///     font_family: "JetBrains Mono".into(),
///     font_size: px(13.0),
///     scrollback: 50000,
///     line_height_multiplier: 1.1,
///     padding: Edges::all(px(10.0)),
///     show_scrollbar: true,
///     colors: ColorPalette::builder()
///         .background(0x1a, 0x1a, 0x1a)
///         .foreground(0xe0, 0xe0, 0xe0)
///         .build(),
///     ..TerminalConfig::default()
/// };
/// ```
///
/// # Runtime Updates
///
/// Configuration can be updated at runtime via [`TerminalView::update_config`].
/// This is useful for implementing features like dynamic font sizing:
///
/// ```ignore
/// terminal.update(cx, |terminal, cx| {
///     let mut config = terminal.config().clone();
///     config.font_size += px(1.0);
///     terminal.update_config(config, cx);
/// });
/// ```
#[derive(Clone, Debug)]
pub struct TerminalConfig {
    /// Number of columns (character width) in the terminal
    pub cols: usize,

    /// Number of rows (lines) in the terminal
    pub rows: usize,

    /// Font family name (e.g., "Fira Code", "JetBrains Mono")
    pub font_family: String,

    /// Font size in pixels
    pub font_size: Pixels,

    /// Maximum number of scrollback lines to keep in history
    pub scrollback: usize,

    /// Multiplier for line height to accommodate tall glyphs (e.g., nerd fonts)
    /// Default is 1.2 (20% extra height)
    pub line_height_multiplier: f32,

    /// Padding around the terminal content (top, right, bottom, left)
    /// The padding area renders with the terminal's background color
    pub padding: Edges<Pixels>,

    /// Whether to draw the thin scrollback indicator and reserve its gutter.
    pub show_scrollbar: bool,

    /// Default cursor shape.
    pub cursor_shape: TerminalCursorShape,

    /// Whether the default cursor blinks.
    pub cursor_blinking: bool,

    /// Cursor blink interval in milliseconds.
    pub cursor_blink_interval_ms: u64,

    /// Seconds after input before cursor blinking stops; zero disables the timeout.
    pub cursor_blink_timeout_secs: u8,

    /// Draw an unfocused block cursor as a hollow outline.
    pub cursor_unfocused_hollow: bool,

    /// Beam/underline cursor thickness as a cell fraction.
    pub cursor_thickness: f32,

    /// Hide the pointer after accepted keyboard input.
    pub hide_mouse_when_typing: bool,

    /// Copy completed selections automatically.
    pub copy_on_select: bool,

    /// Characters terminating semantic selections.
    pub semantic_escape_chars: String,

    /// OSC 52 clipboard policy.
    pub osc52_policy: TerminalOsc52Policy,

    /// Enable the Kitty keyboard protocol.
    pub kitty_keyboard: bool,

    /// Characters used to label keyboard hints.
    pub hint_alphabet: String,

    /// Ordered terminal hint matchers.
    pub hints: Vec<TerminalHintConfig>,

    /// Color palette for terminal colors (16 ANSI colors, 256 extended colors,
    /// foreground, background, and cursor colors)
    pub colors: ColorPalette,
}

impl Default for TerminalConfig {
    fn default() -> Self {
        Self {
            cols: 80,
            rows: 24,
            font_family: "monospace".into(),
            font_size: px(14.0),
            scrollback: 10000,
            line_height_multiplier: 1.2,
            padding: Edges::all(px(0.0)),
            show_scrollbar: true,
            cursor_shape: TerminalCursorShape::Block,
            cursor_blinking: false,
            cursor_blink_interval_ms: 750,
            cursor_blink_timeout_secs: 5,
            cursor_unfocused_hollow: true,
            cursor_thickness: 0.15,
            hide_mouse_when_typing: false,
            copy_on_select: false,
            semantic_escape_chars: SEMANTIC_ESCAPE_CHARS.to_string(),
            osc52_policy: TerminalOsc52Policy::CopyOnly,
            kitty_keyboard: false,
            hint_alphabet: "jfkdls;ahgurieowpq".to_string(),
            hints: vec![TerminalHintConfig::default()],
            colors: ColorPalette::default(),
        }
    }
}

/// Whether every hint label is one terminal column and at least two are available.
pub fn is_valid_hint_alphabet(alphabet: &str) -> bool {
    let mut count = 0;
    for character in alphabet.chars() {
        if unicode_width::UnicodeWidthChar::width(character) != Some(1) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

impl TerminalConfig {
    /// Build the complete Alacritty core configuration.
    pub fn term_options(&self) -> TermConfig {
        let shape = match self.cursor_shape {
            TerminalCursorShape::Block => CursorShape::Block,
            TerminalCursorShape::Underline => CursorShape::Underline,
            TerminalCursorShape::Beam => CursorShape::Beam,
        };
        let osc52 = match self.osc52_policy {
            TerminalOsc52Policy::Disabled => Osc52::Disabled,
            TerminalOsc52Policy::CopyOnly => Osc52::OnlyCopy,
            TerminalOsc52Policy::ReadWrite => Osc52::CopyPaste,
        };

        TermConfig {
            scrolling_history: self.scrollback,
            default_cursor_style: TermCursorStyle {
                shape,
                blinking: self.cursor_blinking,
            },
            vi_mode_cursor_style: None,
            semantic_escape_chars: self.semantic_escape_chars.clone(),
            kitty_keyboard: self.kitty_keyboard,
            osc52,
        }
    }

    /// Normalize interaction values supplied directly by embedders.
    pub fn normalized(mut self) -> Self {
        self.cursor_blink_interval_ms = self.cursor_blink_interval_ms.max(10);
        self.cursor_thickness = if self.cursor_thickness.is_finite() {
            self.cursor_thickness.clamp(0.05, 1.0)
        } else {
            Self::default().cursor_thickness
        };

        let one_column_labels = self
            .hint_alphabet
            .chars()
            .filter(|character| unicode_width::UnicodeWidthChar::width(*character) == Some(1))
            .collect::<String>();
        if one_column_labels.chars().count() >= 2 {
            self.hint_alphabet = one_column_labels;
        } else {
            self.hint_alphabet = Self::default().hint_alphabet;
        }
        self.hints.retain(TerminalHintConfig::is_valid);
        if self.hints.is_empty() {
            self.hints.push(TerminalHintConfig::default());
        }
        self
    }
}

#[derive(Debug)]
struct TerminalSearchState {
    active: bool,
    query: String,
    regex: Option<RegexSearch>,
    error: Option<String>,
    focused_match: Option<RangeInclusive<AlacPoint>>,
    visible_matches: Vec<RangeInclusive<AlacPoint>>,
    visible_cache_key: Option<(u64, usize, usize, usize)>,
    direction: Direction,
    history: VecDeque<String>,
    history_index: Option<usize>,
}

impl Default for TerminalSearchState {
    fn default() -> Self {
        Self {
            active: false,
            query: String::new(),
            regex: None,
            error: None,
            focused_match: None,
            visible_matches: Vec::new(),
            visible_cache_key: None,
            direction: Direction::Right,
            history: VecDeque::new(),
            history_index: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TerminalHintCandidate {
    range: RangeInclusive<AlacPoint>,
    target: String,
    action: TerminalHintAction,
    label: String,
}

#[derive(Debug, Default)]
struct TerminalHintState {
    active: bool,
    typed: String,
    candidates: Vec<TerminalHintCandidate>,
    cache_key: Option<(u64, usize, usize, usize, u64)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct TerminalLinkHit {
    range: RangeInclusive<AlacPoint>,
    uri: String,
}

#[derive(Clone, Copy, Debug)]
struct ScrollbarDragState {
    pointer_offset: f32,
}

/// Callback used by the PTY writer worker to resize the underlying pseudoterminal.
pub use crate::pty::ResizeCallback;

/// Callback type for key event interception.
///
/// This callback is invoked before the terminal processes a key event,
/// allowing you to intercept and handle specific key combinations.
///
/// # Arguments
///
/// * `event` - The key down event from GPUI
///
/// # Returns
///
/// * `true` - Consume the event (terminal will not process it)
/// * `false` - Let the terminal handle the event normally
///
/// # Thread Safety
///
/// This callback must be `Send + Sync`.
///
/// # Example
///
/// ```ignore
/// terminal.with_key_handler(|event| {
///     let keystroke = &event.keystroke;
///
///     // Intercept Ctrl++ for font size increase
///     if keystroke.modifiers.control && (keystroke.key == "+" || keystroke.key == "=") {
///         // Handle font size increase
///         return true; // Consume the event
///     }
///
///     // Intercept Ctrl+- for font size decrease
///     if keystroke.modifiers.control && keystroke.key == "-" {
///         // Handle font size decrease
///         return true;
///     }
///
///     false // Let terminal handle all other keys
/// });
/// ```
pub type KeyHandler = Box<dyn Fn(&TerminalKeyEvent) -> bool + Send + Sync>;

/// Callback for terminal bell events.
pub type BellCallback = Box<dyn Fn(&mut Context<TerminalView>)>;

/// Callback for terminal title changes.
pub type TitleCallback = Box<dyn Fn(&mut Context<TerminalView>, &str)>;

/// Callback invoked exactly once when the PTY lifecycle ends.
pub type ExitCallback = Box<dyn Fn(&mut Context<TerminalView>, ExitReason)>;

/// Callback for PTY read, write, resize, mailbox, input, and clipboard errors.
pub type IoErrorCallback = Box<dyn Fn(&mut Context<TerminalView>, PtyIoOperation, &str, bool)>;

/// The main terminal view component for GPUI applications.
///
/// `TerminalView` is a GPUI entity that implements the [`Render`] trait,
/// providing a complete terminal emulator that can be embedded in any GPUI application.
///
/// # Responsibilities
///
/// - **Terminal State**: Manages the grid, cursor, and colors via [`TerminalState`]
/// - **I/O Streams**: Reads from PTY stdout and writes to PTY stdin
/// - **Event Handling**: Processes keyboard, mouse, and resize events
/// - **Rendering**: Paints text, backgrounds, and cursor via [`TerminalRenderer`]
/// - **Callbacks**: Dispatches events to user-provided callbacks
///
/// # Creating a Terminal
///
/// Use [`TerminalView::new`] within a GPUI entity context:
///
/// ```ignore
/// let terminal = cx.new(|cx| {
///     TerminalView::new(writer, reader, config, cx)
///         .with_resize_callback(resize_callback)
///         .with_exit_callback(|_, cx| cx.quit())
/// });
/// ```
///
/// # Focus
///
/// The terminal must be focused to receive keyboard input:
///
/// ```ignore
/// terminal.read(cx).focus_handle().focus(window);
/// ```
///
/// # Callbacks
///
/// Configure behavior through builder methods:
///
/// - [`with_resize_callback`](Self::with_resize_callback) - PTY size changes
/// - [`with_exit_callback`](Self::with_exit_callback) - Process exit
/// - [`with_key_handler`](Self::with_key_handler) - Key event interception
/// - [`with_bell_callback`](Self::with_bell_callback) - Terminal bell
/// - [`with_title_callback`](Self::with_title_callback) - Title changes
///
/// # Thread Safety
///
/// `TerminalView` is not `Send` as it contains GPUI handles. The stdin writer
/// is internally wrapped in `Arc<parking_lot::Mutex<>>` for safe concurrent access.
pub struct TerminalView {
    /// The terminal state managing the grid and VTE parser
    state: TerminalState,

    /// The renderer for drawing terminal content
    renderer: TerminalRenderer,
    render_cache: Arc<parking_lot::Mutex<TerminalRenderCache>>,

    /// Focus handle for keyboard event handling
    focus_handle: FocusHandle,

    /// Bounded PTY reader/parser/writer driver.
    io_driver: PtyIoDriver,
    /// Cloneable command producer for UI and terminal reply traffic.
    io: PtyIoHandle,
    /// Bounded terminal event mailbox.
    event_mailbox: Arc<TerminalEventMailbox>,
    /// Signal-driven mailbox consumer; remains active while hidden.
    #[allow(dead_code)]
    _event_task: Task<()>,
    /// Runtime terminal configuration.
    config: TerminalConfig,

    /// Optional callback to intercept key events before terminal processing
    key_handler: Option<Arc<KeyHandler>>,
    pressed_keys: HashSet<TerminalKey>,
    pointer_hidden: bool,

    /// Callback for terminal bell events
    bell_callback: Option<BellCallback>,

    /// Callback for terminal title changes
    title_callback: Option<TitleCallback>,

    /// Callback for terminal exit events
    exit_callback: Option<ExitCallback>,
    io_error_callback: Option<IoErrorCallback>,
    exited: bool,

    /// Last painted terminal viewport, used for mouse hit testing.
    viewport: Arc<parking_lot::Mutex<Option<TerminalViewport>>>,

    /// Grid cell and side where the current local selection gesture started.
    selection_anchor: Option<(AlacPoint, Side)>,

    /// Whether the current gesture has produced an active terminal selection.
    selecting: bool,
    held_mouse_button: Option<TerminalMouseButton>,
    pointer_point: Option<AlacPoint>,
    pointer_side: Side,
    selection_scroll_task: Option<Task<()>>,
    selection_scroll_generation: u64,
    selection_scroll_delta: i32,
    selection_scroll_position: Option<Point<Pixels>>,
    search: TerminalSearchState,
    hint: TerminalHintState,
    hovered_link: Option<TerminalLinkHit>,
    scrollbar_drag: Option<ScrollbarDragState>,
    scrollbar_captured: bool,
    render_generation: u64,
    hint_config_generation: u64,
    /// Text currently being composed by the platform input method.
    ime_state: TerminalImeState,
    /// Generation-guarded cursor blink task.
    cursor_blink_task: Option<Task<()>>,

    cursor_blink_generation: u64,
    cursor_visible: bool,
    last_focused: bool,
}

#[derive(Clone, Copy)]
struct TerminalViewport {
    bounds: Bounds<Pixels>,
    padding: Edges<Pixels>,
    cell_width: Pixels,
    cell_height: Pixels,
    cols: usize,
    rows: usize,
    cursor_bounds: Option<Bounds<Pixels>>,
}

#[derive(Default)]
struct TerminalImeState {
    marked_text: Option<String>,
    selected_range_utf16: Option<std::ops::Range<usize>>,
}

impl TerminalImeState {
    fn is_active(&self) -> bool {
        self.marked_text.is_some()
    }

    fn marked_text_range(&self) -> Option<std::ops::Range<usize>> {
        self.marked_text
            .as_ref()
            .map(|text| 0..text.encode_utf16().count())
    }

    fn selected_text_range(&self) -> std::ops::Range<usize> {
        self.selected_range_utf16.clone().unwrap_or(0..0)
    }

    fn caret_utf16(&self) -> usize {
        self.selected_range_utf16
            .as_ref()
            .map_or(0, |range| range.end)
    }

    fn set_marked_text(
        &mut self,
        text: &str,
        selected_range_utf16: Option<std::ops::Range<usize>>,
    ) {
        let len = text.encode_utf16().count();
        let selected = selected_range_utf16.unwrap_or(len..len);
        let start = selected.start.min(len);
        let end = selected.end.min(len).max(start);
        self.marked_text = Some(text.to_owned());
        self.selected_range_utf16 = Some(start..end);
    }

    fn clear_marked_text(&mut self) {
        self.marked_text = None;
        self.selected_range_utf16 = None;
    }

    fn commit_text<'a>(&mut self, text: &'a str) -> Option<&'a str> {
        self.clear_marked_text();
        (!text.is_empty()).then_some(text)
    }

    fn text_for_range(&self, range: std::ops::Range<usize>) -> Option<String> {
        let text = self.marked_text.as_deref()?;
        let start = utf16_byte_offset(text, range.start);
        let end = utf16_byte_offset(text, range.end).max(start);
        Some(text[start..end].to_string())
    }
}

fn utf16_byte_offset(text: &str, target: usize) -> usize {
    let mut units = 0;
    for (byte_offset, character) in text.char_indices() {
        let next = units + character.len_utf16();
        if next > target {
            return byte_offset;
        }
        units = next;
    }
    text.len()
}

impl TerminalView {
    /// Create a new terminal with provided I/O streams.
    ///
    /// This method initializes a new terminal emulator with the given stdin writer
    /// and stdout reader. It spawns a background task to read from stdout and
    /// process incoming bytes through the VTE parser.
    ///
    /// # Arguments
    ///
    /// * `stdin_writer` - Writer for sending input bytes to the terminal process
    /// * `stdout_reader` - Reader for receiving output bytes from the terminal process
    /// * `config` - Terminal configuration (dimensions, font, etc.)
    /// * `cx` - GPUI context for this view
    ///
    /// # Returns
    ///
    /// A new `TerminalView` instance ready to be rendered.
    ///
    /// # Examples
    ///
    /// ```ignore
    /// // In a GPUI window context:
    /// let terminal = cx.new(|cx| {
    ///     TerminalView::new(stdin_writer, stdout_reader, TerminalConfig::default(), cx)
    /// });
    /// ```
    pub fn new<W, R>(
        stdin_writer: W,
        stdout_reader: R,
        config: TerminalConfig,
        cx: &mut Context<Self>,
    ) -> Self
    where
        W: Write + Send + 'static,
        R: Read + Send + 'static,
    {
        let config = config.normalized();
        let (event_mailbox, event_signal) = TerminalEventMailbox::new();
        let event_proxy = GpuiEventProxy::new(event_mailbox.clone());
        let state = TerminalState::new_with_options(
            config.cols,
            config.rows,
            config.term_options(),
            event_proxy,
        );
        let io_driver = PtyIoDriver::start(
            stdin_writer,
            stdout_reader,
            state.term_arc(),
            event_mailbox.clone(),
        );
        let io = io_driver.handle();

        let mut renderer = TerminalRenderer::new(
            config.font_family.clone(),
            config.font_size,
            config.line_height_multiplier,
            config.colors.clone(),
        );
        renderer.cursor_thickness = config.cursor_thickness;
        let focus_handle = cx.focus_handle();

        let event_task = cx.spawn(async move |this: WeakEntity<Self>, cx: &mut AsyncApp| {
            while event_signal.recv().await.is_ok() {
                if this
                    .update(cx, |view: &mut Self, cx: &mut Context<Self>| {
                        view.process_events(cx);
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        Self {
            state,
            renderer,
            render_cache: Arc::new(parking_lot::Mutex::new(TerminalRenderCache::default())),
            focus_handle,
            io_driver,
            io,
            event_mailbox,
            _event_task: event_task,
            config,
            key_handler: None,
            pressed_keys: HashSet::new(),
            pointer_hidden: false,
            bell_callback: None,
            title_callback: None,
            exit_callback: None,
            io_error_callback: None,
            exited: false,
            viewport: Arc::new(parking_lot::Mutex::new(None)),
            selecting: false,
            selection_anchor: None,
            held_mouse_button: None,
            pointer_point: None,
            pointer_side: Side::Left,
            selection_scroll_task: None,
            selection_scroll_generation: 0,
            selection_scroll_delta: 0,
            selection_scroll_position: None,
            search: TerminalSearchState::default(),
            hint: TerminalHintState::default(),
            hovered_link: None,
            scrollbar_drag: None,
            scrollbar_captured: false,
            render_generation: 0,
            hint_config_generation: 0,
            ime_state: TerminalImeState::default(),
            cursor_blink_task: None,
            cursor_blink_generation: 0,
            cursor_visible: true,
            last_focused: false,
        }
    }

    /// Set a callback to be invoked when the terminal is resized.
    ///
    /// This callback should resize the underlying PTY to match the new dimensions.
    /// The callback receives (cols, rows) as arguments.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called with (cols, rows) on resize
    pub fn with_resize_callback(
        self,
        callback: impl Fn(u16, u16) -> Result<(), String> + Send + Sync + 'static,
    ) -> Self {
        self.io.set_resize_callback(Arc::new(callback));
        self
    }

    /// Set a callback to intercept key events before terminal processing.
    ///
    /// The callback receives the key event and should return `true` to consume
    /// the event (prevent the terminal from processing it), or `false` to allow
    /// normal terminal processing.
    ///
    /// # Arguments
    ///
    /// * `handler` - A function that receives key events and returns whether to consume them
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_key_handler(|event| {
    ///     // Handle Ctrl++ to increase font size
    ///     if event.keystroke.modifiers.control && event.keystroke.key == "+" {
    ///         // Handle the event
    ///         return true; // Consume the event
    ///     }
    ///     false // Let terminal handle it
    /// })
    /// ```
    pub fn with_key_handler(
        mut self,
        handler: impl Fn(&TerminalKeyEvent) -> bool + Send + Sync + 'static,
    ) -> Self {
        self.key_handler = Some(Arc::new(Box::new(handler)));
        self
    }

    /// Set a callback to be invoked when the terminal bell is triggered.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// allowing you to play a sound or show a visual indicator.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called when the bell is triggered
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_bell_callback(|window, cx| {
    ///     // Play a sound or flash the screen
    /// })
    /// ```
    pub fn with_bell_callback(
        mut self,
        callback: impl Fn(&mut Context<TerminalView>) + 'static,
    ) -> Self {
        self.bell_callback = Some(Box::new(callback));
        self
    }

    /// Set a callback to be invoked when the terminal title changes.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// along with the new title string.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called with the new title
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_title_callback(|window, cx, title| {
    ///     // Update window title or tab title
    /// })
    /// ```
    pub fn with_title_callback(
        mut self,
        callback: impl Fn(&mut Context<TerminalView>, &str) + 'static,
    ) -> Self {
        self.title_callback = Some(Box::new(callback));
        self
    }

    /// Set a callback to be invoked when the terminal process exits.
    ///
    /// The callback receives a mutable reference to the window and context,
    /// allowing you to close the terminal view or show an exit message.
    ///
    /// # Arguments
    ///
    /// * `callback` - A function that will be called when the process exits
    ///
    /// # Example
    ///
    /// ```ignore
    /// terminal.with_exit_callback(|window, cx| {
    ///     // Close the terminal tab or show exit message
    /// })
    /// ```
    pub fn with_exit_callback(
        mut self,
        callback: impl Fn(&mut Context<TerminalView>, ExitReason) + 'static,
    ) -> Self {
        self.exit_callback = Some(Box::new(callback));
        self
    }

    pub fn with_io_error_callback(
        mut self,
        callback: impl Fn(&mut Context<TerminalView>, PtyIoOperation, &str, bool) + 'static,
    ) -> Self {
        self.io_error_callback = Some(Box::new(callback));
        self
    }

    fn restart_cursor_blink(&mut self, cx: &mut Context<Self>) {
        let blinking = self.last_focused
            && !self.ime_state.is_active()
            && self.state.with_term(|term| term.cursor_style().blinking);
        self.restart_cursor_blink_with(blinking, cx);
    }

    fn restart_cursor_blink_with(&mut self, blinking: bool, cx: &mut Context<Self>) {
        self.cursor_blink_generation = self.cursor_blink_generation.wrapping_add(1);
        self.cursor_blink_task.take();
        let was_hidden = !self.cursor_visible;
        self.cursor_visible = true;
        if was_hidden {
            cx.notify();
        }

        if !self.last_focused || self.ime_state.is_active() || !blinking {
            return;
        }

        let generation = self.cursor_blink_generation;
        let interval = Duration::from_millis(self.config.cursor_blink_interval_ms.max(10));
        let deadline = (self.config.cursor_blink_timeout_secs != 0).then(|| {
            Instant::now() + Duration::from_secs(self.config.cursor_blink_timeout_secs as u64)
        });
        self.cursor_blink_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor().timer(interval).await;
                let result = this.update(cx, |view, cx| {
                    if view.cursor_blink_generation != generation {
                        return false;
                    }
                    if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                        view.cursor_visible = true;
                        cx.notify();
                        return false;
                    }
                    view.cursor_visible = !view.cursor_visible;
                    cx.notify();
                    true
                });
                if !matches!(result, Ok(true)) {
                    break;
                }
            }
        }));
    }

    fn handle_focus_change(&mut self, focused: bool, cx: &mut Context<Self>) {
        if self.last_focused == focused {
            return;
        }
        let mode = self.state.mode();
        cx.set_cursor_hide_mode(if focused && self.config.hide_mouse_when_typing {
            CursorHideMode::OnTypingAndAction
        } else {
            CursorHideMode::Never
        });
        self.last_focused = focused;
        if mode.contains(alacritty_terminal::term::TermMode::FOCUS_IN_OUT) {
            let bytes = if focused {
                Bytes::from_static(b"\x1b[I")
            } else {
                Bytes::from_static(b"\x1b[O")
            };
            let _ = self.enqueue_protocol(bytes);
        }
        if !focused {
            self.stop_selection_scroll();
            self.cancel_hint(cx);
            self.hovered_link = None;
            self.scrollbar_captured = false;
            self.scrollbar_drag = None;
            self.ime_state.clear_marked_text();
            self.pressed_keys.clear();
            self.pointer_hidden = false;
        }
        self.restart_cursor_blink(cx);
        cx.notify();
    }

    fn enqueue_protocol(&self, bytes: Bytes) -> bool {
        self.io.write_input(bytes).is_ok()
    }

    fn enqueue_input(&mut self, bytes: Bytes, cx: &mut Context<Self>) -> bool {
        if bytes.is_empty() {
            return false;
        }
        if let Err(message) = self.io.write_input(bytes) {
            self.report_io_error(PtyIoOperation::InputQueue, &message, false, cx);
            return false;
        }
        let blinking = self.state.with_term_mut(|term| {
            term.selection = None;
            term.scroll_display(Scroll::Bottom);
            term.cursor_style().blinking
        });
        self.selection_anchor = None;
        self.selecting = false;
        self.stop_selection_scroll();
        self.pointer_hidden = self.config.hide_mouse_when_typing;
        self.restart_cursor_blink_with(blinking, cx);
        cx.notify();
        true
    }

    fn report_io_error(
        &self,
        operation: PtyIoOperation,
        message: &str,
        fatal: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(callback) = &self.io_error_callback {
            callback(cx, operation, message, fatal);
        }
    }
    fn enqueue_reply(&self, bytes: Bytes, cx: &mut Context<Self>) {
        if let Err(message) = self.io.write_reply(bytes) {
            self.report_io_error(PtyIoOperation::InputQueue, &message, false, cx);
        }
    }

    fn clipboard_text(target: ClipboardType, cx: &App) -> Option<String> {
        match target {
            ClipboardType::Clipboard => cx.read_from_clipboard().and_then(|item| item.text()),
            ClipboardType::Selection => {
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                {
                    cx.read_from_primary().and_then(|item| item.text())
                }
                #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
                {
                    None
                }
            }
        }
    }

    fn write_clipboard(target: ClipboardType, text: String, cx: &App) -> bool {
        if text.is_empty() {
            return false;
        }
        let item = ClipboardItem::new_string(text);
        match target {
            ClipboardType::Clipboard => {
                cx.write_to_clipboard(item);
                true
            }
            ClipboardType::Selection => {
                #[cfg(any(target_os = "linux", target_os = "freebsd"))]
                {
                    cx.write_to_primary(item);
                    true
                }
                #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
                {
                    false
                }
            }
        }
    }

    fn copy_selection_to_clipboard(&self, cx: &App) -> bool {
        let Some(text) = self
            .state
            .selection_to_string()
            .filter(|text| !text.is_empty())
        else {
            return false;
        };
        Self::write_clipboard(ClipboardType::Clipboard, text, cx)
    }

    fn paste_clipboard_to_terminal(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(text) =
            Self::clipboard_text(ClipboardType::Clipboard, cx).filter(|text| !text.is_empty())
        else {
            return false;
        };
        self.enqueue_input(paste(&text, true, self.state.mode()), cx)
    }

    fn set_search_query(&mut self, query: String, cx: &mut Context<Self>) {
        self.search.query = query;
        self.search.history_index = None;
        self.search.focused_match = None;
        self.search.visible_matches.clear();
        self.search.visible_cache_key = None;
        self.search.error = None;
        self.search.regex = if self.search.query.is_empty() {
            None
        } else {
            match RegexSearch::new(&self.search.query) {
                Ok(regex) => Some(regex),
                Err(error) => {
                    self.search.error = Some(error.to_string());
                    None
                }
            }
        };
        self.refresh_visible_search_matches();
        cx.notify();
    }

    fn append_search_text(&mut self, text: &str, cx: &mut Context<Self>) {
        if text.is_empty() {
            return;
        }
        let mut query = std::mem::take(&mut self.search.query);
        query.push_str(text);
        self.set_search_query(query, cx);
    }

    fn search_backspace(&mut self, cx: &mut Context<Self>) {
        let mut query = std::mem::take(&mut self.search.query);
        query.pop();
        self.set_search_query(query, cx);
    }

    fn record_search_history(&mut self) {
        if self.search.query.is_empty() || self.search.history.back() == Some(&self.search.query) {
            return;
        }
        if self.search.history.len() == MAX_SEARCH_HISTORY {
            self.search.history.pop_front();
        }
        self.search.history.push_back(self.search.query.clone());
    }

    fn search_history_previous(&mut self, cx: &mut Context<Self>) {
        if self.search.history.is_empty() {
            return;
        }
        let index = self
            .search
            .history_index
            .map_or(self.search.history.len() - 1, |index| {
                index.saturating_sub(1)
            });
        let query = self.search.history[index].clone();
        self.set_search_query(query, cx);
        self.search.history_index = Some(index);
    }

    fn search_history_next(&mut self, cx: &mut Context<Self>) {
        let Some(index) = self.search.history_index else {
            return;
        };
        if index + 1 < self.search.history.len() {
            let next = index + 1;
            let query = self.search.history[next].clone();
            self.set_search_query(query, cx);
            self.search.history_index = Some(next);
        } else {
            self.set_search_query(String::new(), cx);
        }
    }

    fn refresh_visible_search_matches(&mut self) {
        if !self.search.active || self.search.regex.is_none() {
            self.search.visible_matches.clear();
            self.search.visible_cache_key = None;
            return;
        }
        let display_offset = self.state.display_offset();
        let cols = self.state.cols();
        let rows = self.state.rows();
        let key = (self.render_generation, display_offset, cols, rows);
        if self.search.visible_cache_key == Some(key) {
            return;
        }

        let Some(mut regex) = self.search.regex.take() else {
            return;
        };
        let matches = self.state.with_term(|term| {
            let top = Line(-(display_offset as i32));
            let bottom = top + rows.saturating_sub(1) as i32;
            let mut origin = AlacPoint::new(top, Column(0));
            let mut matches = Vec::new();
            let limit = cols.saturating_mul(rows).max(1);
            for _ in 0..limit {
                let Some(range) = term.search_next(
                    &mut regex,
                    origin,
                    Direction::Right,
                    Side::Left,
                    Some(rows.saturating_sub(1)),
                ) else {
                    break;
                };
                if range.end().line < top || range.start().line > bottom {
                    break;
                }
                if matches.iter().any(|existing| existing == &range) {
                    break;
                }
                let end = *range.end();
                matches.push(range);
                let next = end.add(term, Boundary::Grid, 1);
                if next == end || next.line > bottom {
                    break;
                }
                origin = next;
            }
            matches
        });
        self.search.regex = Some(regex);
        self.search.visible_matches = matches;
        self.search.visible_cache_key = Some(key);
    }

    fn navigate_search(&mut self, direction: Direction, cx: &mut Context<Self>) {
        let Some(mut regex) = self.search.regex.take() else {
            return;
        };
        self.record_search_history();
        self.search.direction = direction;
        let current = self.search.focused_match.clone();
        let found = self.state.with_term_mut(|term| {
            let top = term.topmost_line();
            let bottom = term.bottommost_line();
            let last_column = term.last_column();
            let origin = match (direction, current.as_ref()) {
                (Direction::Right, Some(range)) => {
                    let end = *range.end();
                    if end.line == bottom && end.column == last_column {
                        AlacPoint::new(top, Column(0))
                    } else {
                        end.add(term, Boundary::Grid, 1)
                    }
                }
                (Direction::Left, Some(range)) => {
                    let start = *range.start();
                    if start.line == top && start.column == Column(0) {
                        AlacPoint::new(bottom, last_column)
                    } else {
                        start.sub(term, Boundary::Grid, 1)
                    }
                }
                (Direction::Right, None) => AlacPoint::new(top, Column(0)),
                (Direction::Left, None) => AlacPoint::new(bottom, last_column),
            };
            let found = term.search_next(&mut regex, origin, direction, Side::Left, None);
            if let Some(range) = &found {
                let point = match direction {
                    Direction::Right => *range.start(),
                    Direction::Left => *range.end(),
                };
                if term.mode().contains(TermMode::VI) {
                    term.vi_goto_point(point);
                } else {
                    term.scroll_to_point(point);
                }
            }
            found
        });
        self.search.regex = Some(regex);
        self.search.focused_match = found;
        self.search.visible_cache_key = None;
        self.refresh_visible_search_matches();
        cx.notify();
    }

    fn cancel_search(&mut self, cx: &mut Context<Self>) {
        if !self.search.active {
            return;
        }
        self.search.active = false;
        self.search.focused_match = None;
        self.search.visible_matches.clear();
        self.search.visible_cache_key = None;
        self.search.history_index = None;
        cx.notify();
    }

    fn visible_hyperlink_spans(&self) -> Vec<(RangeInclusive<AlacPoint>, String)> {
        let cache = self.render_cache.lock();
        let Some(frame) = cache.frame() else {
            return Vec::new();
        };
        let mut spans = Vec::new();
        let mut current: Option<(Hyperlink, AlacPoint, AlacPoint)> = None;
        for cell in frame.rows.iter().flat_map(|row| row.cells.iter()) {
            let Some(hyperlink) = cell.hyperlink.clone() else {
                if let Some((hyperlink, start, end)) = current.take() {
                    spans.push((start..=end, hyperlink.uri().to_string()));
                }
                continue;
            };
            let adjacent = current.as_ref().is_some_and(|(_, _, previous)| {
                (cell.point.line == previous.line && cell.point.column.0 == previous.column.0 + 1)
                    || (cell.point.line == previous.line + 1
                        && cell.point.column == Column(0)
                        && previous.column.0 + 1 >= frame.cols)
            });
            if current
                .as_ref()
                .is_some_and(|(active, _, _)| *active == hyperlink && adjacent)
            {
                if let Some((_, _, end)) = current.as_mut() {
                    *end = cell.point;
                }
            } else {
                if let Some((active, start, end)) = current.take() {
                    spans.push((start..=end, active.uri().to_string()));
                }
                current = Some((hyperlink, cell.point, cell.point));
            }
        }
        if let Some((hyperlink, start, end)) = current {
            spans.push((start..=end, hyperlink.uri().to_string()));
        }
        spans
    }

    fn refresh_hint_candidates(&mut self) {
        if !self.hint.active {
            return;
        }
        let display_offset = self.state.display_offset();
        let cols = self.state.cols();
        let rows = self.state.rows();
        let key = (
            self.render_generation,
            display_offset,
            cols,
            rows,
            self.hint_config_generation,
        );
        if self.hint.cache_key == Some(key) {
            return;
        }

        let hyperlinks = self.visible_hyperlink_spans();
        let configs = self.config.hints.clone();
        let mut candidates = self.state.with_term(|term| {
            let top = Line(-(display_offset as i32));
            let bottom = top + rows.saturating_sub(1) as i32;
            let mut candidates: Vec<TerminalHintCandidate> = Vec::new();
            for config in configs {
                if let Some(pattern) = config.regex.as_deref()
                    && let Ok(mut regex) = RegexSearch::new(pattern)
                {
                    let mut origin = AlacPoint::new(top, Column(0));
                    let mut seen_ranges = BTreeSet::new();
                    let limit = cols.saturating_mul(rows).max(1);
                    for _ in 0..limit {
                        let Some(range) = term.search_next(
                            &mut regex,
                            origin,
                            Direction::Right,
                            Side::Left,
                            Some(rows.saturating_sub(1)),
                        ) else {
                            break;
                        };
                        if range.end().line < top || range.start().line > bottom {
                            break;
                        }
                        if !seen_ranges.insert((*range.start(), *range.end())) {
                            break;
                        }
                        let target = hyperlinks
                            .iter()
                            .find(|(hyperlink_range, _)| *hyperlink_range == range)
                            .map(|(_, uri)| uri.clone())
                            .unwrap_or_else(|| term.bounds_to_string(*range.start(), *range.end()));
                        let candidate = TerminalHintCandidate {
                            range: range.clone(),
                            target,
                            action: config.action,
                            label: String::new(),
                        };
                        if !candidates.iter().any(|existing| {
                            existing.range == candidate.range
                                && existing.target == candidate.target
                                && existing.action == candidate.action
                        }) {
                            candidates.push(candidate);
                        }
                        let end = *range.end();
                        let next = end.add(term, Boundary::Grid, 1);
                        if next == end || next.line > bottom {
                            break;
                        }
                        origin = next;
                    }
                }

                if config.hyperlinks {
                    for (range, uri) in &hyperlinks {
                        let candidate = TerminalHintCandidate {
                            range: range.clone(),
                            target: uri.clone(),
                            action: config.action,
                            label: String::new(),
                        };
                        if !candidates.iter().any(|existing| {
                            existing.range == candidate.range
                                && existing.target == candidate.target
                                && existing.action == candidate.action
                        }) {
                            candidates.push(candidate);
                        }
                    }
                }
            }
            candidates
        });
        candidates.sort_by(|left, right| {
            left.range
                .start()
                .cmp(right.range.start())
                .then_with(|| left.range.end().cmp(right.range.end()))
        });
        let labels = Self::hint_labels(candidates.len(), &self.config.hint_alphabet);
        for (candidate, label) in candidates.iter_mut().zip(labels) {
            candidate.label = label;
        }
        self.hint.candidates = candidates;
        self.hint.cache_key = Some(key);
    }

    fn hint_labels(count: usize, alphabet: &str) -> Vec<String> {
        let alphabet = alphabet.chars().collect::<Vec<_>>();
        if count == 0 || alphabet.len() < 2 {
            return Vec::new();
        }
        let split_point = ((alphabet.len() - 1) as f32 * 0.5) as usize;
        let mut indices = vec![0usize];
        let mut labels = Vec::with_capacity(count);
        for _ in 0..count {
            labels.push(indices.iter().rev().map(|index| alphabet[*index]).collect());

            if indices[0] < split_point {
                indices[0] += 1;
                continue;
            }
            indices[0] = 0;
            let mut incremented = false;
            for index in indices.iter_mut().skip(1) {
                if *index + 1 == alphabet.len() {
                    *index = split_point + 1;
                } else {
                    *index += 1;
                    incremented = true;
                    break;
                }
            }
            if !incremented {
                indices.push(split_point + 1);
            }
        }
        labels
    }

    fn cancel_hint(&mut self, cx: &mut Context<Self>) {
        if !self.hint.active {
            return;
        }
        self.hint = TerminalHintState::default();
        cx.notify();
    }

    fn execute_hint(&mut self, candidate: TerminalHintCandidate, cx: &mut Context<Self>) {
        match candidate.action {
            TerminalHintAction::Open => cx.open_url(&candidate.target),
            TerminalHintAction::Copy => {
                let _ = Self::write_clipboard(ClipboardType::Clipboard, candidate.target, cx);
            }
        }
        self.cancel_hint(cx);
    }

    fn handle_hint_text(&mut self, text: &str, cx: &mut Context<Self>) {
        let Some(character) = text.chars().next() else {
            return;
        };
        if text.chars().count() != 1 || !self.config.hint_alphabet.contains(character) {
            return;
        }
        self.hint.typed.push(character);
        let matching = self
            .hint
            .candidates
            .iter()
            .filter(|candidate| candidate.label.starts_with(&self.hint.typed))
            .cloned()
            .collect::<Vec<_>>();
        if matching.is_empty() {
            self.cancel_hint(cx);
        } else if let Some(candidate) = matching
            .iter()
            .find(|candidate| candidate.label == self.hint.typed)
            .cloned()
        {
            self.execute_hint(candidate, cx);
        } else {
            cx.notify();
        }
    }

    fn render_overlays(&self) -> RenderOverlayState {
        let mut overlays = RenderOverlayState {
            search_matches: self.search.visible_matches.clone(),
            focused_search_match: self.search.focused_match.clone(),
            hovered_hyperlink: self.hovered_link.as_ref().map(|link| link.range.clone()),
            ..RenderOverlayState::default()
        };
        if !self.hint.active {
            return overlays;
        }

        let cols = self.state.cols();
        let max_cells = cols.saturating_mul(self.state.rows()).max(1);
        for candidate in self
            .hint
            .candidates
            .iter()
            .filter(|candidate| candidate.label.starts_with(&self.hint.typed))
        {
            let mut point = *candidate.range.start();
            let mut label = candidate.label.chars();
            for _ in 0..max_cells {
                let label_character = label.next();
                overlays.hints.push(crate::render::HintCellOverlay {
                    point,
                    label: label_character,
                    is_start: label_character.is_some(),
                });
                if point == *candidate.range.end() {
                    break;
                }
                if point.column.0 + 1 < cols {
                    point.column += 1;
                } else {
                    point.line += 1;
                    point.column = Column(0);
                }
            }
        }
        overlays
    }

    fn hyperlink_at_point(&self, point: AlacPoint) -> Option<TerminalLinkHit> {
        self.visible_hyperlink_spans()
            .into_iter()
            .find(|(range, _)| range.contains(&point))
            .map(|(range, uri)| TerminalLinkHit { range, uri })
    }

    fn point_and_side_for_position(&self, position: Point<Pixels>) -> Option<(AlacPoint, Side)> {
        let viewport = (*self.viewport.lock())?;
        let origin = point(
            viewport.bounds.origin.x + viewport.padding.left,
            viewport.bounds.origin.y + viewport.padding.top,
        );
        let display_offset = self.state.display_offset() as i32;
        let raw = pixel_to_cell(position, origin, viewport.cell_width, viewport.cell_height);
        let row = raw.line.0.clamp(0, viewport.rows.saturating_sub(1) as i32);
        let column = raw.column.0.min(viewport.cols.saturating_sub(1));
        let side = pixel_to_cell_side(position.x, origin.x, viewport.cell_width, viewport.cols);
        Some((
            AlacPoint::new(Line(row - display_offset), Column(column)),
            side,
        ))
    }

    fn point_for_position(&self, position: Point<Pixels>) -> Option<AlacPoint> {
        self.point_and_side_for_position(position)
            .map(|(point, _)| point)
    }

    fn scrollbar_geometry(&self) -> Option<(f32, f32, f32, f32, usize, usize)> {
        if !self.config.show_scrollbar {
            return None;
        }
        let viewport = (*self.viewport.lock())?;
        let metrics = self.state.scrollbar_metrics()?;
        let track_top: f32 = (viewport.bounds.origin.y + viewport.padding.top).into();
        let track_height: f32 =
            (viewport.bounds.size.height - viewport.padding.top - viewport.padding.bottom).into();
        if track_height <= 12.0 {
            return None;
        }
        let thumb_height = track_height * metrics.thumb_height_fraction;
        let thumb_top = track_top + track_height * metrics.thumb_top_fraction;
        let (history_size, display_offset) = self.state.with_term(|term| {
            (
                term.grid()
                    .total_lines()
                    .saturating_sub(term.screen_lines()),
                term.grid().display_offset(),
            )
        });
        Some((
            track_top,
            track_height,
            thumb_top,
            thumb_height,
            history_size,
            display_offset,
        ))
    }

    fn scrollbar_contains(&self, position: Point<Pixels>) -> bool {
        let Some(viewport) = *self.viewport.lock() else {
            return false;
        };
        let x: f32 = position.x.into();
        let left: f32 = (viewport.bounds.origin.x + viewport.bounds.size.width - px(6.0)).into();
        let right: f32 = (viewport.bounds.origin.x + viewport.bounds.size.width).into();
        let y: f32 = position.y.into();
        let top: f32 = viewport.bounds.origin.y.into();
        let bottom: f32 = (viewport.bounds.origin.y + viewport.bounds.size.height).into();
        (left..=right).contains(&x) && (top..=bottom).contains(&y)
    }

    fn handle_scrollbar_down(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        if !self.scrollbar_contains(position) {
            return false;
        }
        let Some((_, _, thumb_top, thumb_height, _, _)) = self.scrollbar_geometry() else {
            return false;
        };
        let y: f32 = position.y.into();
        self.scrollbar_captured = true;
        if (thumb_top..=thumb_top + thumb_height).contains(&y) {
            self.scrollbar_drag = Some(ScrollbarDragState {
                pointer_offset: y - thumb_top,
            });
        } else {
            self.state.scroll_display(if y < thumb_top {
                Scroll::PageUp
            } else {
                Scroll::PageDown
            });
        }
        self.search.visible_cache_key = None;
        self.hint.cache_key = None;
        cx.notify();
        true
    }

    fn update_scrollbar_drag(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) -> bool {
        let Some(drag) = self.scrollbar_drag else {
            return false;
        };
        let Some((track_top, track_height, _, thumb_height, history_size, display_offset)) =
            self.scrollbar_geometry()
        else {
            self.scrollbar_drag = None;
            return false;
        };
        let travel = (track_height - thumb_height).max(1.0);
        let y: f32 = position.y.into();
        let thumb_top = (y - track_top - drag.pointer_offset).clamp(0.0, travel);
        let progress_from_top = thumb_top / travel;
        let desired_offset = ((1.0 - progress_from_top) * history_size as f32).round() as usize;
        let delta = desired_offset as i64 - display_offset as i64;
        self.state.scroll_display(Scroll::Delta(
            delta.clamp(i32::MIN as i64, i32::MAX as i64) as i32
        ));
        self.search.visible_cache_key = None;
        self.hint.cache_key = None;
        cx.notify();
        true
    }

    fn terminal_mouse_button(button: MouseButton) -> Option<TerminalMouseButton> {
        match button {
            MouseButton::Left => Some(TerminalMouseButton::Left),
            MouseButton::Middle => Some(TerminalMouseButton::Middle),
            MouseButton::Right => Some(TerminalMouseButton::Right),
            _ => None,
        }
    }

    fn terminal_modifiers(modifiers: Modifiers) -> TerminalModifiers {
        TerminalModifiers {
            shift: modifiers.shift,
            alt: modifiers.alt,
            control: modifiers.control,
            super_key: modifiers.platform,
        }
    }
    fn mouse_reporting(mode: TermMode) -> bool {
        mode.intersects(
            TermMode::MOUSE_REPORT_CLICK | TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION,
        )
    }

    fn selection_scroll_delta_for_position(&self, position: Point<Pixels>) -> i32 {
        let Some(viewport) = *self.viewport.lock() else {
            return 0;
        };
        let top: f32 = (viewport.bounds.origin.y + viewport.padding.top).into();
        let cell_height: f32 = viewport.cell_height.into();
        let bottom = top + cell_height * viewport.rows as f32;
        let y: f32 = position.y.into();
        if y < top {
            1 + (((top - y) / cell_height).floor() as i32).min(5)
        } else if y >= bottom {
            -(1 + (((y - bottom) / cell_height).floor() as i32).min(5))
        } else {
            0
        }
    }

    fn stop_selection_scroll(&mut self) {
        self.selection_scroll_generation = self.selection_scroll_generation.wrapping_add(1);
        self.selection_scroll_task.take();
        self.selection_scroll_delta = 0;
        self.selection_scroll_position = None;
    }

    fn update_selection_scroll(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        let delta = self.selection_scroll_delta_for_position(position);
        self.selection_scroll_position = Some(position);
        if delta == 0 || !self.selecting {
            self.stop_selection_scroll();
            return;
        }
        if self.selection_scroll_task.is_some() && self.selection_scroll_delta == delta {
            return;
        }

        self.selection_scroll_generation = self.selection_scroll_generation.wrapping_add(1);
        self.selection_scroll_task.take();
        self.selection_scroll_delta = delta;
        let generation = self.selection_scroll_generation;
        self.selection_scroll_task = Some(cx.spawn(async move |this, cx| {
            loop {
                cx.background_executor()
                    .timer(Duration::from_millis(16))
                    .await;
                let keep_running = this
                    .update(cx, |view, cx| {
                        if view.selection_scroll_generation != generation
                            || !view.selecting
                            || view.selection_scroll_delta == 0
                        {
                            return false;
                        }
                        view.state
                            .scroll_display(Scroll::Delta(view.selection_scroll_delta));
                        if let Some(position) = view.selection_scroll_position
                            && let Some((point, side)) = view.point_and_side_for_position(position)
                        {
                            view.pointer_point = Some(point);
                            view.pointer_side = side;
                            view.state.update_selection(point, side);
                        }
                        cx.notify();
                        true
                    })
                    .unwrap_or(false);
                if !keep_running {
                    break;
                }
            }
        }));
    }

    fn selection_type(click_count: usize) -> AlacSelectionType {
        match selection_type_from_clicks(click_count.max(1)) {
            crate::mouse::SelectionType::Simple => AlacSelectionType::Simple,
            crate::mouse::SelectionType::Word => AlacSelectionType::Semantic,
            crate::mouse::SelectionType::Line => AlacSelectionType::Lines,
        }
    }

    fn ime_cursor_bounds(&self, caret_utf16: usize, window: &mut Window) -> Option<Bounds<Pixels>> {
        let viewport = (*self.viewport.lock())?;
        let mut bounds = viewport.cursor_bounds?;
        let marked_text = self.ime_state.marked_text.as_deref()?;
        bounds.origin.x += self
            .renderer
            .ime_caret_offset(marked_text, caret_utf16, window);
        Some(bounds)
    }

    /// Handle keyboard input events.
    ///
    /// Converts GPUI keystrokes to terminal escape sequences and writes them
    /// to the stdin writer. If a key handler is set and returns true, the event
    /// is consumed and not sent to the terminal.
    fn on_send_tab(&mut self, _: &SendTab, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.active || self.hint.active {
            cx.stop_propagation();
        } else {
            self.on_key_down(tab_key_down_event(false), window, cx);
        }
    }

    fn on_send_backtab(&mut self, _: &SendBacktab, window: &mut Window, cx: &mut Context<Self>) {
        if self.search.active || self.hint.active {
            cx.stop_propagation();
        } else {
            self.on_key_down(tab_key_down_event(true), window, cx);
        }
    }

    fn on_copy(&mut self, _: &Copy, _window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.copy_selection_to_clipboard(cx);
        cx.stop_propagation();
    }

    fn on_paste(&mut self, _: &Paste, _window: &mut Window, cx: &mut Context<Self>) {
        if self.hint.active {
            cx.stop_propagation();
            return;
        }
        if self.search.active {
            if let Some(text) = Self::clipboard_text(ClipboardType::Clipboard, cx) {
                self.append_search_text(&text, cx);
            }
        } else {
            let _ = self.paste_clipboard_to_terminal(cx);
        }
        cx.stop_propagation();
    }

    fn on_start_search(&mut self, _: &StartSearch, _window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_hint(cx);
        self.search.active = true;
        self.search.history_index = None;
        self.search.visible_cache_key = None;
        self.refresh_visible_search_matches();
        cx.notify();
        cx.stop_propagation();
    }

    fn on_search_next(&mut self, _: &SearchNext, _window: &mut Window, cx: &mut Context<Self>) {
        self.navigate_search(Direction::Right, cx);
        cx.stop_propagation();
    }

    fn on_search_previous(
        &mut self,
        _: &SearchPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.navigate_search(Direction::Left, cx);
        cx.stop_propagation();
    }

    fn on_search_history_previous(
        &mut self,
        _: &SearchHistoryPrevious,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_history_previous(cx);
        cx.stop_propagation();
    }

    fn on_search_history_next(
        &mut self,
        _: &SearchHistoryNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.search_history_next(cx);
        cx.stop_propagation();
    }

    fn on_cancel_search(&mut self, _: &CancelSearch, _window: &mut Window, cx: &mut Context<Self>) {
        self.cancel_search(cx);
        self.cancel_hint(cx);
        cx.stop_propagation();
    }

    fn on_toggle_vi_mode(
        &mut self,
        _: &ToggleViMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.state.with_term_mut(|term| term.toggle_vi_mode());
        self.restart_cursor_blink(cx);
        cx.notify();
        cx.stop_propagation();
    }

    fn on_start_hint_mode(
        &mut self,
        _: &StartHintMode,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.cancel_search(cx);
        self.hint.active = true;
        self.hint.typed.clear();
        self.hint.cache_key = None;
        self.refresh_hint_candidates();
        cx.notify();
        cx.stop_propagation();
    }

    fn handle_vi_key(
        &mut self,
        key_event: &TerminalKeyEvent,
        mode: TermMode,
        cx: &mut Context<Self>,
    ) -> bool {
        if !mode.contains(TermMode::VI) {
            return false;
        }
        let character = key_event
            .text
            .as_deref()
            .filter(|text| text.chars().count() == 1);
        if matches!(
            key_event.key,
            TerminalKey::Named(crate::input::TerminalNamedKey::Escape)
        ) {
            self.state.with_term_mut(|term| term.toggle_vi_mode());
            self.restart_cursor_blink(cx);
            cx.notify();
            return true;
        }
        if character == Some("v") {
            self.state.with_term_mut(|term| {
                if term.selection.is_some() {
                    term.selection = None;
                } else {
                    term.selection = Some(Selection::new(
                        AlacSelectionType::Simple,
                        term.vi_mode_cursor.point,
                        Side::Left,
                    ));
                }
            });
            cx.notify();
            return true;
        }
        if character == Some("y") {
            let _ = self.copy_selection_to_clipboard(cx);
            self.state.clear_selection();
            cx.notify();
            return true;
        }

        let motion =
            match (&key_event.key, character) {
                (TerminalKey::Named(crate::input::TerminalNamedKey::ArrowLeft), _)
                | (_, Some("h")) => Some(ViMotion::Left),
                (TerminalKey::Named(crate::input::TerminalNamedKey::ArrowDown), _)
                | (_, Some("j")) => Some(ViMotion::Down),
                (TerminalKey::Named(crate::input::TerminalNamedKey::ArrowUp), _)
                | (_, Some("k")) => Some(ViMotion::Up),
                (TerminalKey::Named(crate::input::TerminalNamedKey::ArrowRight), _)
                | (_, Some("l")) => Some(ViMotion::Right),
                (_, Some("0")) => Some(ViMotion::First),
                (_, Some("$")) => Some(ViMotion::Last),
                (_, Some("^")) => Some(ViMotion::FirstOccupied),
                (_, Some("H")) => Some(ViMotion::High),
                (_, Some("M")) => Some(ViMotion::Middle),
                (_, Some("L")) => Some(ViMotion::Low),
                (_, Some("b")) => Some(ViMotion::SemanticLeft),
                (_, Some("w")) => Some(ViMotion::SemanticRight),
                (_, Some("e")) => Some(ViMotion::SemanticRightEnd),
                (_, Some("%")) => Some(ViMotion::Bracket),
                (_, Some("{")) => Some(ViMotion::ParagraphUp),
                (_, Some("}")) => Some(ViMotion::ParagraphDown),
                _ => None,
            };
        if let Some(motion) = motion {
            self.state.with_term_mut(|term| term.vi_motion(motion));
            cx.notify();
        }
        true
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(key_event) = TerminalKeyEvent::from_key_down(event) else {
            return;
        };
        if self.search.active {
            if matches!(
                key_event.key,
                TerminalKey::Named(crate::input::TerminalNamedKey::Backspace)
            ) {
                self.search_backspace(cx);
            } else if key_event.state == KeyState::Pressed
                && !key_event.modifiers.control
                && !key_event.modifiers.alt
                && !key_event.modifiers.super_key
                && let Some(text) = key_event.text.as_deref()
            {
                self.append_search_text(text, cx);
            }
            cx.stop_propagation();
            return;
        }
        if self.hint.active {
            if key_event.state == KeyState::Pressed
                && let Some(text) = key_event.text.as_deref()
            {
                self.handle_hint_text(text, cx);
            }
            cx.stop_propagation();
            return;
        }
        if self.ime_state.is_active() {
            cx.stop_propagation();
            return;
        }
        let mode = self.state.mode();
        if self.handle_vi_key(&key_event, mode, cx) {
            cx.stop_propagation();
            return;
        }
        if let Some(handler) = &self.key_handler
            && handler(&key_event)
        {
            cx.stop_propagation();
            return;
        }

        if event.keystroke.modifiers.platform && !event.prefer_character_input {
            return;
        }

        if let Some(bytes) = encode_key(&key_event, mode) {
            let accepted = self.enqueue_input(Bytes::copy_from_slice(&bytes), cx);
            if accepted && key_event.state == KeyState::Pressed {
                self.pressed_keys.insert(key_event.identity());
            }
            cx.stop_propagation();
        }
    }

    fn on_key_up(&mut self, event: &KeyUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(key_event) = TerminalKeyEvent::from_key_up(event) else {
            return;
        };
        let identity = key_event.identity();
        if self.search.active
            || self.hint.active
            || self.ime_state.is_active()
            || self.state.mode().contains(TermMode::VI)
        {
            self.pressed_keys.remove(&identity);
            cx.stop_propagation();
            return;
        }
        if let Some(handler) = &self.key_handler
            && handler(&key_event)
        {
            self.pressed_keys.remove(&identity);
            cx.stop_propagation();
            return;
        }
        if !self.pressed_keys.remove(&identity) {
            return;
        }
        if let Some(bytes) = encode_key(&key_event, self.state.mode()) {
            let _ = self.enqueue_protocol(Bytes::copy_from_slice(&bytes));
        }
        cx.stop_propagation();
    }

    /// Handle mouse down events.
    ///
    /// Currently a placeholder for future mouse selection and interaction support.
    fn on_mouse_down(
        &mut self,
        event: &MouseDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        window.focus(&self.focus_handle, cx);
        if event.button == MouseButton::Left && self.handle_scrollbar_down(event.position, cx) {
            return;
        }
        if self.hint.active {
            self.cancel_hint(cx);
        }
        self.pointer_hidden = false;

        let Some(button) = Self::terminal_mouse_button(event.button) else {
            return;
        };
        self.held_mouse_button = Some(button);
        let Some((point, side)) = self.point_and_side_for_position(event.position) else {
            return;
        };
        self.pointer_point = Some(point);
        self.pointer_side = side;
        self.hovered_link = self.hyperlink_at_point(point);

        let mode = self.state.mode();
        let modifiers = Self::terminal_modifiers(event.modifiers);
        if button == TerminalMouseButton::Left
            && modifiers.super_key
            && let Some(link) = self.hovered_link.clone()
        {
            self.held_mouse_button = None;
            cx.open_url(&link.uri);
            cx.notify();
            return;
        }
        if !modifiers.shift && Self::mouse_reporting(mode) {
            if let Some(bytes) = encode_mouse(
                TerminalMouseEvent::Button {
                    button,
                    state: MouseButtonState::Pressed,
                    point,
                },
                modifiers,
                mode,
            ) {
                self.enqueue_protocol(Bytes::copy_from_slice(&bytes));
            }
            cx.notify();
            return;
        }

        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        if button == TerminalMouseButton::Middle {
            if let Some(text) =
                Self::clipboard_text(ClipboardType::Selection, cx).filter(|text| !text.is_empty())
            {
                let _ = self.enqueue_input(paste(&text, true, mode), cx);
            }
            return;
        }

        if button == TerminalMouseButton::Left {
            self.stop_selection_scroll();
            self.selection_anchor = Some((point, side));
            self.selecting = event.click_count > 1;
            if self.selecting {
                self.state
                    .begin_selection(point, side, Self::selection_type(event.click_count));
            } else {
                self.state.clear_selection();
            }
        }
        cx.notify();
    }

    /// Handle mouse up events.
    ///
    /// Currently a placeholder for future mouse selection support.
    fn on_mouse_up(&mut self, event: &MouseUpEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if event.button == MouseButton::Left && self.scrollbar_captured {
            self.scrollbar_captured = false;
            self.scrollbar_drag = None;
            cx.notify();
            return;
        }
        self.pointer_hidden = false;
        let Some(button) = Self::terminal_mouse_button(event.button) else {
            return;
        };
        if self.held_mouse_button == Some(button) {
            self.held_mouse_button = None;
        }

        let Some((point, side)) = self.point_and_side_for_position(event.position) else {
            if button == TerminalMouseButton::Left {
                self.selection_anchor = None;
                self.selecting = false;
                self.stop_selection_scroll();
            }
            return;
        };
        self.pointer_point = Some(point);
        self.pointer_side = side;

        let mode = self.state.mode();
        let modifiers = Self::terminal_modifiers(event.modifiers);
        let local_left_gesture =
            button == TerminalMouseButton::Left && self.selection_anchor.is_some();
        if !modifiers.shift && !local_left_gesture && Self::mouse_reporting(mode) {
            if let Some(bytes) = encode_mouse(
                TerminalMouseEvent::Button {
                    button,
                    state: MouseButtonState::Released,
                    point,
                },
                modifiers,
                mode,
            ) {
                self.enqueue_protocol(Bytes::copy_from_slice(&bytes));
            }
            cx.notify();
            return;
        }

        if button == TerminalMouseButton::Left {
            if self.selecting {
                self.state.update_selection(point, side);
                if self.config.copy_on_select {
                    let _ = self.copy_selection_to_clipboard(cx);
                }
            }
            self.selection_anchor = None;
            self.selecting = false;
            self.stop_selection_scroll();
        }
        cx.notify();
    }

    /// Handle mouse move events.
    ///
    /// Currently a placeholder for future mouse selection support.
    fn on_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.scrollbar_captured {
            let _ = self.update_scrollbar_drag(event.position, cx);
            return;
        }
        let was_hidden = self.pointer_hidden;
        self.pointer_hidden = false;
        let Some((point, side)) = self.point_and_side_for_position(event.position) else {
            if was_hidden {
                cx.notify();
            }
            return;
        };
        let hovered_link = self.hyperlink_at_point(point);
        let hyperlink_changed = self.hovered_link != hovered_link;
        self.hovered_link = hovered_link;
        let point_changed = self.pointer_point != Some(point);
        self.pointer_point = Some(point);
        self.pointer_side = side;

        let mode = self.state.mode();
        let modifiers = Self::terminal_modifiers(event.modifiers);
        if !modifiers.shift && self.selection_anchor.is_none() && Self::mouse_reporting(mode) {
            if point_changed
                && let Some(bytes) = encode_mouse(
                    TerminalMouseEvent::Motion {
                        held_button: self.held_mouse_button,
                        point,
                    },
                    modifiers,
                    mode,
                )
            {
                self.enqueue_protocol(Bytes::copy_from_slice(&bytes));
            }
            if point_changed || was_hidden || hyperlink_changed {
                cx.notify();
            }
            return;
        }

        let Some((anchor, anchor_side)) = self.selection_anchor else {
            if was_hidden || hyperlink_changed {
                cx.notify();
            }
            return;
        };
        if self.held_mouse_button != Some(TerminalMouseButton::Left) || !event.dragging() {
            return;
        }
        if !self.selecting {
            self.state
                .begin_selection(anchor, anchor_side, AlacSelectionType::Simple);
            self.selecting = true;
        }
        self.state.update_selection(point, side);
        self.update_selection_scroll(event.position, cx);
        cx.notify();
    }

    /// Handle scroll events.
    ///
    /// Currently a placeholder for future scrollback support.
    fn on_scroll(
        &mut self,
        event: &ScrollWheelEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.pointer_hidden = false;
        let Some(viewport) = *self.viewport.lock() else {
            return;
        };
        let Some(point) = self.point_for_position(event.position) else {
            return;
        };
        let pixel_delta = event.delta.pixel_delta(viewport.cell_height);
        let lines = pixels_to_scroll_lines(pixel_delta.y, viewport.cell_height);
        let columns = pixels_to_scroll_lines(pixel_delta.x, viewport.cell_width);
        if lines == 0 && columns == 0 {
            return;
        }

        let mode = self.state.mode();
        let modifiers = Self::terminal_modifiers(event.modifiers);
        if !modifiers.shift && Self::mouse_reporting(mode) {
            let mut reports = BytesMut::with_capacity(
                (lines.unsigned_abs() + columns.unsigned_abs()) as usize * 12,
            );
            let vertical = if lines > 0 {
                TerminalWheelDirection::Up
            } else {
                TerminalWheelDirection::Down
            };
            for _ in 0..lines.unsigned_abs() {
                if let Some(bytes) = encode_mouse(
                    TerminalMouseEvent::Wheel {
                        direction: vertical,
                        point,
                    },
                    modifiers,
                    mode,
                ) {
                    reports.extend_from_slice(&bytes);
                }
            }
            let horizontal = if columns > 0 {
                TerminalWheelDirection::Left
            } else {
                TerminalWheelDirection::Right
            };
            for _ in 0..columns.unsigned_abs() {
                if let Some(bytes) = encode_mouse(
                    TerminalMouseEvent::Wheel {
                        direction: horizontal,
                        point,
                    },
                    modifiers,
                    mode,
                ) {
                    reports.extend_from_slice(&bytes);
                }
            }
            if !reports.is_empty() {
                self.enqueue_protocol(reports.freeze());
            }
            cx.notify();
            return;
        }

        if mode.contains(TermMode::ALT_SCREEN) {
            if mode.contains(TermMode::ALTERNATE_SCROLL) && !modifiers.shift && lines != 0 {
                let sequence = if lines > 0 { b"\x1bOA" } else { b"\x1bOB" };
                let mut input =
                    BytesMut::with_capacity(lines.unsigned_abs() as usize * sequence.len());
                for _ in 0..lines.unsigned_abs() {
                    input.extend_from_slice(sequence);
                }
                self.enqueue_input(input.freeze(), cx);
            }
            return;
        }

        if lines != 0 {
            self.state.scroll_display(Scroll::Delta(lines));
            cx.notify();
        }
    }

    fn process_events(&mut self, cx: &mut Context<Self>) {
        let pending = self.event_mailbox.drain();
        for event in pending.events {
            match event {
                TerminalEvent::ClipboardStore(target, text) => {
                    if !self.last_focused
                        || self.config.osc52_policy == TerminalOsc52Policy::Disabled
                    {
                        self.report_io_error(
                            PtyIoOperation::Clipboard,
                            "terminal clipboard store denied by policy or focus state",
                            false,
                            cx,
                        );
                    } else if !Self::write_clipboard(target, text, cx) {
                        self.report_io_error(
                            PtyIoOperation::Clipboard,
                            "terminal clipboard target is unsupported",
                            false,
                            cx,
                        );
                    }
                }
                TerminalEvent::ClipboardStoreRejected => {
                    self.report_io_error(
                        PtyIoOperation::Clipboard,
                        "terminal clipboard store exceeded the 1 MiB limit",
                        false,
                        cx,
                    );
                }
                TerminalEvent::ClipboardLoad(target, formatter) => {
                    let allowed = self.last_focused
                        && self.config.osc52_policy == TerminalOsc52Policy::ReadWrite;
                    if !allowed {
                        self.report_io_error(
                            PtyIoOperation::Clipboard,
                            "terminal clipboard load denied by policy or focus state",
                            false,
                            cx,
                        );
                    }
                    let mut text = allowed
                        .then(|| Self::clipboard_text(target, cx))
                        .flatten()
                        .unwrap_or_default();
                    if text.len() > MAX_OSC52_BYTES {
                        text.clear();
                        self.report_io_error(
                            PtyIoOperation::Clipboard,
                            "terminal clipboard source exceeded the 1 MiB limit",
                            false,
                            cx,
                        );
                    }
                    self.enqueue_reply(Bytes::from(formatter(&text).into_bytes()), cx);
                }
                TerminalEvent::ColorRequest(index, formatter) => {
                    let dynamic_colors = self.state.dynamic_colors();
                    if let Some(color) = self.config.colors.query_rgb(index, &dynamic_colors) {
                        self.enqueue_reply(Bytes::from(formatter(color).into_bytes()), cx);
                    }
                }
                TerminalEvent::TextAreaSizeRequest(formatter) => {
                    let viewport = *self.viewport.lock();
                    let (rows, cols, cell_width, cell_height) = viewport.map_or_else(
                        || {
                            (
                                self.state.rows(),
                                self.state.cols(),
                                self.renderer.cell_width,
                                self.renderer.cell_height,
                            )
                        },
                        |viewport| {
                            (
                                viewport.rows,
                                viewport.cols,
                                viewport.cell_width,
                                viewport.cell_height,
                            )
                        },
                    );
                    let cell_width: f32 = cell_width.into();
                    let cell_height: f32 = cell_height.into();
                    let size = alacritty_terminal::event::WindowSize {
                        num_lines: rows.min(u16::MAX as usize) as u16,
                        num_cols: cols.min(u16::MAX as usize) as u16,
                        cell_width: cell_width.round().clamp(0.0, u16::MAX as f32) as u16,
                        cell_height: cell_height.round().clamp(0.0, u16::MAX as f32) as u16,
                    };
                    self.enqueue_reply(Bytes::from(formatter(size).into_bytes()), cx);
                }
                TerminalEvent::PtyWrite(data) => {
                    self.enqueue_reply(Bytes::from(data.into_bytes()), cx);
                }
                TerminalEvent::Pty(PtyEvent::Eof) => {
                    self.finish_exit(ExitReason::Completed, cx);
                }
                TerminalEvent::Pty(PtyEvent::IoError {
                    operation,
                    message,
                    fatal,
                }) => {
                    self.report_io_error(operation, &message, fatal, cx);
                    if fatal {
                        self.finish_exit(ExitReason::Failed, cx);
                    }
                }
                TerminalEvent::MouseCursorDirty => cx.notify(),
                TerminalEvent::CursorBlinkingChange => self.restart_cursor_blink(cx),
                TerminalEvent::Title(title) => {
                    if let Some(callback) = &self.title_callback {
                        callback(cx, &title);
                    }
                }
                TerminalEvent::ResetTitle => {
                    if let Some(callback) = &self.title_callback {
                        callback(cx, "");
                    }
                }
                TerminalEvent::Bell => {
                    unreachable!("mailbox drains bell events through its bounded bell counter")
                }
                TerminalEvent::Wakeup => {}
                TerminalEvent::Exit => self.finish_exit(ExitReason::Completed, cx),
                TerminalEvent::ChildExit(code) => {
                    self.finish_exit(Self::child_exit_reason(code), cx);
                }
            }
        }
        if let Some(callback) = &self.bell_callback {
            for _ in 0..pending.bells {
                callback(cx);
            }
        }
        if pending.redraw {
            cx.notify();
        }
    }

    fn child_exit_reason(code: i32) -> ExitReason {
        if code == 0 {
            ExitReason::Completed
        } else {
            ExitReason::Failed
        }
    }

    fn finish_exit(&mut self, reason: ExitReason, cx: &mut Context<Self>) {
        if self.exited {
            return;
        }
        self.exited = true;
        self.io_driver.shutdown();
        if let Some(callback) = &self.exit_callback {
            callback(cx, reason);
        }
    }

    /// Get the current terminal dimensions.
    ///
    /// # Returns
    ///
    /// A tuple of (columns, rows).
    pub fn dimensions(&self) -> (usize, usize) {
        (self.state.cols(), self.state.rows())
    }

    fn apply_viewport_size(
        &mut self,
        cols: usize,
        rows: usize,
        bounds: Bounds<Pixels>,
        padding: Edges<Pixels>,
        cell_width: Pixels,
        cell_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let cols = cols.clamp(1, u16::MAX as usize);
        let rows = rows.clamp(1, u16::MAX as usize);
        let dimensions_changed = self.dimensions() != (cols, rows);
        if dimensions_changed {
            let _ = self.io.resize(cols as u16, rows as u16);
            self.state.resize(cols, rows);
            self.render_cache.lock().clear();
            self.renderer.invalidate_palette();
        }
        *self.viewport.lock() = Some(TerminalViewport {
            bounds,
            padding,
            cell_width,
            cell_height,
            cols,
            rows,
            cursor_bounds: None,
        });
        cx.notify();
    }

    /// Resize the terminal to new dimensions.
    ///
    /// This method should be called when the terminal view size changes.
    /// It updates the internal grid and notifies the terminal process of the new size.
    ///
    /// # Arguments
    ///
    /// * `cols` - New number of columns
    /// * `rows` - New number of rows
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let cols = cols.clamp(1, u16::MAX as usize);
        let rows = rows.clamp(1, u16::MAX as usize);
        let _ = self.io.resize(cols as u16, rows as u16);
        self.state.resize(cols, rows);
        self.render_cache.lock().clear();
        self.renderer.invalidate_palette();
    }

    /// Get the current terminal configuration.
    ///
    /// # Returns
    ///
    /// A reference to the current configuration.
    pub fn config(&self) -> &TerminalConfig {
        &self.config
    }

    /// Get the focus handle for this terminal view.
    ///
    /// # Returns
    ///
    /// A reference to the focus handle.
    pub fn focus_handle(&self) -> &FocusHandle {
        &self.focus_handle
    }

    #[cfg(any(test, debug_assertions))]
    pub fn diagnostics_snapshot(&self) -> TerminalDiagnosticsSnapshot {
        let mut snapshot = self.renderer.diagnostics_snapshot();
        let io = self.io_driver.diagnostics();
        snapshot.bytes_read = io.bytes_read;
        snapshot.parser_batches = io.parser_batches;
        snapshot.read_batches_high_water = io.read_batches_high_water;
        snapshot.queued_input_high_water = io.queued_input_high_water;
        snapshot.queued_reply_high_water = io.queued_reply_high_water;
        snapshot.queued_command_high_water = io.queued_command_high_water;
        snapshot.gpui_wakeups = self.event_mailbox.gpui_wakeups();
        snapshot
    }

    #[cfg(any(test, debug_assertions))]
    pub fn reset_diagnostics(&self) {
        self.renderer.reset_diagnostics();
    }

    /// Update the terminal configuration.
    ///
    /// This method updates the terminal's configuration, including font settings,
    /// padding, and color palette. Changes take effect on the next render.
    ///
    /// # Arguments
    ///
    /// * `config` - The new configuration to apply
    /// * `cx` - The context for triggering a repaint
    pub fn update_config(&mut self, config: TerminalConfig, cx: &mut Context<Self>) {
        let config = config.normalized();
        let font_changed = config.font_family != self.config.font_family
            || config.font_size != self.config.font_size
            || config.line_height_multiplier != self.config.line_height_multiplier;
        let palette_changed = config.colors != self.config.colors;
        let core_changed = config.scrollback != self.config.scrollback
            || config.cursor_shape != self.config.cursor_shape
            || config.cursor_blinking != self.config.cursor_blinking
            || config.semantic_escape_chars != self.config.semantic_escape_chars
            || config.osc52_policy != self.config.osc52_policy
            || config.kitty_keyboard != self.config.kitty_keyboard;
        let hint_changed =
            config.hint_alphabet != self.config.hint_alphabet || config.hints != self.config.hints;
        let hide_mouse_changed =
            config.hide_mouse_when_typing != self.config.hide_mouse_when_typing;
        let interaction_changed = hint_changed
            || config.show_scrollbar != self.config.show_scrollbar
            || config.cursor_unfocused_hollow != self.config.cursor_unfocused_hollow
            || config.cursor_thickness != self.config.cursor_thickness
            || hide_mouse_changed
            || config.copy_on_select != self.config.copy_on_select;
        if core_changed {
            self.state.set_options(config.term_options());
        }

        self.renderer.font_family = config.font_family.clone();
        self.renderer.font_size = config.font_size;
        self.renderer.line_height_multiplier = config.line_height_multiplier;
        self.renderer.palette = config.colors.clone();
        self.renderer.cursor_thickness = config.cursor_thickness;
        if font_changed {
            self.renderer.invalidate_font();
            self.render_cache.lock().clear();
        } else if palette_changed {
            self.renderer.invalidate_palette();
            self.render_cache.lock().clear();
        }
        self.config = config;
        if interaction_changed {
            self.render_generation = self.render_generation.wrapping_add(1);
        }
        if hint_changed {
            self.hint_config_generation = self.hint_config_generation.wrapping_add(1);
            self.hint.typed.clear();
            self.hint.cache_key = None;
            self.refresh_hint_candidates();
        }
        if hide_mouse_changed {
            self.pointer_hidden = false;
        }
        cx.set_cursor_hide_mode(if self.last_focused && self.config.hide_mouse_when_typing {
            CursorHideMode::OnTypingAndAction
        } else {
            CursorHideMode::Never
        });
        self.restart_cursor_blink(cx);
        cx.notify();
    }

    /// Calculate terminal dimensions from pixel bounds and cell size.
    ///
    /// Helper method to determine how many columns and rows fit in the given bounds.
    #[allow(dead_code)]
    fn calculate_dimensions(&self, bounds: Bounds<Pixels>) -> (usize, usize) {
        let width_f32: f32 = bounds.size.width.into();
        let height_f32: f32 = bounds.size.height.into();
        let cell_width_f32: f32 = self.renderer.cell_width.into();
        let cell_height_f32: f32 = self.renderer.cell_height.into();

        let cols = ((width_f32 / cell_width_f32) as usize).max(1);
        let rows = ((height_f32 / cell_height_f32) as usize).max(1);
        (cols, rows)
    }
}

impl Drop for TerminalView {
    fn drop(&mut self) {
        self.io_driver.shutdown();
        self.cursor_blink_generation = self.cursor_blink_generation.wrapping_add(1);
        self.cursor_blink_task.take();
        self.selection_scroll_generation = self.selection_scroll_generation.wrapping_add(1);
        self.selection_scroll_task.take();
        self.pressed_keys.clear();
        self.ime_state.clear_marked_text();
    }
}

impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        range: std::ops::Range<usize>,
        adjusted_range: &mut Option<std::ops::Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        if let Some(marked_range) = self.ime_state.marked_text_range() {
            let range = range.start.min(marked_range.end)..range.end.min(marked_range.end);
            *adjusted_range = Some(range.clone());
            return self.ime_state.text_for_range(range);
        }
        if !self.search.active {
            return None;
        }
        let utf16 = self.search.query.encode_utf16().collect::<Vec<_>>();
        let range = range.start.min(utf16.len())..range.end.min(utf16.len());
        *adjusted_range = Some(range.clone());
        Some(String::from_utf16_lossy(&utf16[range]))
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        let caret = if self.search.active && !self.ime_state.is_active() {
            self.search.query.encode_utf16().count()
        } else {
            self.ime_state.selected_text_range().end
        };
        Some(UTF16Selection {
            range: caret..caret,
            reversed: false,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<std::ops::Range<usize>> {
        self.ime_state.marked_text_range()
    }

    fn unmark_text(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        self.ime_state.clear_marked_text();
        self.restart_cursor_blink(cx);
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        text: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let committed = self.ime_state.commit_text(text);
        if self.search.active {
            if let Some(text) = committed {
                let mut query = self.search.query.clone();
                query.push_str(&text);
                self.set_search_query(query, cx);
            } else {
                self.restart_cursor_blink(cx);
                cx.notify();
            }
        } else if self.hint.active {
            if let Some(text) = committed {
                for character in text.chars() {
                    self.handle_hint_text(&character.to_string(), cx);
                    if !self.hint.active {
                        break;
                    }
                }
            }
            self.restart_cursor_blink(cx);
            cx.notify();
        } else {
            let enqueued = committed.is_some_and(|text| {
                self.enqueue_input(Bytes::copy_from_slice(text.as_bytes()), cx)
            });
            if !enqueued {
                self.restart_cursor_blink(cx);
                cx.notify();
            }
        }
        window.invalidate_character_coordinates();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _range: Option<std::ops::Range<usize>>,
        new_text: &str,
        new_selected_range: Option<std::ops::Range<usize>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.ime_state.set_marked_text(new_text, new_selected_range);
        self.restart_cursor_blink(cx);
        window.invalidate_character_coordinates();
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        _range_utf16: std::ops::Range<usize>,
        _element_bounds: Bounds<Pixels>,
        window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        self.ime_cursor_bounds(self.ime_state.caret_utf16(), window)
    }

    fn character_index_for_point(
        &mut self,
        _point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        None
    }
}

impl Render for TerminalView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focused = self.focus_handle.is_focused(window);
        self.handle_focus_change(focused, cx);
        self.refresh_visible_search_matches();
        self.refresh_hint_candidates();

        let state_arc = self.state.term_arc();
        let renderer = self.renderer.clone();
        let render_cache = self.render_cache.clone();
        let event_mailbox = self.event_mailbox.clone();
        let padding = self.config.padding;
        let show_scrollbar = self.config.show_scrollbar;
        let cursor_unfocused_hollow = self.config.cursor_unfocused_hollow;
        let (background, foreground) = render_cache.lock().frame().map_or_else(
            || {
                (
                    self.config.colors.background(),
                    self.config.colors.foreground(),
                )
            },
            |frame| (frame.default_background, frame.default_foreground),
        );
        let render_overlays = self.render_overlays();
        let render_generation = self.render_generation;
        let hint_active = self.hint.active;
        let key_context = if self.search.active {
            "YtttTerminal YtttTerminalSearch"
        } else if self.hint.active {
            "YtttTerminal YtttTerminalHint"
        } else {
            TERMINAL_KEY_CONTEXT
        };
        let search_status = self.search.active.then(|| {
            self.search.error.as_ref().map_or_else(
                || format!("/{}", self.search.query),
                |error| format!("/{} — {error}", self.search.query),
            )
        });
        let viewport = self.viewport.clone();
        let viewport_for_layout = viewport.clone();
        let terminal = cx.entity();
        let terminal_for_layout = terminal.clone();
        let focus_handle = self.focus_handle.clone();
        let marked_text = self.ime_state.marked_text.clone();
        let cursor_visible = self.cursor_visible && marked_text.is_none();
        let scrollbar_gutter = px(6.0);
        let pointer_style = if self.hovered_link.is_some() {
            CursorStyle::PointingHand
        } else if Self::mouse_reporting(self.state.mode()) && self.selection_anchor.is_none() {
            CursorStyle::Arrow
        } else {
            CursorStyle::IBeam
        };

        div()
            .size_full()
            .bg(background)
            .cursor(pointer_style)
            .track_focus(&self.focus_handle)
            .key_context(key_context)
            .on_action(cx.listener(Self::on_send_tab))
            .on_action(cx.listener(Self::on_send_backtab))
            .on_action(cx.listener(Self::on_copy))
            .on_action(cx.listener(Self::on_paste))
            .on_action(cx.listener(Self::on_start_search))
            .on_action(cx.listener(Self::on_search_next))
            .on_action(cx.listener(Self::on_search_previous))
            .on_action(cx.listener(Self::on_search_history_previous))
            .on_action(cx.listener(Self::on_search_history_next))
            .on_action(cx.listener(Self::on_cancel_search))
            .on_action(cx.listener(Self::on_toggle_vi_mode))
            .on_action(cx.listener(Self::on_start_hint_mode))
            .on_key_down(cx.listener(Self::on_key_down))
            .on_key_up(cx.listener(Self::on_key_up))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::on_mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::on_mouse_down))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::on_mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::on_mouse_up))
            .on_mouse_move(cx.listener(Self::on_mouse_move))
            .on_scroll_wheel(cx.listener(Self::on_scroll))
            .child(
                canvas(
                    move |bounds, window, cx| {
                        let mut measured_renderer = renderer;
                        let metrics = measured_renderer.ensure_metrics(window);
                        let effective_padding = if show_scrollbar {
                            Edges {
                                right: padding.right.max(scrollbar_gutter),
                                ..padding
                            }
                        } else {
                            padding
                        };
                        let available_width: f32 =
                            (bounds.size.width - effective_padding.left - effective_padding.right)
                                .into();
                        let available_height: f32 =
                            (bounds.size.height - effective_padding.top - effective_padding.bottom)
                                .into();
                        let cell_width: f32 = metrics.cell_width.into();
                        let cell_height: f32 = metrics.cell_height.into();
                        let cols = ((available_width.max(0.0) / cell_width) as usize).max(1);
                        let rows = ((available_height.max(0.0) / cell_height) as usize).max(1);
                        let viewport_changed =
                            viewport_for_layout.lock().as_ref().is_none_or(|viewport| {
                                viewport.bounds != bounds
                                    || viewport.padding != effective_padding
                                    || viewport.cell_width != metrics.cell_width
                                    || viewport.cell_height != metrics.cell_height
                                    || viewport.cols != cols
                                    || viewport.rows != rows
                            });
                        if viewport_changed {
                            let terminal = terminal_for_layout.clone();
                            window.defer(cx, move |_window, cx| {
                                terminal.update(cx, |terminal, terminal_cx| {
                                    terminal.apply_viewport_size(
                                        cols,
                                        rows,
                                        bounds,
                                        effective_padding,
                                        metrics.cell_width,
                                        metrics.cell_height,
                                        terminal_cx,
                                    );
                                });
                            });
                        }
                        (measured_renderer, effective_padding)
                    },
                    move |bounds, (measured_renderer, effective_padding), window, cx| {
                        let lock_started = Instant::now();
                        let snapshot = {
                            let mut term = state_arc.lock();
                            let (selection, cursor_row, display_offset, screen_lines) = {
                                let content = term.renderable_content();
                                (
                                    content.selection,
                                    alacritty_terminal::term::point_to_viewport(
                                        content.display_offset,
                                        content.cursor.point,
                                    )
                                    .map(|point| point.line),
                                    content.display_offset,
                                    term.screen_lines(),
                                )
                            };
                            let forced_rows = render_cache.lock().overlay_damage_rows(
                                selection,
                                cursor_row,
                                display_offset,
                                screen_lines,
                                &render_overlays,
                            );
                            TerminalRenderSnapshot::build(
                                &mut term,
                                &measured_renderer.palette,
                                &render_overlays,
                                focused,
                                cursor_unfocused_hollow,
                                cursor_visible,
                                &forced_rows,
                                render_generation,
                            )
                        };
                        event_mailbox.clear_redraw();
                        measured_renderer.record_term_lock(
                            lock_started.elapsed().as_nanos().min(u64::MAX as u128) as u64,
                        );

                        let mut cache = render_cache.lock();
                        cache.merge(snapshot);
                        let snapshot = cache
                            .frame()
                            .expect("a merged terminal snapshot must produce a frame");
                        let cursor_bounds = measured_renderer.cursor_bounds(
                            bounds,
                            effective_padding,
                            snapshot.cursor,
                        );
                        if let Some(viewport) = viewport.lock().as_mut() {
                            viewport.cursor_bounds = cursor_bounds;
                        }

                        measured_renderer.paint(
                            bounds,
                            effective_padding,
                            show_scrollbar,
                            snapshot,
                            window,
                            cx,
                        );
                        if let (Some(cursor_bounds), Some(marked_text)) =
                            (cursor_bounds, marked_text.as_deref())
                        {
                            measured_renderer.paint_ime_text(
                                cursor_bounds,
                                snapshot,
                                marked_text,
                                window,
                                cx,
                            );
                        }
                        drop(cache);
                        if hint_active {
                            let terminal = terminal.clone();
                            window.defer(cx, move |_window, cx| {
                                terminal.update(cx, |terminal, terminal_cx| {
                                    let previous = terminal.hint.candidates.clone();
                                    terminal.hint.cache_key = None;
                                    terminal.refresh_hint_candidates();
                                    if terminal.hint.candidates != previous {
                                        terminal_cx.notify();
                                    }
                                });
                            });
                        }
                        window.handle_input(
                            &focus_handle,
                            ElementInputHandler::new(bounds, terminal.clone()),
                            cx,
                        );
                    },
                )
                .size_full(),
            )
            .when_some(search_status, |terminal, status| {
                terminal.child(
                    div()
                        .absolute()
                        .right_2()
                        .bottom_2()
                        .max_w(px(480.0))
                        .px_2()
                        .py_1()
                        .rounded_sm()
                        .bg(background.alpha(0.94))
                        .text_color(foreground)
                        .text_xs()
                        .child(status),
                )
            })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_TERMINAL_URL_REGEX, TerminalConfig, TerminalCursorShape, TerminalHintAction,
        TerminalHintCandidate, TerminalImeState, TerminalOsc52Policy, TerminalView,
        tab_key_down_event,
    };
    use crate::event::{MAX_OSC52_BYTES, MAX_TITLE_BYTES, TerminalEvent};
    use crate::test_support::{ReadStep, RecordingWriter, ScriptedReader};
    use alacritty_terminal::grid::Scroll;
    use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
    use alacritty_terminal::term::ClipboardType;
    use gpui::{
        ClipboardItem, EntityInputHandler, KeyDownEvent, KeyUpEvent, Keystroke, Modifiers,
        MouseButton, MouseDownEvent, MouseUpEvent, ScrollDelta, ScrollWheelEvent, TestAppContext,
        TouchPhase, point, px,
    };
    use std::io;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::time::{Duration, Instant};
    fn wait_for_bytes(recorded: &RecordingWriter, expected: &[u8]) {
        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.bytes() != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorded.bytes(), expected);
    }

    #[test]
    fn terminal_config_maps_complete_alacritty_options() {
        let config = TerminalConfig {
            scrollback: 42,
            cursor_shape: TerminalCursorShape::Beam,
            cursor_blinking: true,
            semantic_escape_chars: " x".to_string(),
            osc52_policy: TerminalOsc52Policy::ReadWrite,
            kitty_keyboard: true,
            ..TerminalConfig::default()
        };

        let options = config.term_options();
        assert_eq!(options.scrolling_history, 42);
        assert_eq!(
            options.default_cursor_style.shape,
            alacritty_terminal::vte::ansi::CursorShape::Beam
        );
        assert!(options.default_cursor_style.blinking);
        assert_eq!(options.semantic_escape_chars, " x");
        assert!(options.kitty_keyboard);
        assert_eq!(options.osc52, alacritty_terminal::term::Osc52::CopyPaste);
    }

    #[test]
    fn terminal_config_normalizes_hint_and_cursor_bounds() {
        let config = TerminalConfig {
            cursor_blink_interval_ms: 1,
            cursor_thickness: f32::NAN,
            hint_alphabet: "界".to_string(),
            hints: vec![],
            ..TerminalConfig::default()
        }
        .normalized();

        assert_eq!(config.cursor_blink_interval_ms, 10);
        assert_eq!(config.cursor_thickness, 0.15);
        assert_eq!(config.hint_alphabet, "jfkdls;ahgurieowpq");
        assert_eq!(
            config.hints[0].regex.as_deref(),
            Some(DEFAULT_TERMINAL_URL_REGEX)
        );
    }

    #[gpui::test]
    fn terminal_queries_format_replies(cx: &mut TestAppContext) {
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig::default(),
                cx,
            )
        });
        cx.run_until_parked();
        let viewport = cx.read(|cx| {
            (*terminal.read(cx).viewport.lock()).expect("terminal viewport must be painted")
        });
        let cell_width: f32 = viewport.cell_width.into();
        let cell_height: f32 = viewport.cell_height.into();
        let expected = format!(
            "color;size:{}:{}:{}:{};pty;",
            viewport.rows,
            viewport.cols,
            cell_width.round() as u16,
            cell_height.round() as u16,
        );

        terminal.update(cx, |terminal, cx| {
            terminal.event_mailbox.push(TerminalEvent::ColorRequest(
                alacritty_terminal::vte::ansi::NamedColor::Foreground as usize,
                Arc::new(|_| "color;".to_string()),
            ));
            terminal.event_mailbox.push(TerminalEvent::ColorRequest(
                usize::MAX,
                Arc::new(|_| "unsupported;".to_string()),
            ));
            terminal
                .event_mailbox
                .push(TerminalEvent::TextAreaSizeRequest(Arc::new(|size| {
                    format!(
                        "size:{}:{}:{}:{};",
                        size.num_lines, size.num_cols, size.cell_width, size.cell_height
                    )
                })));
            terminal
                .event_mailbox
                .push(TerminalEvent::PtyWrite("pty;".to_string()));
            terminal.process_events(cx);
        });
        wait_for_bytes(&recorded, expected.as_bytes());
    }

    #[test]
    fn child_exit_status_maps_to_terminal_exit_reason() {
        assert_eq!(
            TerminalView::child_exit_reason(0),
            crate::pty::ExitReason::Completed
        );
        assert_eq!(
            TerminalView::child_exit_reason(1),
            crate::pty::ExitReason::Failed
        );
    }

    #[gpui::test]
    fn terminal_events_dispatch_titles_bells_and_exit_once(cx: &mut TestAppContext) {
        let titles = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
        let titles_for_callback = titles.clone();
        let bells = Arc::new(AtomicUsize::new(0));
        let bells_for_callback = bells.clone();
        let exits = Arc::new(parking_lot::Mutex::new(Vec::new()));
        let exits_for_callback = exits.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                io::sink(),
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig::default(),
                cx,
            )
            .with_title_callback(move |_, title| {
                titles_for_callback.lock().push(title.to_string());
            })
            .with_bell_callback(move |_| {
                bells_for_callback.fetch_add(1, Ordering::Relaxed);
            })
            .with_exit_callback(move |_, reason| {
                exits_for_callback.lock().push(reason);
            })
        });

        terminal.update(cx, |terminal, cx| {
            terminal
                .event_mailbox
                .push(TerminalEvent::Title("界".repeat(MAX_TITLE_BYTES)));
            terminal.event_mailbox.push(TerminalEvent::Bell);
            terminal.event_mailbox.push(TerminalEvent::Bell);
            terminal.event_mailbox.push(TerminalEvent::MouseCursorDirty);
            terminal
                .event_mailbox
                .push(TerminalEvent::CursorBlinkingChange);
            terminal.event_mailbox.push(TerminalEvent::Wakeup);
            terminal.process_events(cx);
            terminal.event_mailbox.push(TerminalEvent::ResetTitle);
            terminal.event_mailbox.push(TerminalEvent::Exit);
            terminal.event_mailbox.push(TerminalEvent::ChildExit(7));
            terminal.process_events(cx);
        });

        let titles = titles.lock();
        assert_eq!(titles.len(), 2);
        assert!(titles[0].len() <= MAX_TITLE_BYTES);
        assert_eq!(titles[1], "");
        assert_eq!(bells.load(Ordering::Relaxed), 2);
        assert_eq!(
            exits.lock().as_slice(),
            [crate::pty::ExitReason::Completed],
            "the shared exit latch must ignore later child-exit events"
        );
    }

    #[gpui::test]
    fn osc52_policy_enforces_focus_and_target(cx: &mut TestAppContext) {
        let errors = Arc::new(parking_lot::Mutex::new(Vec::<String>::new()));
        let errors_for_callback = errors.clone();
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let config = TerminalConfig {
            osc52_policy: TerminalOsc52Policy::ReadWrite,
            ..TerminalConfig::default()
        };
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                config,
                cx,
            )
            .with_io_error_callback(move |_, operation, message, fatal| {
                assert_eq!(operation, crate::pty::PtyIoOperation::Clipboard);
                assert!(!fatal);
                errors_for_callback.lock().push(message.to_string());
            })
        });
        cx.write_to_clipboard(ClipboardItem::new_string("secret".to_string()));

        terminal.update(cx, |terminal, cx| {
            terminal.event_mailbox.push(TerminalEvent::ClipboardLoad(
                ClipboardType::Clipboard,
                Arc::new(|text| format!("off:{text};")),
            ));
            terminal.process_events(cx);
            terminal.last_focused = true;
            terminal.event_mailbox.push(TerminalEvent::ClipboardLoad(
                ClipboardType::Clipboard,
                Arc::new(|text| format!("on:{text};")),
            ));
            terminal.process_events(cx);
            terminal.event_mailbox.push(TerminalEvent::ClipboardStore(
                ClipboardType::Clipboard,
                "stored".to_string(),
            ));
            terminal.process_events(cx);
        });
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("stored".to_string())
        );

        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        terminal.update(cx, |terminal, cx| {
            terminal.event_mailbox.push(TerminalEvent::ClipboardLoad(
                ClipboardType::Selection,
                Arc::new(|text| format!("selection:{text};")),
            ));
            terminal.process_events(cx);
        });

        cx.write_to_clipboard(ClipboardItem::new_string("x".repeat(MAX_OSC52_BYTES + 1)));
        terminal.update(cx, |terminal, cx| {
            terminal.event_mailbox.push(TerminalEvent::ClipboardLoad(
                ClipboardType::Clipboard,
                Arc::new(|text| format!("oversize:{};", text.len())),
            ));
            terminal.process_events(cx);
            terminal.config.osc52_policy = TerminalOsc52Policy::Disabled;
            terminal.event_mailbox.push(TerminalEvent::ClipboardStore(
                ClipboardType::Clipboard,
                "denied".to_string(),
            ));
            terminal.process_events(cx);
        });

        #[cfg(not(any(target_os = "linux", target_os = "freebsd")))]
        let expected = b"off:;on:secret;selection:;oversize:0;";
        #[cfg(any(target_os = "linux", target_os = "freebsd"))]
        let expected = b"off:;on:secret;oversize:0;";
        wait_for_bytes(&recorded, expected);
        let errors = errors.lock();
        assert!(errors.iter().any(|error| error.contains("denied")));
        assert!(errors.iter().any(|error| error.contains("1 MiB")));
        assert!(errors.iter().all(|error| !error.contains("secret")));
    }

    #[gpui::test]
    fn terminal_copy_paste_actions_use_gpui_clipboard(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig::default(),
                cx,
            )
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(b"copy");
            terminal.state.set_simple_selection(
                AlacPoint::new(Line(0), Column(0)),
                AlacPoint::new(Line(0), Column(3)),
            );
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let focus = terminal.read(cx).focus_handle().clone();
            focus.focus(window, cx);
        });

        #[cfg(target_os = "macos")]
        cx.simulate_keystrokes("cmd-c");
        #[cfg(not(target_os = "macos"))]
        cx.simulate_keystrokes("ctrl-shift-c");
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("copy".to_string())
        );

        cx.write_to_clipboard(ClipboardItem::new_string("paste\ntext".to_string()));
        #[cfg(target_os = "macos")]
        cx.simulate_keystrokes("cmd-v");
        #[cfg(not(target_os = "macos"))]
        cx.simulate_keystrokes("ctrl-shift-v");
        wait_for_bytes(&recorded, b"paste\rtext");
    }

    #[test]
    fn ime_composition_waits_for_commit() {
        let mut ime_state = TerminalImeState::default();

        ime_state.set_marked_text("你😀", Some(1..3));
        assert_eq!(ime_state.marked_text_range(), Some(0..3));
        assert_eq!(ime_state.selected_text_range(), 1..3);
        assert_eq!(ime_state.caret_utf16(), 3);
        assert_eq!(ime_state.text_for_range(1..3), Some("😀".to_string()));

        assert_eq!(ime_state.commit_text("中文"), Some("中文"));
        assert_eq!(ime_state.marked_text_range(), None);
        assert_eq!(ime_state.selected_text_range(), 0..0);
        assert_eq!(ime_state.commit_text(""), None);
    }

    #[gpui::test]
    fn unchanged_repaint_rebuilds_no_rows_and_shapes_without_term_lock(cx: &mut TestAppContext) {
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(io::sink(), io::empty(), TerminalConfig::default(), cx)
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(b"A");
            cx.notify();
        });
        cx.run_until_parked();

        let term = cx.read(|cx| terminal.read(cx).state.term_arc());
        let shaped_without_lock = Arc::new(AtomicBool::new(false));
        terminal.update(cx, {
            let shaped_without_lock = shaped_without_lock.clone();
            move |terminal, cx| {
                terminal.renderer.invalidate_font();
                terminal.renderer.set_shaping_hook(Some(Arc::new(move || {
                    assert!(term.try_lock_unfair().is_some());
                    shaped_without_lock.store(true, Ordering::Relaxed);
                })));
                cx.notify();
            }
        });
        cx.run_until_parked();
        assert!(shaped_without_lock.load(Ordering::Relaxed));

        terminal.update(cx, |terminal, cx| {
            terminal.renderer.set_shaping_hook(None);
            terminal.reset_diagnostics();
            cx.notify();
        });
        cx.run_until_parked();
        let diagnostics = cx.read(|cx| terminal.read(cx).diagnostics_snapshot());
        assert_eq!(diagnostics.rebuilt_rows, 0);
        assert_eq!(diagnostics.shape_cache_misses, 0);
    }

    #[gpui::test]
    fn repaint_reuses_cached_cjk_clusters_without_reshaping(cx: &mut TestAppContext) {
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(io::sink(), io::empty(), TerminalConfig::default(), cx)
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes("中A中".as_bytes());
            cx.notify();
        });
        cx.run_until_parked();

        let shape_calls = Arc::new(AtomicUsize::new(0));
        terminal.update(cx, {
            let shape_calls = shape_calls.clone();
            move |terminal, cx| {
                terminal.renderer.invalidate_palette();
                terminal.reset_diagnostics();
                terminal.renderer.set_shaping_hook(Some(Arc::new(move || {
                    shape_calls.fetch_add(1, Ordering::Relaxed);
                })));
                cx.notify();
            }
        });
        cx.run_until_parked();

        let diagnostics = cx.read(|cx| terminal.read(cx).diagnostics_snapshot());
        assert_eq!(shape_calls.load(Ordering::Relaxed), 0);
        assert_eq!(diagnostics.shape_cache_misses, 0);
        assert!(diagnostics.shape_cache_hits >= 3);
        terminal.update(cx, |terminal, _| {
            terminal.renderer.set_shaping_hook(None);
        });
    }

    #[gpui::test]
    fn terminal_ime_preedit_blocks_pty_bytes(cx: &mut TestAppContext) {
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig::default(),
                cx,
            )
        });

        terminal.update_in(cx, |terminal, window, cx| {
            terminal.replace_and_mark_text_in_range(None, "中文", Some(2..2), window, cx);
            terminal.on_key_down(tab_key_down_event(false), window, cx);
        });
        assert!(recorded.bytes().is_empty());

        terminal.update_in(cx, |terminal, window, cx| {
            terminal.replace_text_in_range(None, "中文", window, cx);
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.bytes() != "中文".as_bytes() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorded.bytes(), "中文".as_bytes());

        terminal.update_in(cx, |terminal, window, cx| {
            terminal.replace_and_mark_text_in_range(None, "未提交", Some(3..3), window, cx);
            terminal.unmark_text(window, cx);
        });
        assert_eq!(recorded.bytes(), "中文".as_bytes());
    }

    #[gpui::test]
    fn kitty_release_requires_a_successful_tracked_press(cx: &mut TestAppContext) {
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig {
                    kitty_keyboard: true,
                    ..TerminalConfig::default()
                },
                cx,
            )
        });
        let keystroke = Keystroke::parse("a").unwrap();
        let press = KeyDownEvent {
            keystroke: keystroke.clone(),
            is_held: false,
            prefer_character_input: false,
        };
        let release = KeyUpEvent { keystroke };

        terminal.update_in(cx, |terminal, window, cx| {
            terminal.state.process_bytes(b"\x1b[>3u");
            assert!(
                terminal
                    .state
                    .mode()
                    .contains(alacritty_terminal::term::TermMode::REPORT_EVENT_TYPES)
            );
            terminal.on_key_down(&press, window, cx);
            terminal.on_key_up(&release, window, cx);
        });
        let expected = b"a\x1b[97;1:3u";
        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.bytes() != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorded.bytes(), expected);

        terminal.update_in(cx, |terminal, window, cx| {
            terminal.on_key_up(&release, window, cx);
        });
        std::thread::sleep(Duration::from_millis(10));
        assert_eq!(recorded.bytes(), expected);
    }

    #[gpui::test]
    fn focus_reporting_emits_once_per_transition(cx: &mut TestAppContext) {
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig::default(),
                cx,
            )
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(b"\x1b[?1004h");
            terminal.handle_focus_change(true, cx);
            terminal.handle_focus_change(true, cx);
            terminal.handle_focus_change(false, cx);
            terminal.handle_focus_change(false, cx);
        });

        let expected = b"\x1b[I\x1b[O";
        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.bytes() != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorded.bytes(), expected);
    }

    #[gpui::test]
    fn terminal_input_start_clears_selection_and_scrollback(cx: &mut TestAppContext) {
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig {
                    rows: 2,
                    hide_mouse_when_typing: true,
                    ..TerminalConfig::default()
                },
                cx,
            )
        });
        let press = KeyDownEvent {
            keystroke: Keystroke::parse("x").unwrap(),
            is_held: false,
            prefer_character_input: false,
        };
        terminal.update_in(cx, |terminal, window, cx| {
            let history = "line\r\n".repeat(2048);
            terminal.state.process_bytes(history.as_bytes());
            terminal.state.scroll_display(Scroll::Top);
            terminal.state.set_simple_selection(
                AlacPoint::new(Line(0), Column(0)),
                AlacPoint::new(Line(0), Column(1)),
            );
            assert!(terminal.state.display_offset() > 0);
            assert!(terminal.state.selection_to_string().is_some());
            terminal.on_key_down(&press, window, cx);
            assert_eq!(terminal.state.display_offset(), 0);
            assert!(terminal.state.selection_to_string().is_none());
            assert!(terminal.pointer_hidden);
        });

        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.bytes() != b"x" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorded.bytes(), b"x");
    }

    #[gpui::test]
    fn shift_bypasses_application_mouse_reporting_for_local_selection(cx: &mut TestAppContext) {
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig::default(),
                cx,
            )
        });
        terminal.update(cx, |terminal, cx| {
            terminal
                .state
                .process_bytes(b"\x1b[?1000h\x1b[?1006hPTY text");
            cx.notify();
        });
        cx.run_until_parked();

        let viewport = cx.read(|cx| {
            (*terminal.read(cx).viewport.lock()).expect("terminal viewport must be painted")
        });
        let start = point(
            viewport.bounds.origin.x + viewport.padding.left + viewport.cell_width * 0.25,
            viewport.bounds.origin.y + viewport.padding.top + viewport.cell_height * 0.5,
        );
        let end = point(
            viewport.bounds.origin.x + viewport.padding.left + viewport.cell_width * 2.75,
            viewport.bounds.origin.y + viewport.padding.top + viewport.cell_height * 0.5,
        );

        cx.simulate_click(start, Modifiers::none());
        let expected = b"\x1b[<0;1;1M\x1b[<0;1;1m";
        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.bytes() != expected && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorded.bytes(), expected);

        let shift = Modifiers {
            shift: true,
            ..Modifiers::none()
        };
        cx.simulate_mouse_down(start, MouseButton::Left, shift);
        cx.simulate_mouse_move(end, MouseButton::Left, shift);
        cx.simulate_mouse_up(end, MouseButton::Left, shift);
        assert_eq!(recorded.bytes(), expected);
        assert_eq!(
            cx.read(|cx| terminal.read(cx).state.selection_to_string()),
            Some("PTY".to_string())
        );
    }

    #[gpui::test]
    fn alternate_screen_scroll_requires_alternate_scroll_mode(cx: &mut TestAppContext) {
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig::default(),
                cx,
            )
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(b"\x1b[?1049h\x1b[?1007h");
            cx.notify();
        });
        cx.run_until_parked();
        let viewport = cx.read(|cx| {
            (*terminal.read(cx).viewport.lock()).expect("terminal viewport must be painted")
        });
        let position = point(
            viewport.bounds.origin.x + viewport.padding.left + viewport.cell_width * 0.5,
            viewport.bounds.origin.y + viewport.padding.top + viewport.cell_height * 0.5,
        );

        cx.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.0), viewport.cell_height)),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        let deadline = Instant::now() + Duration::from_secs(1);
        while recorded.bytes() != b"\x1bOA" && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(1));
        }
        assert_eq!(recorded.bytes(), b"\x1bOA");

        terminal.update(cx, |terminal, _| {
            terminal.state.process_bytes(b"\x1b[?1007l");
        });
        cx.simulate_event(ScrollWheelEvent {
            position,
            delta: ScrollDelta::Pixels(point(px(0.0), viewport.cell_height)),
            modifiers: Modifiers::none(),
            touch_phase: TouchPhase::Moved,
        });
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            recorded.bytes(),
            b"\x1bOA",
            "alternate-screen wheel input without alternate-scroll must be unhandled"
        );
    }

    #[gpui::test]
    fn plain_click_does_not_select_text_and_drag_still_selects(cx: &mut TestAppContext) {
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(io::sink(), io::empty(), TerminalConfig::default(), cx)
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(b"PTY text");
            terminal.state.set_simple_selection(
                AlacPoint::new(Line(0), Column(0)),
                AlacPoint::new(Line(0), Column(2)),
            );
            cx.notify();
        });
        cx.run_until_parked();

        let viewport = cx.read(|cx| {
            (*terminal.read(cx).viewport.lock()).expect("terminal viewport must be painted")
        });
        let start = point(
            viewport.bounds.origin.x + viewport.padding.left + viewport.cell_width * 0.5,
            viewport.bounds.origin.y + viewport.padding.top + viewport.cell_height * 0.5,
        );
        let end = point(
            viewport.bounds.origin.x + viewport.padding.left + viewport.cell_width * 2.5,
            viewport.bounds.origin.y + viewport.padding.top + viewport.cell_height * 0.5,
        );

        cx.simulate_click(start, Modifiers::none());
        assert_eq!(
            cx.read(|cx| terminal.read(cx).state.selection_to_string()),
            None,
            "a plain click must clear selection without selecting the clicked cell"
        );

        cx.simulate_mouse_down(start, MouseButton::Left, Modifiers::none());
        assert_eq!(
            cx.read(|cx| terminal.read(cx).state.selection_to_string()),
            None,
            "selection must remain pending until the pointer reaches another cell"
        );
        cx.simulate_mouse_move(end, MouseButton::Left, Modifiers::none());
        cx.simulate_mouse_up(end, MouseButton::Left, Modifiers::none());
        assert_eq!(
            cx.read(|cx| terminal.read(cx).state.selection_to_string()),
            Some("PTY".to_string())
        );
    }

    #[gpui::test]
    fn double_click_still_selects_a_word(cx: &mut TestAppContext) {
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(io::sink(), io::empty(), TerminalConfig::default(), cx)
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(b"PTY text");
            cx.notify();
        });
        cx.run_until_parked();

        let viewport = cx.read(|cx| {
            (*terminal.read(cx).viewport.lock()).expect("terminal viewport must be painted")
        });
        let position = point(
            viewport.bounds.origin.x + viewport.padding.left + viewport.cell_width * 0.5,
            viewport.bounds.origin.y + viewport.padding.top + viewport.cell_height * 0.5,
        );
        let modifiers = Modifiers::none();
        cx.simulate_event(MouseDownEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: 2,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position,
            modifiers,
            button: MouseButton::Left,
            click_count: 2,
        });

        assert_eq!(
            cx.read(|cx| terminal.read(cx).state.selection_to_string()),
            Some("PTY".to_string())
        );
    }

    #[gpui::test]
    fn search_highlight_wraps_once_and_invalid_query_clears_highlights(cx: &mut TestAppContext) {
        let (terminal, _cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(io::sink(), io::empty(), TerminalConfig::default(), cx)
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(b"alpha\r\nbeta alpha");
            terminal.search.active = true;
            terminal.set_search_query("alpha".to_string(), cx);
        });

        let visible = cx.read(|cx| terminal.read(cx).search.visible_matches.clone());
        assert_eq!(visible.len(), 2);
        terminal.update(cx, |terminal, cx| {
            terminal.navigate_search(alacritty_terminal::index::Direction::Right, cx);
        });
        let first = cx.read(|cx| terminal.read(cx).search.focused_match.clone());

        terminal.update(cx, |terminal, cx| {
            terminal.navigate_search(alacritty_terminal::index::Direction::Right, cx);
        });
        let second = cx.read(|cx| terminal.read(cx).search.focused_match.clone());
        terminal.update(cx, |terminal, cx| {
            terminal.navigate_search(alacritty_terminal::index::Direction::Right, cx);
        });
        let wrapped = cx.read(|cx| terminal.read(cx).search.focused_match.clone());
        assert_ne!(first, second);
        assert_eq!(wrapped, first);

        terminal.update(cx, |terminal, cx| {
            terminal.set_search_query("[".to_string(), cx);
        });
        cx.read(|cx| {
            let search = &terminal.read(cx).search;
            assert_eq!(search.query, "[");
            assert!(search.error.is_some());
            assert!(search.visible_matches.is_empty());
        });
    }

    #[test]
    fn hint_mode_labels_match_alacritty_split_alphabet_sequence() {
        assert_eq!(
            TerminalView::hint_labels(10, "0123"),
            vec!["0", "1", "20", "21", "30", "31", "220", "221", "230", "231"]
        );
    }

    #[gpui::test]
    fn hint_mode_discovers_one_hyperlink_candidate_and_labels_it(cx: &mut TestAppContext) {
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(io::sink(), io::empty(), TerminalConfig::default(), cx)
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(
                b"\x1b]8;;https://example.com\x1b\\https://example.com\x1b]8;;\x1b\\",
            );
            cx.notify();
        });
        cx.run_until_parked();
        terminal.update(cx, |terminal, _| {
            let links = terminal.visible_hyperlink_spans();
            assert_eq!(links.len(), 1);
            assert_eq!(links[0].1, "https://example.com");
            terminal.hint.active = true;
            terminal.refresh_hint_candidates();
        });
        cx.read(|cx| {
            let hint = &terminal.read(cx).hint;
            assert_eq!(hint.candidates.len(), 1);
            assert_eq!(hint.candidates[0].target, "https://example.com");
            assert_eq!(hint.candidates[0].label, "j");
        });
    }

    #[gpui::test]
    fn terminal_search_and_hint_modes_isolate_input(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(
                writer,
                ScriptedReader::new([ReadStep::Sleep(Duration::from_secs(2)), ReadStep::Eof]),
                TerminalConfig::default(),
                cx,
            )
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let focus = terminal.read(cx).focus_handle().clone();
            focus.focus(window, cx);
        });

        terminal.update(cx, |terminal, cx| {
            terminal.search.active = true;
            terminal.set_search_query(String::new(), cx);
        });
        cx.simulate_keystrokes("x");
        assert_eq!(cx.read(|cx| terminal.read(cx).search.query.clone()), "x");
        assert!(recorded.bytes().is_empty());

        terminal.update(cx, |terminal, _cx| {
            terminal.search.active = false;
            terminal.hint.active = true;
            terminal.hint.candidates = vec![TerminalHintCandidate {
                range: AlacPoint::new(Line(0), Column(0))..=AlacPoint::new(Line(0), Column(0)),
                target: "https://example.com".to_string(),
                label: "j".to_string(),
                action: TerminalHintAction::Copy,
            }];
        });
        cx.simulate_keystrokes("j");
        assert_eq!(
            cx.read_from_clipboard().and_then(|item| item.text()),
            Some("https://example.com".to_string())
        );
        assert!(recorded.bytes().is_empty());
    }

    #[gpui::test]
    fn vi_mode_motion_moves_cursor_without_writing_to_pty(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let writer = RecordingWriter::default();
        let recorded = writer.clone();
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(writer, io::empty(), TerminalConfig::default(), cx)
        });
        terminal.update(cx, |terminal, cx| {
            terminal.state.process_bytes(b"abc");
            terminal.state.with_term_mut(|term| term.toggle_vi_mode());
            cx.notify();
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let focus = terminal.read(cx).focus_handle().clone();
            focus.focus(window, cx);
        });
        let before = cx.read(|cx| {
            terminal
                .read(cx)
                .state
                .with_term(|term| term.vi_mode_cursor.point)
        });
        cx.simulate_keystrokes("h");
        let after = cx.read(|cx| {
            terminal
                .read(cx)
                .state
                .with_term(|term| term.vi_mode_cursor.point)
        });
        assert_ne!(after, before);
        assert!(recorded.bytes().is_empty());
    }

    #[gpui::test]
    fn scrollbar_track_click_pages_scrollback(cx: &mut TestAppContext) {
        let (terminal, cx) = cx.add_window_view(|_, cx| {
            TerminalView::new(io::sink(), io::empty(), TerminalConfig::default(), cx)
        });
        terminal.update(cx, |terminal, cx| {
            for line in 0..3000 {
                terminal
                    .state
                    .process_bytes(format!("line-{line}\r\n").as_bytes());
            }
            cx.notify();
        });
        cx.run_until_parked();
        assert!(
            cx.read(|cx| terminal.read(cx).state.scrollbar_metrics().is_some()),
            "the fixture must have scrollback before exercising its scrollbar"
        );
        assert_eq!(cx.read(|cx| terminal.read(cx).state.display_offset()), 0);
        let (x, y) = cx.read(|cx| {
            let terminal = terminal.read(cx);
            let viewport = terminal
                .viewport
                .lock()
                .expect("terminal viewport must be painted");
            (
                viewport.bounds.origin.x + viewport.bounds.size.width - px(2.0),
                viewport.bounds.origin.y + viewport.padding.top + px(1.0),
            )
        });
        cx.simulate_click(point(x, y), Modifiers::none());
        assert!(
            cx.read(|cx| terminal.read(cx).state.display_offset()) > 0,
            "a track click above the bottom-positioned thumb must page up"
        );
        cx.read(|cx| {
            let terminal = terminal.read(cx);
            assert!(!terminal.scrollbar_captured);
            assert!(terminal.scrollbar_drag.is_none());
        });
    }

    #[test]
    fn terminal_view_implements_gpui_text_input() {
        fn assert_input_handler<T: EntityInputHandler>() {}
        assert_input_handler::<TerminalView>();
    }
}
