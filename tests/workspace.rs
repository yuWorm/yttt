use std::path::PathBuf;
use yttt_agent_core::{
    AgentInstanceId, AgentProcessState, AgentSnapshot, AgentTurnState, AgentViewState, ProviderId,
};

use yttt::model::{
    ids::ProjectId,
    layout::TabStartup,
    project::{ProjectDescriptor, ProjectLocation},
    workspace::{
        CloseProjectDecision, CloseProjectError, ClosedProject, PaneExitCloseOutcome,
        PaneProcessState, TabStartState, Workspace, WorkspaceNavigation,
    },
};

fn local_project(path: PathBuf) -> ProjectDescriptor {
    let location = ProjectLocation::local(path);
    ProjectDescriptor::new(
        ProjectId::from_legacy_location(&location.display_path()),
        location,
    )
}

fn completed_agent_snapshot() -> AgentSnapshot {
    AgentSnapshot {
        instance_id: AgentInstanceId::new("workspace-test-agent").unwrap(),
        provider_id: ProviderId::from_static("test"),
        generation: 1,
        process_state: AgentProcessState::Running,
        turn_state: AgentTurnState::Completed,
        waiting_reason: None,
        waiting_message: None,
        task: None,
        current_action: None,
        last_action_failed: false,
        session: None,
        children: Vec::new(),
        process_exit: None,
        state_started_at: 1,
        updated_at: 1,
        board: Default::default(),
    }
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
fn command_center_requires_open_projects_and_last_close_restores_dashboard() {
    let mut workspace = Workspace::new();
    workspace.select_command_center();
    assert_eq!(workspace.navigation(), &WorkspaceNavigation::Dashboard);
    let restored = Workspace::restore_persisted_state(workspace.persisted_state()).unwrap();
    assert_eq!(restored.navigation(), &WorkspaceNavigation::Dashboard);

    let project_id = workspace
        .open_project(
            local_project(PathBuf::from("/tmp/board-boundary")),
            sample_layout(),
        )
        .unwrap();
    assert_eq!(workspace.selected_project_id(), Some(&project_id));
    workspace.select_command_center();
    assert_eq!(workspace.navigation(), &WorkspaceNavigation::CommandCenter);
    assert!(workspace.selected_project_id().is_none());
    let mut restored = Workspace::restore_persisted_state(workspace.persisted_state()).unwrap();
    assert_eq!(restored.navigation(), &WorkspaceNavigation::CommandCenter);

    assert!(matches!(
        restored.request_close_project(&project_id).unwrap(),
        CloseProjectDecision::Closed(_)
    ));
    assert!(restored.opened_projects().is_empty());
    assert_eq!(restored.navigation(), &WorkspaceNavigation::Dashboard);
    let restored = Workspace::restore_persisted_state(restored.persisted_state()).unwrap();
    assert_eq!(restored.navigation(), &WorkspaceNavigation::Dashboard);
}

#[test]
fn command_center_round_trip_preserves_running_projects_without_selecting_one() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(
            local_project(PathBuf::from("/tmp/board-retention")),
            sample_layout(),
        )
        .unwrap();
    workspace.select_tab("agent").unwrap();
    workspace
        .mark_pane_running(&project_id, "agent", "codex")
        .unwrap();
    workspace
        .record_agent_snapshot(&project_id, "agent", "codex", completed_agent_snapshot())
        .unwrap();
    let projects = workspace.opened_projects().to_vec();

    workspace.select_command_center();
    let encoded = serde_json::to_vec(&workspace.persisted_state()).unwrap();
    let mut restored =
        Workspace::restore_persisted_state(serde_json::from_slice(&encoded).unwrap()).unwrap();

    assert!(restored.selected_project_id().is_none());
    assert_eq!(restored.opened_projects(), projects);
    assert!(restored.close_selected_tab().is_err());
    assert_eq!(restored.opened_projects(), projects);
    restored.select_project(&project_id).unwrap();
    assert_eq!(
        restored.project(&project_id).unwrap().selected_tab_id,
        "agent"
    );
    assert_eq!(restored.opened_projects(), projects);
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
    assert_eq!(workspace.navigation(), &WorkspaceNavigation::Dashboard);
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
fn recording_agent_snapshot_preserves_process_lifecycle() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();

    workspace
        .record_agent_snapshot(&project_id, "agent", "codex", completed_agent_snapshot())
        .unwrap();

    let project = workspace.project(&project_id).unwrap();
    let pane = project
        .tab_state("agent")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "codex")
        .unwrap();
    assert_eq!(pane.process_state, PaneProcessState::Idle);
    assert_eq!(
        pane.agent_snapshot.as_ref().map(AgentSnapshot::view_state),
        Some(AgentViewState::Completed)
    );

    workspace
        .clear_agent_snapshot(&project_id, "agent", "codex")
        .unwrap();
    let pane = workspace
        .project(&project_id)
        .unwrap()
        .tab_state("agent")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "codex")
        .unwrap();
    assert!(pane.agent_snapshot.is_none());
}

#[test]
fn pane_exit_downgrades_live_agent_without_overwriting_terminal_outcomes() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();
    let mut running = completed_agent_snapshot();
    running.turn_state = AgentTurnState::Working;
    workspace
        .record_agent_snapshot(&project_id, "agent", "codex", running)
        .unwrap();
    workspace
        .record_pane_exited(&project_id, "agent", "codex")
        .unwrap();

    let pane = workspace
        .project(&project_id)
        .unwrap()
        .tab_state("agent")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "codex")
        .unwrap();
    assert_eq!(pane.process_state, PaneProcessState::Exited);
    assert_eq!(
        pane.agent_snapshot.as_ref().map(AgentSnapshot::view_state),
        Some(AgentViewState::Stale)
    );

    workspace
        .record_agent_snapshot(&project_id, "agent", "codex", completed_agent_snapshot())
        .unwrap();
    workspace
        .record_pane_exited(&project_id, "agent", "codex")
        .unwrap();
    let pane = workspace
        .project(&project_id)
        .unwrap()
        .tab_state("agent")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "codex")
        .unwrap();
    assert_eq!(
        pane.agent_snapshot.as_ref().map(AgentSnapshot::view_state),
        Some(AgentViewState::Completed)
    );
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
    assert_eq!(project.layout.project.default_tab.as_deref(), Some("dev"));
    assert!(Workspace::restore_persisted_state(workspace.persisted_state()).is_ok());
}

#[test]
fn closing_default_tab_reassigns_default_tab_for_persisted_state() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();
    workspace.select_tab("dev").unwrap();

    workspace.close_selected_tab().unwrap();

    let project = workspace.project(&project_id).unwrap();
    assert_eq!(project.layout.project.default_tab.as_deref(), Some("agent"));
    let restored = Workspace::restore_persisted_state(workspace.persisted_state()).unwrap();
    assert_eq!(
        restored
            .project(&project_id)
            .unwrap()
            .layout
            .project
            .default_tab
            .as_deref(),
        Some("agent")
    );
}

#[test]
fn restoring_state_rejects_missing_default_tab() {
    let mut workspace = Workspace::new();
    workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();
    let mut state = workspace.persisted_state();
    state.opened_projects[0].layout.project.default_tab = Some("missing".to_string());

    assert_eq!(
        Workspace::restore_persisted_state(state).unwrap_err(),
        yttt::model::workspace::WorkspaceRestoreError::InvalidLayout(
            yttt::model::layout::LayoutError::MissingDefaultTab("missing".to_string())
        )
    );
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
fn bulk_closing_default_tab_reassigns_default_tab_for_persisted_state() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), sample_layout())
        .unwrap();

    workspace.close_tabs(&["dev".to_string()]).unwrap();

    let project = workspace.project(&project_id).unwrap();
    assert_eq!(project.layout.project.default_tab.as_deref(), Some("agent"));
    assert!(Workspace::restore_persisted_state(workspace.persisted_state()).is_ok());
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
    assert!(project.layout.project.default_tab.is_none());
    assert!(Workspace::restore_persisted_state(workspace.persisted_state()).is_ok());
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
fn process_exit_closes_default_single_pane_tab() {
    let mut workspace = Workspace::new();
    let mut layout = sample_layout();
    layout.project.default_tab = Some("agent".to_string());
    let project_id = workspace
        .open_project(local_project(PathBuf::from("/tmp/yttt")), layout)
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
    assert_eq!(project.layout.project.default_tab.as_deref(), Some("dev"));
    assert!(Workspace::restore_persisted_state(workspace.persisted_state()).is_ok());
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
    assert!(project.layout.project.default_tab.is_none());
    assert!(Workspace::restore_persisted_state(workspace.persisted_state()).is_ok());
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
fn legacy_restoring_pane_reads_as_running() {
    let mut workspace = Workspace::new();
    let project_id = workspace
        .open_project(
            local_project(PathBuf::from("/tmp/recovery")),
            sample_layout(),
        )
        .unwrap();
    workspace
        .mark_pane_running(&project_id, "dev", "shell")
        .unwrap();
    let saved = serde_json::to_string(&workspace.persisted_state())
        .unwrap()
        .replace("\"Running\"", "\"Restoring\"");
    assert!(saved.contains("\"Restoring\""));

    let restored =
        Workspace::restore_persisted_state(serde_json::from_str(&saved).unwrap()).unwrap();
    let pane = restored
        .project(&project_id)
        .unwrap()
        .tab_state("dev")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "shell")
        .unwrap();
    assert_eq!(pane.process_state, PaneProcessState::Running);
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
