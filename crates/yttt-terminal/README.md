# yttt-terminal

A terminal emulator component for [GPUI](https://gpui.rs) applications. It delegates VT parsing, terminal state, selection, search, and protocol modes to [alacritty_terminal](https://docs.rs/alacritty_terminal), while retaining GPUI rendering and generic `Read`/`Write` integration.

<img width="1763" alt="Image" src="https://github.com/user-attachments/assets/713ed2b3-e08d-4ff5-8aeb-2d02660ecf7d" />

## Architecture

- A fixed-buffer reader, parser coordinator, and ordered writer/resize worker form a bounded three-worker PTY driver. PTY output never passes through an unbounded UI queue, and keyboard input is never written synchronously from a GPUI handler.
- Terminal semantics are resolved under the Alacritty `Term` lock into immutable render snapshots. GPUI shaping and painting happen after the lock is released.
- Alacritty damage plus selection, cursor, IME, search, hint, and hyperlink-overlay damage updates an authoritative visible-row cache. Contiguous same-style cells of equal terminal width are shaped during canvas prepaint; double-width runs retain explicit two-column glyph offsets, and paint submits one layer per run instead of per cell.
- Alacritty events pass through a bounded, coalescing mailbox. Wakeups, titles, bells, clipboard requests, query replies, cursor changes, I/O failures, and process exit keep their required ordering.

## Features

- ANSI, 256-color, true-color, dynamic-color, cursor, title, and terminal-query support
- Legacy and Kitty keyboard protocols, application cursor/keypad modes, key press/repeat/release events, associated text, and pure IME commits
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

`spawn_portable_pty_session` applies `configure_terminal_environment`, owns the child/master lifecycle, provides a race-safe resize handle, and reaps the child through `PortablePtySession::finish`. On Unix, a child with no inherited `LC_ALL`, `LC_CTYPE`, or `LANG` receives `LANG=C.UTF-8`; any explicit locale is preserved.

For a custom reader, dropping `TerminalView` cannot force an arbitrary blocking `Read` implementation to return. The embedder must provide cancellation that closes or otherwise unblocks the reader. A reader that returns after shutdown is discarded without touching terminal or GPUI state.

## Configuration defaults

`TerminalConfig::default()` uses an 80 × 24 grid, 10,000 scrollback lines, a block non-blinking cursor, a 750 ms blink interval, a 5 s blink timeout, a hollow unfocused cursor, copy-only OSC 52 access, disabled Kitty negotiation, a visible scrollbar, Alacritty semantic escape characters, and URL hints. The host application may persist and update these values at runtime.

`with_resize_callback` returns `Result<(), String>`. Resize failures are reported through `with_io_error_callback` but the local grid still updates. Fatal read/write/mailbox failures report the I/O error first and then invoke the exit callback exactly once with `ExitReason::Failed`.

See `src/main.rs` for the standalone portable-PTY example.

## Reproducible performance benchmark

From the repository root, run the same deterministic Unicode-heavy TUI workload in
`yttt-terminal`, Kitty, and Alacritty:

```sh
scripts/run-terminal-perf.sh \
  --terminal all \
  --trace metal \
  --scenario full \
  --duration 20 \
  --warmup 3 \
  --fps 60 \
  --rows 40 \
  --columns 120
```

`--scenario full` rewrites the viewport, `damage` changes selected rows, and
`scroll` appends rows. Use `--trace time` for Time Profiler, `--trace both` for
sequential Time Profiler and Metal System Trace captures, or `--trace none` for
a fast in-process run. Instruments capture requires macOS and `xcrun`; Kitty and
Alacritty are skipped with an explicit message when they are not installed.

The runner builds the release binary with `perf-metrics`, waits for Instruments
to finish attaching before opening the workload gate, excludes warmup from
yttt's counters, and injects repeated English and Chinese-IME samples.

For a fair cross-terminal CPU trace without yttt's histogram/correlation probes
or synthetic input, add `--external-only`. That mode builds the normal release
binary and intentionally omits yttt's `metrics.json`; workload and Instruments
outputs remain directly comparable:

```sh
scripts/run-terminal-perf.sh --terminal all --trace time --external-only
```

Instrumented run directories contain:

- `workload.json`: achieved generator rate, throughput, deadline misses, and
  echoed input totals.
- `metrics.json`: PTY queue pressure, parser/lock/prepaint/paint distributions,
  frame cadence, input-to-write/echo/paint latency, and IME-preedit latency.
- `*.trace` plus exported XML: application CPU stacks, Metal present requests,
  and compositor surface-display cadence.
- `summary.json`: the normalized machine-readable comparison printed by
  `scripts/summarize-terminal-perf.py`.

Treat generator FPS as the offered load, present-request cadence as the
application's drawable submission rate, and displayed-surface cadence as the
compositor observation. The runner reports these separately; the in-process
paint callback interval is not mislabeled as GPU/display presentation.

## License

MIT OR Apache-2.0
