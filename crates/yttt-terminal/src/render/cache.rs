use super::content::{RenderOverlayState, TerminalRenderSnapshot};
use alacritty_terminal::index::Line;
use alacritty_terminal::selection::SelectionRange;
use std::collections::BTreeSet;

/// Authoritative resolved visible-frame cache.
#[derive(Clone, Debug, Default)]
pub(crate) struct TerminalRenderCache {
    frame: Option<TerminalRenderSnapshot>,
    next_generation: u64,
    previous_selection_rows: BTreeSet<usize>,
    previous_cursor_row: Option<usize>,
    previous_interaction_rows: BTreeSet<usize>,
}

impl TerminalRenderCache {
    pub(crate) fn overlay_damage_rows(
        &mut self,
        selection: Option<SelectionRange>,
        cursor_row: Option<usize>,
        display_offset: usize,
        screen_lines: usize,
        overlays: &RenderOverlayState,
    ) -> Vec<usize> {
        let mut current_selection_rows = BTreeSet::new();
        if let Some(selection) = selection {
            for viewport_row in 0..screen_lines {
                let line = Line(viewport_row as i32 - display_offset as i32);
                if selection.start.line <= line && selection.end.line >= line {
                    current_selection_rows.insert(viewport_row);
                }
            }
        }

        let mut current_interaction_rows = BTreeSet::new();
        for hint in &overlays.hints {
            let row = hint.point.line.0 + display_offset as i32;
            if (0..screen_lines as i32).contains(&row) {
                current_interaction_rows.insert(row as usize);
            }
        }
        let ranges = overlays
            .search_matches
            .iter()
            .chain(overlays.focused_search_match.iter())
            .chain(overlays.hovered_hyperlink.iter());
        for range in ranges {
            for viewport_row in 0..screen_lines {
                let line = Line(viewport_row as i32 - display_offset as i32);
                if range.start().line <= line && range.end().line >= line {
                    current_interaction_rows.insert(viewport_row);
                }
            }
        }

        let mut damaged = self
            .previous_selection_rows
            .union(&current_selection_rows)
            .copied()
            .collect::<BTreeSet<_>>();
        if let Some(row) = self.previous_cursor_row {
            damaged.insert(row);
        }
        if let Some(row) = cursor_row.filter(|row| *row < screen_lines) {
            damaged.insert(row);
        }
        damaged.extend(
            self.previous_interaction_rows
                .union(&current_interaction_rows)
                .copied(),
        );
        self.previous_interaction_rows = current_interaction_rows;
        self.previous_selection_rows = current_selection_rows;
        self.previous_cursor_row = cursor_row;
        damaged.into_iter().collect()
    }

    /// Merge resolved row updates and return the number of rows whose display
    /// generation actually changed.
    pub(crate) fn merge(&mut self, mut update: TerminalRenderSnapshot) -> usize {
        let incompatible = self.frame.as_ref().is_none_or(|frame| {
            frame.display_offset != update.display_offset
                || frame.cols != update.cols
                || frame.screen_lines != update.screen_lines
        });
        if incompatible {
            self.next_generation = self.next_generation.wrapping_add(1);
            for row in &mut update.rows {
                row.generation = self.next_generation;
            }
            let rebuilt_rows = update.rows.len();
            self.frame = Some(update);
            return rebuilt_rows;
        }

        let mut rebuilt_rows = 0;

        let Some(frame) = &mut self.frame else {
            return 0;
        };
        for mut row in update.rows {
            let viewport_row = row.line.0 + update.display_offset as i32;
            if viewport_row < 0 || viewport_row as usize >= frame.rows.len() {
                continue;
            }
            let cached = &mut frame.rows[viewport_row as usize];
            if cached.line == row.line && cached.cells == row.cells {
                continue;
            }
            self.next_generation = self.next_generation.wrapping_add(1);
            row.generation = self.next_generation;
            *cached = row;
            rebuilt_rows += 1;
        }
        frame.cursor = update.cursor;
        frame.default_background = update.default_background;
        frame.default_foreground = update.default_foreground;
        frame.damage = update.damage;
        frame.history_size = update.history_size;
        rebuilt_rows
    }

    pub(crate) fn frame(&self) -> Option<&TerminalRenderSnapshot> {
        self.frame.as_ref()
    }

    pub(crate) fn clear(&mut self) {
        self.frame = None;
        self.previous_selection_rows.clear();
        self.previous_cursor_row = None;
        self.previous_interaction_rows.clear();
    }

    #[cfg(test)]
    pub(crate) fn row_generations(&self) -> Vec<u64> {
        self.frame
            .as_ref()
            .map(|frame| frame.rows.iter().map(|row| row.generation).collect())
            .unwrap_or_default()
    }
}
