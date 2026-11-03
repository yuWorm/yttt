use gpui::{Div, ElementId, Stateful, div, prelude::*, px};

use crate::{style::UiStyle, theme::WorkbenchTheme};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct YtttTreeRowState {
    pub depth: usize,
    pub selected: bool,
    /// The tree holds keyboard focus.
    pub focused: bool,
    /// Outline the selected row while the tree is focused.
    pub focus_ring: bool,
    pub right_clicked: bool,
    pub disabled: bool,
}

/// A file tree row. Callers add the chevron, icon, label and trailing marks.
pub fn yttt_tree_row(
    id: impl Into<ElementId>,
    state: YtttTreeRowState,
    theme: WorkbenchTheme,
    ui_style: UiStyle,
) -> Stateful<Div> {
    let metrics = ui_style.tree;
    let selectable = !state.disabled;
    let outlined = metrics.outlined_focus && state.focused && state.focus_ring;
    let filled = state.selected && selectable && !outlined;
    let ring = state.selected && selectable && state.focus_ring;
    let radius = if metrics.outlined_focus {
        px(0.)
    } else {
        ui_style.radius.compact
    };
    let hover = ui_style.hover_background(theme);

    div()
        .id(id)
        .relative()
        .flex()
        .items_center()
        .w_full()
        .h(metrics.row_height)
        .pl(metrics.padding_left + metrics.indent * state.depth as f32)
        .pr(ui_style.spacing.sm)
        .gap(metrics.content_gap)
        .rounded(radius)
        .text_sm()
        .text_color(if selectable {
            theme.text
        } else {
            theme.text_muted
        })
        .when(metrics.indent_guides, |row| {
            row.children((0..state.depth).map(|level| {
                div()
                    .absolute()
                    .top_0()
                    .bottom_0()
                    .left(
                        metrics.padding_left
                            + metrics.indent * level as f32
                            // Guides run through the center of the parent's icon.
                            + metrics.icon_size / 2.,
                    )
                    .w(ui_style.border.hairline)
                    .bg(theme.border)
            }))
        })
        .when(selectable && !state.selected, |row| {
            row.hover(move |row| row.bg(hover))
        })
        .when(filled, |row| row.bg(ui_style.active_background(theme)))
        .when(ring, |row| {
            row.child(
                div()
                    .debug_selector(|| "project-tree-focused-row-indicator".to_string())
                    .absolute()
                    .inset_0()
                    .rounded(radius)
                    .border(ui_style.border.hairline)
                    .border_color(theme.border_focused),
            )
        })
        .when(state.right_clicked && selectable && !ring, |row| {
            row.child(
                div()
                    .absolute()
                    .inset_0()
                    .rounded(radius)
                    .border(ui_style.border.hairline)
                    .border_color(theme.border),
            )
        })
}
