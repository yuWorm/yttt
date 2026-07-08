use gpui::{App, ClickEvent, IntoElement, SharedString, Window, div, prelude::*, px, rgb};
use gpui_component::sidebar::{Sidebar, SidebarMenu, SidebarMenuItem};

use crate::model::workspace::Workspace;
use crate::ui::components::SelectableState;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSidebarItem {
    pub id: String,
    pub title: String,
    pub path: String,
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
        menu = menu.child(
            SidebarMenuItem::new(SharedString::from(item.title.clone()))
                .active(item.state == SelectableState::Active)
                .suffix(
                    div()
                        .text_xs()
                        .text_color(rgb(0xa3a3a3))
                        .child(compact_path(&item.path)),
                )
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
