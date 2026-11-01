//! yttt patch: the programmatic editing surface its vim layer drives.
//!
//! Offsets are UTF-8 byte offsets. See `vendor/PATCHES.md`.

use std::ops::Range;

use gpui::{App, Context, Window};
use sum_tree::Bias;
use unicode_segmentation::UnicodeSegmentation as _;

use super::{InputBaseState, RopeExt as _};
use crate::input::InputModeKind;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum InputCursorShape {
    #[default]
    Bar,
    Block,
}

impl<M: InputModeKind> InputBaseState<M> {
    /// The fixed end of the active selection.
    pub fn selection_anchor(&self) -> usize {
        let active = self.active_selection();
        if active.reversed {
            active.end
        } else {
            active.start
        }
    }

    /// The moving end of the active selection.
    pub fn selection_head(&self) -> usize {
        self.cursor()
    }

    /// Move the cursor and collapse every selection.
    pub fn set_cursor_offset(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.move_to(offset.min(self.text.len()), None, cx);
    }

    /// Select from a fixed anchor to a moving head.
    pub fn set_anchored_selection(
        &mut self,
        anchor: usize,
        head: usize,
        cx: &mut Context<Self>,
    ) {
        let anchor = self.cursor_boundary(anchor.min(self.text.len()), Bias::Left);
        let head = self.cursor_boundary(head.min(self.text.len()), Bias::Left);
        self.undo_manager.break_transaction_coalescing();
        self.selections.remove_all_but_active();
        self.set_selection(anchor.min(head), anchor.max(head));
        self.active_selection_mut().reversed = head < anchor;
        self.selected_word_range = None;
        self.ime_marked_range = None;
        self.cursor_line_end_affinity = false;
        self.scroll_to(head, None, cx);
        self.pause_blink_cursor(cx);
        self.update_preferred_column();
        M::hide_context_menu(self, cx);
        M::clear_inline_completion(self, cx);
        cx.notify();
    }

    /// Replace a range as an editor operation, even while text input is off.
    pub fn replace_range(
        &mut self,
        range: Range<usize>,
        text: impl AsRef<str>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let start = self.text.clip_offset(range.start.min(self.text.len()), Bias::Left);
        let end = self.text.clip_offset(range.end.min(self.text.len()), Bias::Right);
        let range_utf16 = self.range_to_utf16(&(start..end));
        self.with_text_input_enabled(|this| {
            this.replace_text_in_range_silent(Some(range_utf16), text.as_ref(), window, cx);
        });
    }

    /// Open one undo group for a compound command. Groups nest.
    pub fn begin_history_group(&mut self) {
        self.undo_manager.begin_transaction();
    }

    /// Close the group opened by [`Self::begin_history_group`].
    pub fn end_history_group(&mut self) {
        self.undo_manager.commit_transaction();
    }

    /// Enable or disable edits from the platform input handler and edit actions.
    pub fn set_text_input_enabled(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.text_input_enabled == enabled {
            return;
        }
        self.text_input_enabled = enabled;
        if !enabled {
            self.ime_marked_range = None;
        }
        cx.notify();
    }

    pub fn text_input_enabled(&self) -> bool {
        self.text_input_enabled
    }

    pub fn set_cursor_shape(&mut self, shape: InputCursorShape, cx: &mut Context<Self>) {
        if self.cursor_shape != shape {
            self.cursor_shape = shape;
            cx.notify();
        }
    }

    pub fn cursor_shape(&self) -> InputCursorShape {
        self.cursor_shape
    }

    pub fn previous_grapheme_boundary(&self, offset: usize) -> usize {
        let offset = self.text.clip_offset(offset.min(self.text.len()), Bias::Left);
        if offset == 0 {
            return 0;
        }
        let row = self.text.offset_to_point(offset).row;
        let start = self.text.line_start_offset(row.saturating_sub(1));
        let end = self.line_end_including_break(row);
        let text = self.text.slice(start..end).to_string();
        let relative = offset - start;
        text.grapheme_indices(true)
            .take_while(|(index, _)| *index < relative)
            .map(|(index, _)| start + index)
            .last()
            .unwrap_or(0)
    }

    pub fn next_grapheme_boundary(&self, offset: usize) -> usize {
        let offset = self.text.clip_offset(offset.min(self.text.len()), Bias::Left);
        if offset >= self.text.len() {
            return self.text.len();
        }
        let row = self.text.offset_to_point(offset).row;
        let start = self.text.line_start_offset(row);
        let end = self.line_end_including_break(row);
        let text = self.text.slice(start..end).to_string();
        let relative = offset - start;
        text.grapheme_indices(true)
            .find(|(index, _)| *index > relative)
            .map(|(index, _)| start + index)
            .unwrap_or(end)
    }

    /// Undo one group even while text input is off, returning the new cursor.
    pub fn undo_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        self.with_text_input_enabled(|this| this.replay_history(true, window, cx))
    }

    /// Redo one group even while text input is off, returning the new cursor.
    pub fn redo_edit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<usize> {
        self.with_text_input_enabled(|this| this.replay_history(false, window, cx))
    }

    /// Move across rendered display lines, including soft-wrapped rows.
    pub fn move_display_lines(
        &mut self,
        lines: isize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.move_vertical(lines, false, window, cx);
    }

    /// Jump to the next or previous match of the active search query.
    pub fn move_to_search_match(
        &mut self,
        backwards: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = if backwards {
            self.previous_search_match(cx)
        } else {
            self.next_search_match(cx)
        };
        if let Some(range) = range {
            self.move_to(range.start, None, cx);
            self.focus(window, cx);
        }
    }

    pub fn search_panel_is_open(&self, _: &App) -> bool {
        self.search_session().is_active()
    }

    fn line_end_including_break(&self, row: usize) -> usize {
        if row.saturating_add(1) < self.text.lines_len() {
            self.text.line_start_offset(row + 1)
        } else {
            self.text.len()
        }
    }

    fn with_text_input_enabled<R>(&mut self, edit: impl FnOnce(&mut Self) -> R) -> R {
        let was_enabled = std::mem::replace(&mut self.text_input_enabled, true);
        let result = edit(self);
        self.text_input_enabled = was_enabled;
        result
    }
}
