use gpui::{Context, Edges, Entity, IntoElement, Render, Window, div, prelude::*, px, rgb};
use gpui_terminal::{ColorPalette, TerminalConfig, TerminalView};

use crate::{
    model::layout::PaneConfig,
    runtime::terminal::{PortablePtySession, spawn_portable_pty_session},
};

pub struct TerminalPaneView {
    pane_id: String,
    title: String,
    command: String,
    terminal: Option<Entity<TerminalView>>,
    session: Option<PortablePtySession>,
    launch_error: Option<String>,
}

impl TerminalPaneView {
    pub fn new(pane: PaneConfig, cx: &mut Context<Self>) -> Self {
        let mut session = match spawn_portable_pty_session(&pane.id, &pane.command, 80, 24) {
            Ok(session) => session,
            Err(error) => {
                return Self {
                    pane_id: pane.id,
                    title: pane.title,
                    command: pane.command,
                    terminal: None,
                    session: None,
                    launch_error: Some(error.to_string()),
                };
            }
        };

        let Some(io) = session.take_io() else {
            return Self {
                pane_id: pane.id,
                title: pane.title,
                command: pane.command,
                terminal: None,
                session: Some(session),
                launch_error: Some("pty session I/O was already taken".to_string()),
            };
        };

        let resize_handle = session.resize_handle();
        let terminal = cx.new(|cx| {
            TerminalView::new(io.writer, io.reader, terminal_config(), cx).with_resize_callback(
                move |cols, rows| {
                    let _ = resize_handle.resize(cols, rows);
                },
            )
        });

        Self {
            pane_id: pane.id,
            title: pane.title,
            command: pane.command,
            terminal: Some(terminal),
            session: Some(session),
            launch_error: None,
        }
    }
}

impl Drop for TerminalPaneView {
    fn drop(&mut self) {
        if let Some(session) = &mut self.session {
            let _ = session.kill();
        }
    }
}

impl Render for TerminalPaneView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        let header = div()
            .flex()
            .items_center()
            .justify_between()
            .gap_2()
            .flex_none()
            .border_b_1()
            .border_color(rgb(0x2a2a2a))
            .bg(rgb(0x171717))
            .px_2()
            .py_1()
            .text_xs()
            .text_color(rgb(0xd4d4d4))
            .child(
                div()
                    .truncate()
                    .child(format!("{} · {}", self.title, self.pane_id)),
            )
            .child(
                div()
                    .truncate()
                    .text_color(rgb(0x737373))
                    .child(self.command.clone()),
            );

        let body = if let Some(terminal) = &self.terminal {
            div().flex().flex_1().child(terminal.clone())
        } else {
            div()
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .bg(rgb(0x111111))
                .text_color(rgb(0xef4444))
                .child(
                    self.launch_error
                        .clone()
                        .unwrap_or_else(|| "terminal did not start".to_string()),
                )
        };

        div()
            .flex()
            .flex_col()
            .flex_1()
            .bg(rgb(0x111111))
            .child(header)
            .child(body)
    }
}

fn terminal_config() -> TerminalConfig {
    TerminalConfig {
        font_family: "monospace".into(),
        font_size: px(13.0),
        cols: 80,
        rows: 24,
        scrollback: 10000,
        line_height_multiplier: 1.15,
        padding: Edges::all(px(6.0)),
        colors: ColorPalette::default(),
    }
}
