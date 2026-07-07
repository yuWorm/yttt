use gpui::{div, prelude::*, px, rgb, IntoElement};

use crate::model::workspace::Workspace;

pub fn project_sidebar(workspace: &Workspace) -> impl IntoElement {
    let mut sidebar = div()
        .flex()
        .flex_col()
        .gap_2()
        .w(px(220.0))
        .h_full()
        .bg(rgb(0x171717))
        .text_color(rgb(0xf5f5f5))
        .p_3()
        .child(div().text_sm().child("Projects"));

    for project in workspace.opened_projects() {
        sidebar = sidebar.child(
            div()
                .flex()
                .flex_col()
                .gap_1()
                .child(project.layout.project.name.clone())
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0xa3a3a3))
                        .child(project.path.display().to_string()),
                ),
        );
    }

    sidebar
}
