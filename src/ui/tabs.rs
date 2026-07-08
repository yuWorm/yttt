use gpui::{App, ClickEvent, IntoElement, Pixels, SharedString, Window, div, prelude::*, px, rgb};
use gpui_component::tab::{Tab, TabBar};

use crate::{
    model::workspace::{TabStartState, Workspace},
    ui::{
        agent_status::{agent_status_label, tab_agent_status},
        components::SelectableState,
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTabItem {
    pub id: String,
    pub title: String,
    pub status: Option<String>,
    pub state: SelectableState,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectTabsStyle {
    pub height: Pixels,
    pub border_width: Pixels,
}

pub fn project_tabs_style() -> ProjectTabsStyle {
    ProjectTabsStyle {
        height: px(32.0),
        border_width: px(1.0),
    }
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
            status: project.tab_state(&tab.id).map(|state| {
                let mut parts = vec![tab_start_state_label(state.start_state).to_string()];
                if let Some(agent_status) = tab_agent_status(project, &tab.id) {
                    parts.push(agent_status_label(agent_status).to_string());
                }
                parts.join(" · ")
            }),
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
    let style = project_tabs_style();
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
        .h(style.height)
        .bg(rgb(0x242a34))
        .border_b_1()
        .border_color(rgb(0x343b46))
        .px_2()
        .into_any_element()
}

fn tab_start_state_label(state: TabStartState) -> &'static str {
    match state {
        TabStartState::Lazy => "lazy",
        TabStartState::Started => "started",
    }
}
