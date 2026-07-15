use gpui::{App, SharedString, Task, Window};
use gpui_component::{
    IndexPath,
    searchable_list::{SearchableListDelegate, SearchableListItem},
};

pub const SYSTEM_FONT_FAMILY_LABEL: &str = "System default";

#[derive(Debug)]
pub(crate) struct FontFamilyOptions {
    items: Vec<SharedString>,
    search_keys: Vec<String>,
    matched_indices: Vec<usize>,
    query: String,
}

impl FontFamilyOptions {
    pub(crate) fn new(items: impl Into<Vec<String>>) -> Self {
        let items = items.into();
        let mut search_keys = Vec::with_capacity(items.len());
        let items = items
            .into_iter()
            .map(|item| {
                search_keys.push(item.to_lowercase());
                SharedString::from(item)
            })
            .collect::<Vec<_>>();
        let matched_indices = (0..items.len()).collect();

        Self {
            items,
            search_keys,
            matched_indices,
            query: String::new(),
        }
    }
}

impl SearchableListDelegate for FontFamilyOptions {
    type Item = SharedString;

    fn items_count(&self, _: usize) -> usize {
        self.matched_indices.len()
    }

    fn item(&self, ix: IndexPath) -> Option<&Self::Item> {
        self.matched_indices
            .get(ix.row)
            .and_then(|index| self.items.get(*index))
    }

    fn position<V>(&self, value: &V) -> Option<IndexPath>
    where
        Self::Item: SearchableListItem<Value = V>,
        V: PartialEq,
    {
        self.matched_indices
            .iter()
            .position(|index| self.items[*index].value() == value)
            .map(|row| IndexPath::default().row(row))
    }

    fn perform_search(&mut self, query: &str, _: &mut Window, _: &mut App) -> Task<()> {
        let query = query.to_lowercase();
        if query == self.query {
            return Task::ready(());
        }

        if query.starts_with(&self.query) {
            self.matched_indices
                .retain(|index| self.search_keys[*index].contains(&query));
        } else {
            self.matched_indices = self
                .search_keys
                .iter()
                .enumerate()
                .filter_map(|(index, search_key)| search_key.contains(&query).then_some(index))
                .collect();
        }
        self.query = query;

        Task::ready(())
    }
}

pub fn font_family_options_from_system(
    current: &str,
    system_fonts: impl IntoIterator<Item = impl Into<String>>,
) -> Vec<String> {
    let current = current.trim();
    let mut values = vec![SYSTEM_FONT_FAMILY_LABEL.to_string()];
    let mut fonts: Vec<String> = system_fonts
        .into_iter()
        .map(Into::into)
        .map(|font| font.trim().to_string())
        .filter(|font| !font.is_empty())
        .collect();
    fonts.sort_by_key(|font| font.to_ascii_lowercase());
    fonts.dedup_by(|left, right| left.eq_ignore_ascii_case(right));
    values.extend(fonts);

    if !current.is_empty() && values.iter().all(|font| font != current) {
        values.insert(1, current.to_string());
    }

    values
}

pub fn font_family_option_for_setting(font_family: &str) -> String {
    let font_family = font_family.trim();
    if font_family.is_empty() {
        SYSTEM_FONT_FAMILY_LABEL.to_string()
    } else {
        font_family.to_string()
    }
}

pub fn font_family_setting_from_option(option: &str) -> String {
    if option == SYSTEM_FONT_FAMILY_LABEL {
        String::new()
    } else {
        option.to_string()
    }
}

pub fn terminal_font_family_options_from_system(
    current: &str,
    system_fonts: impl IntoIterator<Item = impl Into<String>>,
) -> Vec<String> {
    font_family_options_from_system(current, system_fonts)
}

pub fn terminal_font_family_option_for_setting(font_family: &str) -> String {
    font_family_option_for_setting(font_family)
}

pub fn terminal_font_family_setting_from_option(option: &str) -> String {
    font_family_setting_from_option(option)
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, rc::Rc};

    use gpui::{
        AppContext as _, Context, InteractiveElement as _, IntoElement, Modifiers,
        ParentElement as _, Render, Styled as _, TestAppContext, Window, div, px,
    };
    use gpui_component::{
        Root,
        select::{Select, SelectState},
    };

    use super::*;

    struct FontSelectTestView {
        select: gpui::Entity<SelectState<FontFamilyOptions>>,
    }

    impl Render for FontSelectTestView {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
                .debug_selector(|| "font-select-test".to_string())
                .w(px(220.0))
                .h(px(32.0))
                .child(Select::new(&self.select))
        }
    }

    #[gpui::test]
    fn font_family_select_filters_and_confirms_without_rebuilding_items(cx: &mut TestAppContext) {
        cx.update(gpui_component::init);
        let select_slot = Rc::new(RefCell::new(None));
        let select_slot_for_window = select_slot.clone();
        let (_root, mut cx) = cx.add_window_view(move |window, cx| {
            let select = cx.new(|cx| {
                SelectState::new(
                    FontFamilyOptions::new(vec![
                        SYSTEM_FONT_FAMILY_LABEL.to_string(),
                        "Fira Code".to_string(),
                        "JetBrains Mono".to_string(),
                        "Menlo".to_string(),
                    ]),
                    Some(IndexPath::default()),
                    window,
                    cx,
                )
                .searchable(true)
            });
            *select_slot_for_window.borrow_mut() = Some(select.clone());
            let view = cx.new(|_| FontSelectTestView { select });
            Root::new(view, window, cx)
        });
        let select = select_slot.borrow_mut().take().unwrap();
        cx.refresh().unwrap();

        let select_bounds = cx.debug_bounds("font-select-test").unwrap();
        cx.simulate_click(select_bounds.center(), Modifiers::none());
        cx.run_until_parked();
        for key in ["m", "e", "n", "l", "o"] {
            cx.simulate_keystrokes(key);
            cx.run_until_parked();
        }
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();

        cx.read(|app| {
            assert_eq!(
                select.read(app).selected_value().map(AsRef::as_ref),
                Some("Menlo")
            );
        });
    }
}
