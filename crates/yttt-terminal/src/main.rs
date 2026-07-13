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
}

impl TerminalApp {
    fn new(terminal: Entity<TerminalView>, session: PortablePtySession) -> Self {
        Self {
            terminal,
            session: Some(session),
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
        let mut session =
            spawn_portable_pty_session(TerminalSpawnRequest::for_shell("shell", &shell, ""))
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

            cx.open_window(
                gpui::WindowOptions {
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
                                cx.quit();
                            })
                    });

                    // Focus the terminal so it receives key events
                    let focus_handle = terminal.read(cx).focus_handle().clone();
                    focus_handle.focus(window, cx);

                    // Wrap in TerminalApp to handle font size shortcuts
                    cx.new(|_cx| TerminalApp::new(terminal, session))
                },
            )?;

            Ok::<_, anyhow::Error>(())
        })
        .detach();
    });

    Ok(())
}
