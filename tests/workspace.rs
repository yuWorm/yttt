use std::path::PathBuf;

use yttt::model::{
    ids::ProjectId,
    layout::TabStartup,
    project::{ProjectDescriptor, ProjectLocation},
    workspace::{
        AgentStatus, CloseProjectDecision, CloseProjectError, ClosedProject, PaneExitCloseOutcome,
        PaneProcessState, TabStartState, Workspace,
    },
};

fn local_project(path: PathBuf) -> ProjectDescriptor {
    let location = ProjectLocation::local(path);
    ProjectDescriptor::new(
        ProjectId::from_legacy_location(&location.display_path()),
        location,
    )
}

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
        .open_project(local_project(path.clone()), sample_layout())
        .unwrap();

    assert_eq!(workspace.opened_projects().len(), 1);
    assert_eq!(
        workspace.opened_projects()[0].location.local_path(),
        Some(&path)
    );
    assert_eq!(workspace.selected_project_id(), Some(&project_id));
}

#[test]
fn switching_projects_preserves_both_projects() {
    let mut workspace = Workspace::new();
    let first = workspace
        .open_project(local_project(PathBuf::from("/tmp/one")), sample_layout())
        .unwrap();
    let second = workspace
        .open_project(local_project(PathBuf::from("/tmp/two")), sample_layout())
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
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
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
fn opening_project_marks_eager_tabs_started_before_selection() {
    let mut workspace = Workspace::new();
    let mut layout = sample_layout();
    layout.tabs[1].startup = TabStartup::Eager;

    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt-eager")), layout)
        .unwrap();
    let project = workspace.project(&project_id).unwrap();

    assert_eq!(project.selected_tab_id, "dev");
    assert_eq!(
        project.tab_state("agent").unwrap().start_state,
        TabStartState::Started
    );
}

#[test]
fn closing_project_with_running_panes_is_blocked() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
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
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
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
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
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
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();
    workspace
        .mark_pane_running(&project_id, "dev", "server")
        .unwrap();

    let closed = workspace.confirm_close_project(&project_id).unwrap();

    assert_eq!(closed.project_id, project_id);
    assert!(workspace.opened_projects().is_empty());
    assert!(workspace.selected_project_id().is_none());
}

#[test]
fn recording_agent_status_marks_pane_exited_with_result() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();

    workspace
        .record_agent_status(&project_id, "agent", "codex", AgentStatus::Completed)
        .unwrap();

    let project = workspace.project(&project_id).unwrap();
    let pane = project
        .tab_state("agent")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "codex")
        .unwrap();
    assert_eq!(pane.process_state, PaneProcessState::Exited);
    assert_eq!(pane.agent_status, Some(AgentStatus::Completed));
}

#[test]
fn close_selected_tab_selects_adjacent_tab() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();
    workspace.select_tab("agent").unwrap();

    let closed_tab_id = workspace.close_selected_tab().unwrap();

    let project = workspace.project(&project_id).unwrap();
    assert_eq!(closed_tab_id, "agent");
    assert_eq!(project.selected_tab_id, "dev");
    assert!(project.layout.tab("agent").is_none());
    assert!(project.tab_state("agent").is_none());
}

#[test]
fn close_selected_tab_rejects_last_tab() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(
            local_project(PathBuf::from("/tmp/single")),
            single_tab_layout(),
        )
        .unwrap();

    let err = workspace.close_selected_tab().unwrap_err();

    assert_eq!(
        err,
        yttt::model::workspace::WorkspaceError::CannotCloseLastTab
    );
}

#[test]
fn bulk_tab_close_can_leave_a_project_without_terminal_tabs() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();

    let closed = workspace
        .close_tabs(&["dev".to_string(), "agent".to_string()])
        .unwrap();

    assert_eq!(closed, vec!["dev", "agent"]);
    let project = workspace.project(&project_id).unwrap();
    assert!(project.layout.tabs.is_empty());
    assert!(project.tab_states.is_empty());
    assert!(project.selected_tab_id.is_empty());
    assert!(workspace.create_shell_tab().is_ok());
}

#[test]
fn process_exit_closes_exact_pane_without_changing_project_selection() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();

    let outcome = workspace
        .close_pane_for_exit(&project_id, "dev", "server")
        .unwrap();

    let project = workspace.project(&project_id).unwrap();
    assert_eq!(outcome, PaneExitCloseOutcome::PaneClosed);
    assert!(
        project
            .layout
            .tab("dev")
            .unwrap()
            .layout
            .find_pane("server")
            .is_none()
    );
    assert!(
        project
            .layout
            .tab("dev")
            .unwrap()
            .layout
            .find_pane("shell")
            .is_some()
    );
    assert_eq!(project.selected_tab_id, "dev");
    assert_eq!(
        project.tab_state("dev").unwrap().focused_pane_id.as_deref(),
        Some("shell")
    );
}

#[test]
fn process_exit_closes_single_pane_tab() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();
    workspace.select_tab("agent").unwrap();

    let outcome = workspace
        .close_pane_for_exit(&project_id, "agent", "codex")
        .unwrap();

    let project = workspace.project(&project_id).unwrap();
    assert_eq!(outcome, PaneExitCloseOutcome::TabClosed);
    assert!(project.layout.tab("agent").is_none());
    assert!(project.tab_state("agent").is_none());
    assert_eq!(project.selected_tab_id, "dev");
}

#[test]
fn process_exit_keeps_project_open_when_last_tab_closes() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(
            local_project(PathBuf::from("/tmp/single")),
            single_tab_layout(),
        )
        .unwrap();

    let outcome = workspace
        .close_pane_for_exit(&project_id, "dev", "shell")
        .unwrap();

    let project = workspace.project(&project_id).unwrap();
    assert_eq!(outcome, PaneExitCloseOutcome::ProjectEmptied);
    assert_eq!(workspace.opened_projects().len(), 1);
    assert!(project.layout.tabs.is_empty());
    assert!(project.tab_states.is_empty());
    assert_eq!(project.selected_tab_id, "");
}

#[test]
fn rename_selected_tab_changes_title_without_changing_id() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();

    workspace.rename_selected_tab("Main Dev").unwrap();

    let project = workspace.project(&project_id).unwrap();
    let tab = project.layout.tab("dev").unwrap();
    assert_eq!(tab.id, "dev");
    assert_eq!(tab.title, "Main Dev");
}

#[test]
fn rename_focused_pane_changes_title_without_changing_id() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();

    workspace.rename_focused_pane("Server").unwrap();

    let project = workspace.project(&project_id).unwrap();
    let pane = project
        .layout
        .tab("dev")
        .unwrap()
        .layout
        .find_pane("server")
        .unwrap();
    assert_eq!(pane.id, "server");
    assert_eq!(pane.title, "Server");
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

fn single_tab_layout() -> yttt::model::layout::ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "single"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"
        layout = { type = "pane", id = "shell", title = "shell", command = "$SHELL" }
    "#,
    )
    .unwrap()
}
