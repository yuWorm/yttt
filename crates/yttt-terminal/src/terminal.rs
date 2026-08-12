//! Framework adapter for the canonical terminal core.

use crate::event::GpuiEventProxy;

pub use yttt_terminal_core::{ParserBatchStats, TerminalParser, TerminalScrollbarMetrics};

pub type TerminalState = yttt_terminal_core::TerminalState<GpuiEventProxy>;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::event::{TerminalEvent, TerminalEventMailbox};
    use alacritty_terminal::grid::{Dimensions, Scroll};
    use alacritty_terminal::index::{Column, Line, Point as AlacPoint};
    use std::sync::Arc;

    fn event_proxy() -> GpuiEventProxy {
        let (mailbox, _) = TerminalEventMailbox::new();
        GpuiEventProxy::new(mailbox)
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
        let (mailbox, _) = TerminalEventMailbox::new();
        let mut terminal = TerminalState::new(80, 24, GpuiEventProxy::new(mailbox.clone()));

        terminal.process_bytes(b"\x1b[c");

        let events = mailbox.drain().events;
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
