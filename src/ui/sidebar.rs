use gpui::{
    App, ClickEvent, InteractiveElement as _, IntoElement, Pixels, Rgba,
    StatefulInteractiveElement as _, Window, div, prelude::*, px,
};
use gpui_component::{Icon, IconName};

use crate::model::workspace::Workspace;
use crate::ui::agent_status::{agent_status_label, project_agent_status};
use crate::ui::components::SelectableState;
use crate::ui::theme::WorkbenchTheme;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectSidebarItem {
    pub id: String,
    pub title: String,
    pub path: String,
    pub agent_status: Option<String>,
    pub state: SelectableState,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProjectSidebarStyle {
    pub width: Pixels,
    pub border_width: Pixels,
    pub item_height: Pixels,
    pub background: Rgba,
    pub active_background: Rgba,
    pub hover_background: Rgba,
}

pub fn project_sidebar_style() -> ProjectSidebarStyle {
    let theme = WorkbenchTheme::dark();

    ProjectSidebarStyle {
        width: px(216.0),
        border_width: px(1.0),
        item_height: px(28.0),
        background: theme.app_background,
        active_background: theme.active_surface,
        hover_background: theme.hover_surface,
    }
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
    let style = project_sidebar_style();
    let theme = WorkbenchTheme::dark();
    let mut sidebar = div()
        .flex()
        .flex_col()
        .flex_none()
        .h_full()
        .w(style.width)
        .bg(style.background)
        .border_r_1()
        .border_color(theme.border)
        .px_2()
        .py_3()
        .child(
            div()
                .px_1()
                .pb_3()
                .text_xs()
                .text_color(theme.text_subtle)
                .child("Projects"),
        );

    for (index, item) in visible_project_items(workspace).into_iter().enumerate() {
        let suffix = match item.agent_status.as_deref() {
            Some(status) => format!("{} · {status}", compact_path(&item.path)),
            None => compact_path(&item.path),
        };
        let on_click = on_select_project(item.id.clone());
        sidebar = sidebar.child(project_sidebar_item(
            index, item, suffix, style, theme, on_click,
        ));
    }

    sidebar
}

fn project_sidebar_item<H>(
    index: usize,
    item: ProjectSidebarItem,
    suffix: String,
    style: ProjectSidebarStyle,
    theme: WorkbenchTheme,
    on_select_project: H,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let background = if item.state == SelectableState::Active {
        style.active_background
    } else {
        style.background
    };
    let title_color = if item.state == SelectableState::Active {
        theme.text
    } else {
        theme.text_muted
    };

    div()
        .id(("project-sidebar-item", index))
        .flex()
        .items_center()
        .justify_between()
        .gap_2()
        .h(style.item_height)
        .w(style.width)
        .rounded_sm()
        .px_2()
        .bg(background)
        .hover(move |this| this.bg(style.hover_background))
        .on_click(on_select_project)
        .child(
            div()
                .flex()
                .items_center()
                .gap_2()
                .overflow_hidden()
                .child(
                    Icon::new(IconName::Folder)
                        .size_3()
                        .text_color(theme.text_subtle),
                )
                .child(
                    div()
                        .text_sm()
                        .text_color(title_color)
                        .truncate()
                        .child(item.title),
                ),
        )
        .child(
            div()
                .flex_none()
                .text_xs()
                .text_color(theme.text_subtle)
                .truncate()
                .child(suffix),
        )
}

fn compact_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}
