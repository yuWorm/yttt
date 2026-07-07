use gpui::{div, prelude::*, rgb, Context, Div, IntoElement, Render, Window};

use std::path::PathBuf;

use crate::{
    model::{layout::ProjectLayout, workspace::Workspace},
    ui::{sidebar::project_sidebar, split_view::active_split_view, tabs::project_tabs},
};

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

    pub fn dev_fixture() -> Self {
        let mut workspace = Workspace::new();
        workspace
            .open_project(PathBuf::from("/tmp/yttt"), dev_fixture_layout())
            .expect("dev fixture layout should be valid");
        Self { workspace }
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
                        .child(project_tabs(&self.workspace))
                        .child(active_split_view(&self.workspace)),
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

fn dev_fixture_layout() -> ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "yttt"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"

        [tabs.layout]
        type = "split"
        direction = "horizontal"
        ratio = 0.65
        left = { type = "pane", id = "server", title = "server", command = "npm run dev" }
        right = { type = "pane", id = "shell", title = "shell", command = "$SHELL" }

        [[tabs]]
        id = "agent"
        title = "Agent"
        layout = { type = "pane", id = "codex", title = "Codex", command = "codex", kind = "agent", notify_on_exit = true }
    "#,
    )
    .expect("static dev fixture TOML should parse")
}
