use gpui::{div, prelude::*, rgb, Div, IntoElement};

use crate::model::workspace::Workspace;

pub fn visible_tab_titles(workspace: &Workspace) -> Vec<String> {
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
        .map(|tab| tab.title.clone())
        .collect()
}

pub fn project_tabs(workspace: &Workspace) -> impl IntoElement {
    let Some(selected_project_id) = workspace.selected_project_id() else {
        return div();
    };
    let Some(project) = workspace.project(selected_project_id) else {
        return div();
    };

    let mut bar = div()
        .flex()
        .gap_1()
        .bg(rgb(0x171717))
        .text_color(rgb(0xf5f5f5))
        .p_2();

    for tab in &project.layout.tabs {
        bar = bar.child(tab_label(
            &tab.title,
            tab.id == project.selected_tab_id,
            project
                .tab_state(&tab.id)
                .map(|state| format!("{:?}", state.start_state).to_ascii_lowercase()),
        ));
    }

    bar
}

fn tab_label(title: &str, selected: bool, status: Option<String>) -> Div {
    let background = if selected { 0x262626 } else { 0x171717 };
    let mut label = div()
        .flex()
        .gap_2()
        .bg(rgb(background))
        .px_3()
        .py_2()
        .child(title.to_string());

    if let Some(status) = status {
        label = label.child(div().text_xs().text_color(rgb(0xa3a3a3)).child(status));
    }

    label
}
