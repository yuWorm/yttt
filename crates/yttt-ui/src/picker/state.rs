use std::ops::Range;

use super::PickerItem;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PickerState {
    pub query: String,
    pub selected_index: usize,
}

impl PickerState {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.selected_index = 0;
    }

    pub fn filtered_items<'a>(&self, items: &'a [PickerItem]) -> Vec<&'a PickerItem> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return items.iter().collect();
        }

        items
            .iter()
            .filter(|item| {
                item.id.to_lowercase().contains(&query)
                    || item.title.to_lowercase().contains(&query)
                    || item
                        .subtitle
                        .as_deref()
                        .is_some_and(|subtitle| subtitle.to_lowercase().contains(&query))
                    || item
                        .status
                        .as_deref()
                        .is_some_and(|status| status.to_lowercase().contains(&query))
                    || item
                        .keybinding
                        .as_deref()
                        .is_some_and(|keybinding| keybinding.to_lowercase().contains(&query))
            })
            .collect()
    }

    pub fn clamped_selected_index(&self, items: &[PickerItem]) -> Option<usize> {
        let count = self.filtered_items(items).len();
        if count == 0 {
            None
        } else {
            Some(self.selected_index.min(count.saturating_sub(1)))
        }
    }

    pub fn select_next(&mut self, items: &[PickerItem]) {
        let count = self.filtered_items(items).len();
        if count == 0 {
            self.selected_index = 0;
        } else {
            self.selected_index = (self.selected_index + 1) % count;
        }
    }

    pub fn select_prev(&mut self, items: &[PickerItem]) {
        let count = self.filtered_items(items).len();
        if count == 0 {
            self.selected_index = 0;
        } else if self.selected_index == 0 {
            self.selected_index = count - 1;
        } else {
            self.selected_index -= 1;
        }
    }
}

/// The byte range of the first case-insensitive occurrence of `query` in
/// `text`, matching how [`PickerState::filtered_items`] filters.
pub fn query_match(text: &str, query: &str) -> Option<Range<usize>> {
    let query = query.trim();
    if query.is_empty() {
        return None;
    }
    text.char_indices().find_map(|(start, _)| {
        let mut rest = text[start..].char_indices();
        let mut end = start;
        for wanted in query.chars().flat_map(char::to_lowercase) {
            let (offset, found) = rest.next()?;
            if !found.to_lowercase().eq(std::iter::once(wanted)) {
                return None;
            }
            end = start + offset + found.len_utf8();
        }
        Some(start..end)
    })
}

#[cfg(test)]
mod tests {
    use super::query_match;

    #[test]
    fn query_match_finds_the_first_case_insensitive_occurrence() {
        assert_eq!(query_match("Open Project", "proj"), Some(5..9));
        assert_eq!(query_match("agent: new thread", " NEW "), Some(7..10));
        assert_eq!(query_match("abab", "ab"), Some(0..2));
    }

    #[test]
    fn query_match_handles_empty_and_missing_queries() {
        assert_eq!(query_match("Open", ""), None);
        assert_eq!(query_match("Open", "   "), None);
        assert_eq!(query_match("Open", "close"), None);
        assert_eq!(query_match("Op", "open"), None);
    }

    #[test]
    fn query_match_returns_byte_ranges_for_non_ascii_text() {
        assert_eq!(query_match("打开项目", "项目"), Some(6..12));
        assert_eq!(query_match("Ärger", "ä"), Some(0..2));
    }
}
