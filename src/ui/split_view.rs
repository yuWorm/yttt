use gpui::{Div, IntoElement, div, prelude::*, px, relative, rgb};

use crate::{
    commands::CommandId,
    model::{
        layout::{LayoutNode, PaneConfig, SplitDirection},
        split_tree::ResizeDirection,
        workspace::Workspace,
    },
};

const SPLIT_RESIZE_DRAG_THRESHOLD_PX: f32 = 6.0;
const SPLIT_POINTER_RESIZE_SENSITIVITY_PX: f32 = 600.0;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerSplitResize {
    pub direction: ResizeDirection,
    pub delta: f32,
}

pub fn visible_pane_titles(workspace: &Workspace) -> Vec<String> {
    let Some((_project, layout)) = selected_tab_layout(workspace) else {
        return Vec::new();
    };

    let mut titles = Vec::new();
    collect_pane_titles(layout, &mut titles);
    titles
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitChildBasis {
    pub left: f32,
    pub right: f32,
}

pub fn root_split_child_basis(workspace: &Workspace) -> Option<SplitChildBasis> {
    let (_project, layout) = selected_tab_layout(workspace)?;
    match layout {
        LayoutNode::Pane(_) => None,
        LayoutNode::Split(split) => Some(split_child_basis(split.ratio)),
    }
}

pub fn split_child_basis(ratio: f32) -> SplitChildBasis {
    SplitChildBasis {
        left: ratio,
        right: 1.0 - ratio,
    }
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
            &mut inert_split_resize_handle,
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
    render_handle: &mut impl FnMut(SplitDirection) -> Div,
) -> Div {
    match layout {
        LayoutNode::Pane(pane) => render_pane(pane, tab_id),
        LayoutNode::Split(split) => {
            let basis = split_child_basis(split.ratio);
            let mut container = div().flex().flex_1().gap_1();
            if split.direction == SplitDirection::Vertical {
                container = container.flex_col();
            }
            container
                .child(split_child(
                    split_view_for_layout(&split.left, tab_id, render_pane, render_handle),
                    basis.left,
                ))
                .child(render_handle(split.direction))
                .child(split_child(
                    split_view_for_layout(&split.right, tab_id, render_pane, render_handle),
                    basis.right,
                ))
        }
    }
}

fn split_child(child: Div, basis: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_basis(relative(basis))
        .flex_shrink()
        .overflow_hidden()
        .child(child)
}

pub fn resize_command_for_drag_delta(
    direction: SplitDirection,
    delta_x: f32,
    delta_y: f32,
) -> Option<CommandId> {
    match direction {
        SplitDirection::Horizontal if delta_x >= SPLIT_RESIZE_DRAG_THRESHOLD_PX => {
            Some(CommandId::PaneResizeRight)
        }
        SplitDirection::Horizontal if delta_x <= -SPLIT_RESIZE_DRAG_THRESHOLD_PX => {
            Some(CommandId::PaneResizeLeft)
        }
        SplitDirection::Vertical if delta_y >= SPLIT_RESIZE_DRAG_THRESHOLD_PX => {
            Some(CommandId::PaneResizeDown)
        }
        SplitDirection::Vertical if delta_y <= -SPLIT_RESIZE_DRAG_THRESHOLD_PX => {
            Some(CommandId::PaneResizeUp)
        }
        _ => None,
    }
}

pub fn pointer_resize_for_drag_delta(
    direction: SplitDirection,
    delta_x: f32,
    delta_y: f32,
) -> Option<PointerSplitResize> {
    let axis_delta = match direction {
        SplitDirection::Horizontal => delta_x,
        SplitDirection::Vertical => delta_y,
    };

    if axis_delta.abs() < SPLIT_RESIZE_DRAG_THRESHOLD_PX {
        return None;
    }

    let resize_direction = match (direction, axis_delta.is_sign_positive()) {
        (SplitDirection::Horizontal, true) => ResizeDirection::Right,
        (SplitDirection::Horizontal, false) => ResizeDirection::Left,
        (SplitDirection::Vertical, true) => ResizeDirection::Down,
        (SplitDirection::Vertical, false) => ResizeDirection::Up,
    };

    Some(PointerSplitResize {
        direction: resize_direction,
        delta: axis_delta.abs() / SPLIT_POINTER_RESIZE_SENSITIVITY_PX,
    })
}

fn inert_split_resize_handle(direction: SplitDirection) -> Div {
    let handle = div().flex_none().bg(rgb(0x1f1f1f));
    match direction {
        SplitDirection::Horizontal => handle.w(px(9.0)).cursor_ew_resize(),
        SplitDirection::Vertical => handle.h(px(9.0)).cursor_ns_resize(),
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
