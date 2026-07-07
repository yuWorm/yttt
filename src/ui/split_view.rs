use gpui::{Div, IntoElement, div, prelude::*, rgb};

use crate::model::{
    layout::{LayoutNode, PaneConfig, SplitDirection},
    workspace::Workspace,
};

pub fn visible_pane_titles(workspace: &Workspace) -> Vec<String> {
    let Some((_project, layout)) = selected_tab_layout(workspace) else {
        return Vec::new();
    };

    let mut titles = Vec::new();
    collect_pane_titles(layout, &mut titles);
    titles
}

pub fn active_split_view(workspace: &Workspace) -> impl IntoElement {
    let Some((project, layout)) = selected_tab_layout(workspace) else {
        return div();
    };

    div()
        .flex()
        .flex_1()
        .bg(rgb(0x0a0a0a))
        .text_color(rgb(0xf5f5f5))
        .p_2()
        .child(split_view_for_layout(
            layout,
            &project.selected_tab_id,
            &mut render_mock_pane,
        ))
}

fn selected_tab_layout(
    workspace: &Workspace,
) -> Option<(&crate::model::workspace::OpenedProject, &LayoutNode)> {
    let selected_project_id = workspace.selected_project_id()?;
    let project = workspace.project(selected_project_id)?;
    let tab = project
        .layout
        .tabs
        .iter()
        .find(|tab| tab.id == project.selected_tab_id)?;
    Some((project, &tab.layout))
}

pub fn split_view_for_layout(
    layout: &LayoutNode,
    tab_id: &str,
    render_pane: &mut impl FnMut(&PaneConfig, &str) -> Div,
) -> Div {
    match layout {
        LayoutNode::Pane(pane) => render_pane(pane, tab_id),
        LayoutNode::Split(split) => {
            let mut container = div().flex().flex_1().gap_1();
            if split.direction == SplitDirection::Vertical {
                container = container.flex_col();
            }
            container
                .child(split_view_for_layout(&split.left, tab_id, render_pane))
                .child(split_view_for_layout(&split.right, tab_id, render_pane))
        }
    }
}

fn render_mock_pane(pane: &PaneConfig, tab_id: &str) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_1()
        .gap_2()
        .bg(rgb(0x111111))
        .text_color(rgb(0xf5f5f5))
        .p_3()
        .child(pane.title.clone())
        .child(
            div()
                .text_xs()
                .text_color(rgb(0xa3a3a3))
                .child(format!("{tab_id} · {}", pane.command)),
        )
}

fn collect_pane_titles(layout: &LayoutNode, titles: &mut Vec<String>) {
    match layout {
        LayoutNode::Pane(pane) => titles.push(pane.title.clone()),
        LayoutNode::Split(split) => {
            collect_pane_titles(&split.left, titles);
            collect_pane_titles(&split.right, titles);
        }
    }
}
