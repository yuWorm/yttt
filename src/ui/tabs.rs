use gpui::{App, ClickEvent, IntoElement, SharedString, Window, div, prelude::*, rgb};
use gpui_component::tab::{Tab, TabBar};

use crate::{model::workspace::Workspace, ui::components::SelectableState};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTabItem {
    pub id: String,
    pub title: String,
    pub status: Option<String>,
    pub state: SelectableState,
}

pub fn visible_tab_titles(workspace: &Workspace) -> Vec<String> {
    visible_tab_items(workspace)
        .into_iter()
        .map(|tab| tab.title)
        .collect()
}

pub fn visible_tab_items(workspace: &Workspace) -> Vec<ProjectTabItem> {
    let Some(selected_project_id) = workspace.selected_project_id() else {
        return Vec::new();
    };
    let Some(project) = workspace.project(selected_project_id) else {
        return Vec::new();
    };

    project
        .layout
        .tabs
        .iter()
        .map(|tab| ProjectTabItem {
            id: tab.id.clone(),
            title: tab.title.clone(),
            status: project
                .tab_state(&tab.id)
                .map(|state| format!("{:?}", state.start_state).to_ascii_lowercase()),
            state: if tab.id == project.selected_tab_id {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            },
        })
        .collect()
}

pub fn project_tabs<H, F>(workspace: &Workspace, mut on_select_tab: F) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(String) -> H,
{
    let items = visible_tab_items(workspace);
    if items.is_empty() {
        return div().into_any_element();
    }

    let selected_index = items
        .iter()
        .position(|tab| tab.state == SelectableState::Active)
        .unwrap_or(0);

    let mut tabs = Vec::new();
    for item in items {
        let mut tab = Tab::new()
            .label(SharedString::from(item.title.clone()))
            .on_click(on_select_tab(item.id.clone()));

        if let Some(status) = item.status {
            tab = tab.suffix(div().text_xs().text_color(rgb(0xa3a3a3)).child(status));
        }

        tabs.push(tab);
    }

    TabBar::new("project-tabs")
        .underline()
        .selected_index(selected_index)
        .children(tabs)
        .bg(rgb(0x171717))
        .p_2()
        .into_any_element()
}
