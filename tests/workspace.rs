use std::path::PathBuf;

use yttt::model::workspace::{
    CloseProjectDecision, CloseProjectError, ClosedProject, TabStartState, Workspace,
};

#[test]
fn new_workspace_starts_empty() {
    let workspace = Workspace::new();

    assert!(workspace.opened_projects().is_empty());
    assert!(workspace.selected_project_id().is_none());
}

#[test]
fn opening_project_adds_it_to_sidebar_state_and_selects_it() {
    let mut workspace = Workspace::new();
    let path = PathBuf::from("/tmp/yttt");

    let project_id = workspace
        .open_project(path.clone(), sample_layout())
        .unwrap();

    assert_eq!(workspace.opened_projects().len(), 1);
    assert_eq!(workspace.opened_projects()[0].path, path);
    assert_eq!(workspace.selected_project_id(), Some(&project_id));
}

#[test]
fn switching_projects_preserves_both_projects() {
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(PathBuf::from("/tmp/one"), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(PathBuf::from("/tmp/two"), sample_layout())
        .unwrap();

    workspace.select_project(&first).unwrap();

    assert_eq!(workspace.opened_projects().len(), 2);
    assert_eq!(workspace.selected_project_id(), Some(&first));
    assert_ne!(first, second);
}

#[test]
fn opening_project_marks_only_default_tab_started() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    let project = workspace.project(&project_id).unwrap();

    assert_eq!(
        project.tab_state("dev").unwrap().start_state,
        TabStartState::Started
    );
    assert_eq!(
        project.tab_state("agent").unwrap().start_state,
        TabStartState::Lazy
    );
}

#[test]
fn closing_project_with_running_panes_is_blocked() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();

    workspace
        .mark_pane_running(&project_id, "dev", "server")
        .unwrap();
    let err = workspace.close_project(&project_id).unwrap_err();

    assert_eq!(err, CloseProjectError::RunningProcesses);
    assert_eq!(workspace.opened_projects().len(), 1);
}

#[test]
fn closing_project_with_no_running_panes_succeeds() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();

    let decision = workspace.request_close_project(&project_id).unwrap();

    assert_eq!(
        decision,
        CloseProjectDecision::Closed(ClosedProject {
            project_id: project_id.clone()
        })
    );
    assert!(workspace.opened_projects().is_empty());
}

#[test]
fn closing_project_with_running_panes_returns_confirmation_requirement() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace
        .mark_pane_running(&project_id, "dev", "server")
        .unwrap();

    let decision = workspace.request_close_project(&project_id).unwrap();

    assert_eq!(
        decision,
        CloseProjectDecision::NeedsConfirmation {
            project_id: project_id.clone(),
            running_pane_count: 1,
        }
    );
    assert_eq!(workspace.opened_projects().len(), 1);
}

#[test]
fn confirmed_close_removes_project_with_running_panes() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(PathBuf::from("/tmp/yttt"), sample_layout())
        .unwrap();
    workspace
        .mark_pane_running(&project_id, "dev", "server")
        .unwrap();

    let closed = workspace.confirm_close_project(&project_id).unwrap();

    assert_eq!(closed.project_id, project_id);
    assert!(workspace.opened_projects().is_empty());
    assert!(workspace.selected_project_id().is_none());
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
