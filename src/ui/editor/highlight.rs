use std::{ops::Range, sync::Arc};

use gpui::{App, HighlightStyle};
use gpui_base::input::Rope;
use gpui_component::{ActiveTheme as _, highlighter::SyntaxHighlighter};

pub use gpui_component::highlighter::HighlightTheme;

/// The syntax theme code is currently painted with.
pub fn current_highlight_theme(cx: &App) -> Arc<HighlightTheme> {
    cx.theme().highlight_theme.clone()
}

/// Syntax colors for each line of `source`, with ranges relative to that line.
///
/// `line_ranges` are the byte ranges of the lines inside `source`, and
/// `highlighter_name` comes from an [`super::EditorLanguageResolution`].
pub fn highlight_lines(
    highlighter_name: &str,
    source: &str,
    line_ranges: impl IntoIterator<Item = Range<usize>>,
    theme: &HighlightTheme,
) -> Vec<Vec<(Range<usize>, HighlightStyle)>> {
    let rope = Rope::from(source);
    let mut highlighter = SyntaxHighlighter::new(highlighter_name);
    highlighter.update(None, &rope, None);
    line_ranges
        .into_iter()
        .map(|range| {
            let line_start = range.start;
            let line_end = range.end;
            highlighter
                .styles(&(line_start..line_end), theme)
                .into_iter()
                .filter_map(|(style_range, style)| {
                    let start = style_range.start.max(line_start) - line_start;
                    let end = style_range.end.min(line_end).saturating_sub(line_start);
                    (start < end).then_some((start..end, style))
                })
                .collect()
        })
        .collect()
}
