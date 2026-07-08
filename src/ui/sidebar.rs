use gpui::{App, ClickEvent, IntoElement, SharedString, Window, div, prelude::*, px, rgb};
use gpui_component::sidebar::{Sidebar, SidebarMenu, SidebarMenuItem};

use crate::model::workspace::Workspace;
use crate::ui::agent_status::{agent_status_label, project_agent_status};
use crate::ui::components::SelectableState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSidebarItem {
    pub id: String,
    pub title: String,
    pub path: String,
    pub agent_status: Option<String>,
    pub state: SelectableState,
}

pub fn visible_project_items(workspace: &Workspace) -> Vec<ProjectSidebarItem> {
    let selected_project_id = workspace.selected_project_id();

    workspace
        .opened_projects()
        .iter()
        .map(|project| ProjectSidebarItem {
            id: project.id.as_str().to_string(),
            title: project.layout.project.name.clone(),
            path: project.path.display().to_string(),
            agent_status: project_agent_status(project)
                .map(agent_status_label)
                .map(String::from),
            state: if Some(&project.id) == selected_project_id {
                SelectableState::Active
            } else {
                SelectableState::Inactive
            },
        })
        .collect()
}

pub fn project_sidebar<H, F>(workspace: &Workspace, mut on_select_project: F) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    F: FnMut(String) -> H,
{
    let mut menu = SidebarMenu::new();

    for item in visible_project_items(workspace) {
        let suffix = match item.agent_status.as_deref() {
            Some(status) => format!("{} · {status}", compact_path(&item.path)),
            None => compact_path(&item.path),
        };
        menu = menu.child(
            SidebarMenuItem::new(SharedString::from(item.title.clone()))
                .active(item.state == SelectableState::Active)
                .suffix(div().text_xs().text_color(rgb(0xa3a3a3)).child(suffix))
                .on_click(on_select_project(item.id.clone())),
        );
    }

    Sidebar::left()
        .collapsible(false)
        .w(px(220.0))
        .header(div().text_sm().child("Projects"))
        .child(menu)
}

fn compact_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}
