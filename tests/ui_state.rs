use std::path::PathBuf;

use yttt::{
    model::workspace::Workspace,
    palette::PaletteKind,
    ui::{root::RootView, split_view::visible_pane_titles, tabs::visible_tab_titles},
};

#[test]
fn root_view_starts_with_empty_workspace() {
    let root = RootView::new();

    assert!(root.workspace().opened_projects().is_empty());
}

#[test]
fn root_view_dev_fixture_contains_sample_project() {
    let root = RootView::dev_fixture();

    assert_eq!(root.workspace().opened_projects().len(), 1);
}

#[test]
fn visible_tab_titles_come_from_selected_project() {
    let workspace = workspace_with_sample_project();

    assert_eq!(visible_tab_titles(&workspace), vec!["Dev", "Agent"]);
}

#[test]
fn visible_pane_titles_come_from_selected_tab() {
    let workspace = workspace_with_sample_project();

    assert_eq!(visible_pane_titles(&workspace), vec!["server", "shell"]);
}

#[test]
fn root_view_tab_palette_scopes_to_current_project_tabs() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Tab);

    assert_eq!(root.visible_palette_titles(), vec!["Dev", "Agent"]);
}

#[test]
fn root_view_pane_palette_scopes_to_current_tab_panes() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Pane);

    assert_eq!(root.visible_palette_titles(), vec!["server", "shell"]);
}

#[test]
fn root_view_confirming_tab_palette_selection_switches_tabs() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Tab);
    root.set_palette_query("agent");
    root.confirm_palette_selection().unwrap();

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    assert_eq!(project.selected_tab_id, "agent");
}

#[test]
fn root_view_confirming_pane_palette_selection_focuses_pane() {
    let mut root = RootView::dev_fixture();

    root.open_palette(PaletteKind::Pane);
    root.set_palette_query("shell");
    root.confirm_palette_selection().unwrap();

    let project_id = root.workspace().selected_project_id().unwrap().clone();
    let project = root.workspace().project(&project_id).unwrap();
    let tab = project.tab_state("dev").unwrap();
    assert_eq!(tab.focused_pane_id.as_deref(), Some("shell"));
}

fn workspace_with_sample_project() -> Workspace {
    let mut workspace = Workspace::new();
    workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace
}

fn sample_layout() -> yttt::model::layout::ProjectLayout {
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
    .unwrap()
}
