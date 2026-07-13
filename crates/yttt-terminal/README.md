# yttt-terminal

A terminal emulator component for [GPUI](https://gpui.rs) applications. It delegates VT parsing, terminal state, selection, search, and protocol modes to [alacritty_terminal](https://docs.rs/alacritty_terminal), while retaining GPUI rendering and generic `Read`/`Write` integration.

<img width="1763" alt="Image" src="https://github.com/user-attachments/assets/713ed2b3-e08d-4ff5-8aeb-2d02660ecf7d" />

## Architecture

- A fixed-buffer reader, parser coordinator, and ordered writer/resize worker form a bounded three-worker PTY driver. PTY output never passes through an unbounded UI queue, and keyboard input is never written synchronously from a GPUI handler.
- Terminal semantics are resolved under the Alacritty `Term` lock into immutable render snapshots. GPUI shaping and painting happen after the lock is released.
- Alacritty damage plus selection, cursor, IME, search, hint, and hyperlink-overlay damage updates an authoritative visible-row cache. Shaped glyph clusters use a bounded LRU cache.
- Alacritty events pass through a bounded, coalescing mailbox. Wakeups, titles, bells, clipboard requests, query replies, cursor changes, I/O failures, and process exit keep their required ordering.

## Features

- ANSI, 256-color, true-color, dynamic-color, cursor, title, and terminal-query support
- Legacy and Kitty keyboard protocols, application cursor/keypad modes, key press/repeat/release events
- X10, UTF-8, SGR, and SGR-pixel mouse reporting; local selection and scrollback interaction
- Bracketed paste filtering, OSC 52 policy enforcement, and GPUI clipboard/primary-selection integration
- Search, URL/hyperlink hints, Vi navigation/selection, scrollbar track paging and dragging
- Wide cells, combining characters, all Alacritty underline styles, cursor blinking, and IME preedit
- Runtime updates for fonts, colors, core terminal options, and interaction settings

## Portable PTY usage

Register bindings once during application initialization, spawn a managed session, retain the session for child reaping, and pass its I/O handles to `TerminalView`:

```rust
use yttt_terminal::{
    ExitReason, TerminalConfig, TerminalSpawnRequest, TerminalView,
    spawn_portable_pty_session,
};

yttt_terminal::init(cx);

let mut session = spawn_portable_pty_session(
    TerminalSpawnRequest::for_shell("shell", "/bin/zsh", "")
        .cwd(project_directory),
)?;
let io = session.take_io().expect("PTY I/O can only be taken once");
let resize = session.resize_handle();

let terminal = cx.new(|cx| {
    TerminalView::new(io.writer, io.reader, TerminalConfig::default(), cx)
        .with_resize_callback(move |cols, rows| {
            resize
                .resize(cols as usize, rows as usize)
                .map_err(|error| error.to_string())
        })
        .with_title_callback(|_cx, title| {
            println!("terminal title: {title}");
        })
        .with_io_error_callback(|_cx, operation, message, fatal| {
            eprintln!("terminal {operation:?} error (fatal={fatal}): {message}");
        })
        .with_exit_callback(|_cx, reason| {
            assert!(matches!(
                reason,
                ExitReason::Completed | ExitReason::Failed | ExitReason::KilledByUser
            ));
        })
});
```

`spawn_portable_pty_session` applies `configure_terminal_environment`, owns the child/master lifecycle, provides a race-safe resize handle, and reaps the child through `PortablePtySession::finish`.

For a custom reader, dropping `TerminalView` cannot force an arbitrary blocking `Read` implementation to return. The embedder must provide cancellation that closes or otherwise unblocks the reader. A reader that returns after shutdown is discarded without touching terminal or GPUI state.

## Configuration defaults

`TerminalConfig::default()` uses an 80 × 24 grid, 10,000 scrollback lines, a block non-blinking cursor, a 750 ms blink interval, a 5 s blink timeout, a hollow unfocused cursor, copy-only OSC 52 access, disabled Kitty negotiation, a visible scrollbar, Alacritty semantic escape characters, and URL hints. The host application may persist and update these values at runtime.

`with_resize_callback` returns `Result<(), String>`. Resize failures are reported through `with_io_error_callback` but the local grid still updates. Fatal read/write/mailbox failures report the I/O error first and then invoke the exit callback exactly once with `ExitReason::Failed`.

See `src/main.rs` for the standalone portable-PTY example.
## License

MIT OR Apache-2.0
