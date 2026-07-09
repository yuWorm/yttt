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
