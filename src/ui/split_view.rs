use gpui::{div, prelude::*, rgb, Div, IntoElement};

use crate::model::{
    layout::{LayoutNode, SplitDirection},
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
        .child(render_layout(layout, &project.selected_tab_id))
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

fn render_layout(layout: &LayoutNode, tab_id: &str) -> Div {
    match layout {
        LayoutNode::Pane(pane) => div()
            .flex()
            .flex_col()
            .flex_1()
            .gap_2()
            .bg(rgb(0x111111))
            .text_color(rgb(0xf5f5f5))
            .p_3()
            .child(pane.title.clone())
            .child(div().text_xs().text_color(rgb(0xa3a3a3)).child(format!(
                "{tab_id} · {}",
                pane.command
            ))),
        LayoutNode::Split(split) => {
            let mut container = div().flex().flex_1().gap_1();
            if split.direction == SplitDirection::Vertical {
                container = container.flex_col();
            }
            container
                .child(render_layout(&split.left, tab_id))
                .child(render_layout(&split.right, tab_id))
        }
    }
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
