//! Minimal terminal emulator application using yttt-terminal library.
//!
//! This example demonstrates how to embed a terminal in a GPUI application
//! using portable-pty for proper PTY support.

use anyhow::Result;
use gpui::{
    AppContext, Context, Edges, Entity, InteractiveElement, IntoElement, KeyDownEvent,
    ParentElement, Render, Styled, Window, div, px,
};
use yttt_terminal::{
    ColorPalette, ExitReason, PortablePtySession, TerminalConfig, TerminalSpawnRequest,
    TerminalView, spawn_portable_pty_session,
};

/// Wrapper view that holds the terminal and handles font size shortcuts.
struct TerminalApp {
    terminal: Entity<TerminalView>,
    session: Option<PortablePtySession>,
    #[cfg(feature = "perf-metrics")]
    _performance_reporter: Option<yttt_terminal::TerminalPerformanceReporter>,
}

impl TerminalApp {
    #[cfg(not(feature = "perf-metrics"))]

    fn new(terminal: Entity<TerminalView>, session: PortablePtySession) -> Self {
        Self {
            terminal,
            session: Some(session),
        }
    }
    #[cfg(feature = "perf-metrics")]
    fn new(
        terminal: Entity<TerminalView>,
        session: PortablePtySession,
        performance_reporter: Option<yttt_terminal::TerminalPerformanceReporter>,
    ) -> Self {
        Self {
            terminal,
            session: Some(session),
            _performance_reporter: performance_reporter,
        }
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        let keystroke = &event.keystroke;

        // Check for Ctrl++ or Ctrl+= (increase font size)
        if keystroke.modifiers.control && (keystroke.key == "+" || keystroke.key == "=") {
            self.terminal.update(cx, |terminal, cx| {
                let mut config = terminal.config().clone();
                config.font_size += px(1.0);
                terminal.update_config(config, cx);
            });
            cx.stop_propagation();
        } else if keystroke.modifiers.control && keystroke.key == "-" {
            // Check for Ctrl+- (decrease font size)
            self.terminal.update(cx, |terminal, cx| {
                let mut config = terminal.config().clone();
                // Don't go below 6px font size
                if config.font_size > px(6.0) {
                    config.font_size -= px(1.0);
                    terminal.update_config(config, cx);
                }
            });
            cx.stop_propagation();
        }
    }
}

impl Drop for TerminalApp {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            let _ = std::thread::Builder::new()
                .name("yttt-terminal-example-reaper".to_string())
                .spawn(move || {
                    let _ = session.finish(ExitReason::KilledByUser);
                });
        }
    }
}

impl Render for TerminalApp {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .on_key_down(cx.listener(Self::on_key_down))
            .child(self.terminal.clone())
    }
}

fn main() -> Result<()> {
    let app = gpui_platform::application();
    app.run(move |cx| {
        yttt_terminal::init(cx);
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
        let start_command = std::env::var("YTTT_TERMINAL_START_COMMAND").unwrap_or_default();
        let mut session = spawn_portable_pty_session(TerminalSpawnRequest::for_shell(
            "shell",
            &shell,
            start_command,
        ))
        .expect("Failed to spawn portable PTY session");
        let io = session.take_io().expect("PTY I/O can only be taken once");
        let resize_handle = session.resize_handle();

        // Spawn window creation on the main thread.
        cx.spawn(async move |cx| {
            let colors = ColorPalette::builder()
                .background(0x16, 0x16, 0x17)
                .foreground(0xC9, 0xC7, 0xCD)
                .cursor(0xC9, 0xC7, 0xCD)
                // Normal colors
                .black(0x10, 0x10, 0x10)
                .red(0xEF, 0xA6, 0xA2)
                .green(0x80, 0xC9, 0x90)
                .yellow(0xA6, 0x94, 0x60)
                .blue(0xA3, 0xB8, 0xEF)
                .magenta(0xE6, 0xA3, 0xDC)
                .cyan(0x50, 0xCA, 0xCD)
                .white(0x80, 0x80, 0x80)
                // Bright colors
                .bright_black(0x39, 0x41, 0x4E)
                .bright_red(0xE0, 0xAF, 0x85)
                .bright_green(0x5A, 0xCC, 0xAF)
                .bright_yellow(0xC8, 0xC8, 0x74)
                .bright_blue(0xCC, 0xAC, 0xED)
                .bright_magenta(0xF2, 0xA1, 0xC2)
                .bright_cyan(0x74, 0xC3, 0xE4)
                .bright_white(0xC0, 0xC0, 0xC0)
                .build();

            let config = TerminalConfig {
                font_family: "Mononoki Nerd Font".into(),
                font_size: px(14.0),
                cols: 80,
                rows: 24,
                scrollback: 10000,
                line_height_multiplier: 1.05,
                padding: Edges::all(px(8.0)),
                show_scrollbar: true,
                colors,
                ..TerminalConfig::default()
            };

            let resize_callback = move |cols: u16, rows: u16| {
                resize_handle
                    .resize(cols as usize, rows as usize)
                    .map_err(|error| error.to_string())
            };

            let benchmark_mode = std::env::var_os("YTTT_TERMINAL_PERF_OUTPUT").is_some()
                || std::env::var_os("YTTT_TERMINAL_BENCHMARK_MODE").is_some();

            let benchmark_window_bounds = benchmark_mode.then(|| {
                gpui::WindowBounds::Windowed(gpui::Bounds::new(
                    gpui::point(px(100.0), px(100.0)),
                    gpui::size(px(1024.0), px(768.0)),
                ))
            });

            cx.open_window(
                gpui::WindowOptions {
                    window_bounds: benchmark_window_bounds,
                    kind: if benchmark_mode {
                        gpui::WindowKind::PopUp
                    } else {
                        gpui::WindowKind::Normal
                    },
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("yttt-terminal".into()),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                |window, cx| {
                    // Create the terminal view
                    let terminal = cx.new(|cx| {
                        TerminalView::new(io.writer, io.reader, config, cx)
                            .with_resize_callback(resize_callback)
                            .with_exit_callback(|cx, _reason| {
                                if std::env::var_os("YTTT_TERMINAL_KEEP_OPEN_AFTER_EXIT").is_none()
                                {
                                    cx.quit();
                                }
                            })
                    });

                    // Focus the terminal so it receives key events
                    let focus_handle = terminal.read(cx).focus_handle().clone();
                    focus_handle.focus(window, cx);
                    cx.activate(true);
                    window.activate_window();
                    #[cfg(feature = "perf-metrics")]
                    let performance_start_file = std::env::var_os("YTTT_TERMINAL_PERF_START_FILE")
                        .map(std::path::PathBuf::from);

                    #[cfg(feature = "perf-metrics")]
                    if let Some(delay_ms) = std::env::var("YTTT_TERMINAL_PERF_INPUT_DELAY_MS")
                        .ok()
                        .and_then(|value| value.parse::<u64>().ok())
                    {
                        terminal.update(cx, |terminal, cx| {
                            terminal.start_performance_input_probe(
                                std::time::Duration::from_millis(delay_ms),
                                performance_start_file,
                                window,
                                cx,
                            );
                        });
                    }

                    // Wrap in TerminalApp to handle font size shortcuts.
                    #[cfg(feature = "perf-metrics")]
                    let terminal_app = {
                        let performance_reporter = terminal
                            .read(cx)
                            .performance_handle()
                            .spawn_reporter_from_env()
                            .unwrap_or_else(|error| {
                                eprintln!("failed to start terminal performance reporter: {error}");
                                None
                            });
                        TerminalApp::new(terminal, session, performance_reporter)
                    };
                    #[cfg(not(feature = "perf-metrics"))]
                    let terminal_app = TerminalApp::new(terminal, session);
                    cx.new(|_cx| terminal_app)
                },
            )
            .inspect_err(|error| eprintln!("failed to open terminal window: {error}"))?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });

    Ok(())
}
