use gpui::{div, prelude::*, rgb, Context, Div, IntoElement, Render, Window};

use crate::{model::workspace::Workspace, ui::sidebar::project_sidebar};

pub struct RootView {
    workspace: Workspace,
}

impl RootView {
    pub fn new() -> Self {
        Self {
            workspace: Workspace::new(),
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }
}

impl Default for RootView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for RootView {
    fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
        if self.workspace.opened_projects().is_empty() {
            empty_workspace()
        } else {
            div()
                .flex()
                .size_full()
                .bg(rgb(0x101010))
                .text_color(rgb(0xf5f5f5))
                .child(project_sidebar(&self.workspace))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .gap_3()
                        .p_4()
                        .child("Project content"),
                )
        }
    }
}

fn empty_workspace() -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .size_full()
        .justify_center()
        .items_center()
        .bg(rgb(0x101010))
        .text_color(rgb(0xf5f5f5))
        .child(div().text_xl().child("yttt"))
        .child("Open a directory or choose a recent project.")
        .child("Command Palette: Cmd/Ctrl+P")
}
