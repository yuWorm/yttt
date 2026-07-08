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
    pub collapsed_width: Pixels,
    pub border_width: Pixels,
    pub item_height: Pixels,
    pub item_padding_x: Pixels,
    pub background: Rgba,
    pub active_background: Rgba,
    pub hover_background: Rgba,
}

pub fn project_sidebar_style(theme: WorkbenchTheme) -> ProjectSidebarStyle {
    ProjectSidebarStyle {
        width: px(216.0),
        collapsed_width: px(46.0),
        border_width: px(1.0),
        item_height: px(28.0),
        item_padding_x: px(8.0),
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

pub fn project_sidebar<SelectH, SelectF, ToggleH>(
    workspace: &Workspace,
    theme: WorkbenchTheme,
    collapsed: bool,
    on_toggle_sidebar: ToggleH,
    mut on_select_project: SelectF,
) -> impl IntoElement
where
    SelectH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
    SelectF: FnMut(String) -> SelectH,
    ToggleH: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let style = project_sidebar_style(theme);
    let width = if collapsed {
        style.collapsed_width
    } else {
        style.width
    };
    let mut sidebar = div()
        .flex()
        .flex_col()
        .flex_none()
        .h_full()
        .w(width)
        .bg(style.background)
        .border_r_1()
        .border_color(theme.border)
        .px_2()
        .py_3()
        .child(project_sidebar_header(collapsed, theme, on_toggle_sidebar));

    for (index, item) in visible_project_items(workspace).into_iter().enumerate() {
        let suffix = match item.agent_status.as_deref() {
            Some(status) => format!("{} · {status}", compact_path(&item.path)),
            None => compact_path(&item.path),
        };
        let on_click = on_select_project(item.id.clone());
        sidebar = sidebar.child(project_sidebar_item(
            index, item, suffix, collapsed, style, theme, on_click,
        ));
    }

    sidebar
}

fn project_sidebar_header<H>(
    collapsed: bool,
    theme: WorkbenchTheme,
    on_toggle_sidebar: H,
) -> impl IntoElement
where
    H: Fn(&ClickEvent, &mut Window, &mut App) + 'static,
{
    let icon = if collapsed {
        IconName::PanelLeftOpen
    } else {
        IconName::PanelLeftClose
    };
    let mut header = div()
        .flex()
        .items_center()
        .justify_between()
        .pb_3()
        .text_xs()
        .text_color(theme.text_subtle);

    if !collapsed {
        header = header.child(div().px_1().child("Projects"));
    }

    header.child(
        div()
            .id("sidebar-toggle")
            .flex()
            .items_center()
            .justify_center()
            .size_6()
            .rounded_sm()
            .text_color(theme.text_subtle)
            .hover(move |this| this.bg(theme.hover_surface).text_color(theme.text))
            .on_click(on_toggle_sidebar)
            .child(Icon::new(icon).size_3()),
    )
}

fn project_sidebar_item<H>(
    index: usize,
    item: ProjectSidebarItem,
    suffix: String,
    collapsed: bool,
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
        .w_full()
        .rounded_sm()
        .px(style.item_padding_x)
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
                .children((!collapsed).then(|| {
                    div()
                        .text_sm()
                        .text_color(title_color)
                        .truncate()
                        .child(item.title)
                })),
        )
        .children((!collapsed).then(|| {
            div()
                .flex_none()
                .text_xs()
                .text_color(theme.text_subtle)
                .truncate()
                .child(suffix)
        }))
}

fn compact_path(path: &str) -> String {
    path.rsplit('/').next().unwrap_or(path).to_string()
}
