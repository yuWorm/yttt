use std::path::PathBuf;

use serde_json::{Value, json};
use yttt_core::model::{
    editor::{DocumentId, ProjectWorkItemSession, WorkItemId},
    host_workspace::{
        HostWorkspace, HostWorkspaceError, HostWorkspaceState, IntentError, NewTab, PaneAddress,
        ProjectPanel, WorkspaceIntent,
    },
    ids::ProjectId,
    layout::{PaneKind, ProjectLayout, SplitDirection, TerminalExecutionMode},
    project::{ProjectDescriptor, ProjectLocation},
    work_area::{TabGroupId, WorkAreaDropPlacement},
    workspace::{OpenedProject, PaneProcessState, WorkspaceError},
};

const ROOT: &str = "/host/projects/fixture";

fn fixture() -> Value {
    serde_json::from_str(include_str!("fixtures/workspace-schema2.json")).unwrap()
}

fn migrate(snapshot: Value) -> Result<HostWorkspace, HostWorkspaceError> {
    HostWorkspace::from_legacy(serde_json::from_value(snapshot).unwrap(), 7)
}

fn migrated() -> HostWorkspace {
    migrate(fixture()).unwrap()
}

fn project_id() -> ProjectId {
    ProjectId::new("fixture-project")
}

fn pane(tab_id: &str, pane_id: &str) -> PaneAddress {
    PaneAddress {
        project_id: project_id(),
        tab_id: tab_id.to_string(),
        pane_id: pane_id.to_string(),
    }
}

fn project(workspace: &HostWorkspace) -> &OpenedProject {
    workspace.workspace().project(&project_id()).unwrap()
}

fn session(workspace: &HostWorkspace) -> &ProjectWorkItemSession {
    workspace.editor().session(&project_id()).unwrap()
}

fn main_rs() -> DocumentId {
    DocumentId {
        project_id: project_id(),
        canonical_path: PathBuf::from(ROOT).join("src/main.rs"),
    }
}

fn single_pane_layout(tab_id: &str, pane_id: &str) -> ProjectLayout {
    serde_json::from_value(json!({
        "project": { "name": "solo", "default_tab": tab_id },
        "tabs": [{
            "id": tab_id,
            "title": tab_id,
            "layout": { "type": "pane", "id": pane_id, "title": pane_id, "command": "$SHELL" }
        }]
    }))
    .unwrap()
}

/// The active terminal item, when there is one, is always the selected tab.
fn assert_selection_consistent(workspace: &HostWorkspace) {
    if let Some(WorkItemId::Terminal(tab_id)) = session(workspace).active_work_item() {
        assert_eq!(tab_id, &project(workspace).selected_tab_id);
    }
}

#[test]
fn schema_2_snapshot_migrates_and_round_trips() {
    let workspace = migrated();

    assert_eq!(workspace.revision(), 7);
    let tab_ids = project(&workspace)
        .layout
        .tabs
        .iter()
        .map(|tab| tab.id.as_str())
        .collect::<Vec<_>>();
    assert_eq!(tab_ids, ["dev", "agent"]);
    assert_eq!(project(&workspace).selected_tab_id, "dev");
    assert_eq!(session(&workspace).file_ids(), [main_rs()]);
    assert_eq!(
        session(&workspace).active_work_item(),
        Some(&WorkItemId::File(main_rs()))
    );
    assert_eq!(
        session(&workspace).group_items_containing(&WorkItemId::Terminal("dev".into())),
        Some(
            [
                WorkItemId::Terminal("dev".into()),
                WorkItemId::Terminal("agent".into())
            ]
            .as_slice()
        )
    );
    assert!(session(&workspace).project_panel_visible());
    assert_eq!(session(&workspace).project_panel_width(), 312.0);

    let state = workspace.state();
    let decoded: HostWorkspaceState =
        serde_json::from_value(serde_json::to_value(&state).unwrap()).unwrap();
    assert_eq!(HostWorkspace::restore(decoded).unwrap().state(), state);
}

#[test]
fn only_schemas_1_and_2_migrate() {
    let mut snapshot = fixture();
    snapshot["schema_version"] = json!(3);

    assert_eq!(
        migrate(snapshot).unwrap_err(),
        HostWorkspaceError::UnsupportedLegacySchema(3)
    );
}

#[test]
fn migration_repairs_a_default_tab_that_no_longer_exists() {
    let mut snapshot = fixture();
    snapshot["workspace"]["opened_projects"][0]["layout"]["project"]["default_tab"] =
        json!("removed");

    let workspace = migrate(snapshot).unwrap();

    assert_eq!(
        project(&workspace).layout.project.default_tab.as_deref(),
        Some("dev")
    );
}

#[test]
fn migration_rejects_editor_sessions_with_a_different_root() {
    let mut snapshot = fixture();
    snapshot["editor"]["sessions"][0]["root"] = json!("/somewhere/else");

    assert!(matches!(
        migrate(snapshot),
        Err(HostWorkspaceError::InvalidEditor(_))
    ));
}

#[test]
fn split_pane_reports_the_new_pane_and_focuses_it() {
    let mut workspace = migrated();

    let outcome = workspace
        .apply(
            WorkspaceIntent::SplitPane {
                project_id: project_id(),
                tab_id: "dev".into(),
                pane_id: "shell".into(),
                direction: SplitDirection::Vertical,
                command: "$SHELL".into(),
                new_pane_id: None,
            },
            None,
        )
        .unwrap();

    assert_eq!(outcome.revision, 8);
    assert_eq!(workspace.revision(), 8);
    assert!(outcome.panes.removed.is_empty());
    let [added] = outcome.panes.added.as_slice() else {
        panic!("expected one added pane: {:?}", outcome.panes.added);
    };
    assert_eq!(added.tab_id, "dev");
    let dev = project(&workspace).layout.tab("dev").unwrap();
    assert!(dev.layout.find_pane(&added.pane_id).is_some());
    assert_eq!(
        project(&workspace)
            .tab_state("dev")
            .unwrap()
            .focused_pane_id
            .as_deref(),
        Some(added.pane_id.as_str())
    );
}

#[test]
fn close_pane_reports_the_removed_pane() {
    let mut workspace = migrated();

    let outcome = workspace
        .apply(
            WorkspaceIntent::ClosePane {
                project_id: project_id(),
                tab_id: "dev".into(),
                pane_id: "server".into(),
            },
            None,
        )
        .unwrap();

    assert_eq!(outcome.panes.removed, [pane("dev", "server")]);
    assert!(outcome.panes.added.is_empty());
}

#[test]
fn rejected_intent_leaves_revision_and_state_unchanged() {
    let mut workspace = migrated();
    let before = workspace.state();

    let error = workspace
        .apply(
            WorkspaceIntent::ClosePane {
                project_id: project_id(),
                tab_id: "dev".into(),
                pane_id: "missing".into(),
            },
            None,
        )
        .unwrap_err();

    assert_eq!(
        error,
        IntentError::Workspace(WorkspaceError::PaneNotFound("missing".into()))
    );
    assert_eq!(workspace.state(), before);
}

#[test]
fn close_tabs_with_an_unknown_tab_closes_nothing() {
    let mut workspace = migrated();
    let before = workspace.state();

    let error = workspace
        .apply(
            WorkspaceIntent::CloseTabs {
                project_id: project_id(),
                tab_ids: vec!["agent".into(), "missing".into()],
            },
            None,
        )
        .unwrap_err();

    assert_eq!(
        error,
        IntentError::Workspace(WorkspaceError::TabNotFound("missing".into()))
    );
    assert_eq!(workspace.state(), before);
}

#[test]
fn activating_a_terminal_item_selects_its_tab() {
    let mut workspace = migrated();

    workspace
        .apply(
            WorkspaceIntent::ActivateItem {
                project_id: project_id(),
                item: WorkItemId::Terminal("agent".into()),
            },
            None,
        )
        .unwrap();

    assert_eq!(project(&workspace).selected_tab_id, "agent");
    assert_eq!(
        session(&workspace).active_work_item(),
        Some(&WorkItemId::Terminal("agent".into()))
    );
}

#[test]
fn closing_the_selected_tab_removes_it_from_the_work_area_and_reselects() {
    let mut workspace = migrated();
    workspace
        .apply(
            WorkspaceIntent::ActivateItem {
                project_id: project_id(),
                item: WorkItemId::Terminal("agent".into()),
            },
            None,
        )
        .unwrap();

    let outcome = workspace
        .apply(
            WorkspaceIntent::CloseTabs {
                project_id: project_id(),
                tab_ids: vec!["agent".into()],
            },
            None,
        )
        .unwrap();

    assert_eq!(outcome.panes.removed, [pane("agent", "codex")]);
    assert_eq!(project(&workspace).selected_tab_id, "dev");
    assert_eq!(
        session(&workspace).group_items_containing(&WorkItemId::Terminal("agent".into())),
        None
    );
    assert_selection_consistent(&workspace);
}

#[test]
fn activating_a_file_that_is_not_open_is_rejected() {
    let mut workspace = migrated();
    let not_open = DocumentId {
        project_id: project_id(),
        canonical_path: PathBuf::from(ROOT).join("README.md"),
    };

    let error = workspace
        .apply(
            WorkspaceIntent::ActivateItem {
                project_id: project_id(),
                item: WorkItemId::File(not_open),
            },
            None,
        )
        .unwrap_err();

    assert_eq!(
        error,
        IntentError::WorkItemNotFound("fixture-project".into())
    );
    assert_eq!(workspace.revision(), 7);
}

#[test]
fn open_file_must_stay_inside_the_project_root() {
    let mut workspace = migrated();

    let error = workspace
        .apply(
            WorkspaceIntent::OpenFile {
                project_id: project_id(),
                path: "/etc/passwd".into(),
            },
            None,
        )
        .unwrap_err();
    assert_eq!(error, IntentError::FileOutsideProject("/etc/passwd".into()));

    let readme = PathBuf::from(ROOT).join("README.md");
    workspace
        .apply(
            WorkspaceIntent::OpenFile {
                project_id: project_id(),
                path: readme.clone(),
            },
            None,
        )
        .unwrap();
    let expected = DocumentId {
        project_id: project_id(),
        canonical_path: readme,
    };
    assert_eq!(
        session(&workspace).file_ids(),
        [main_rs(), expected.clone()]
    );
    assert_eq!(
        session(&workspace).active_work_item(),
        Some(&WorkItemId::File(expected))
    );
}

#[test]
fn replace_project_layout_requires_the_current_revision() {
    let mut workspace = migrated();
    let replace = || WorkspaceIntent::ReplaceProjectLayout {
        project_id: project_id(),
        layout: single_pane_layout("solo", "main"),
    };

    assert_eq!(
        workspace.apply(replace(), None).unwrap_err(),
        IntentError::RevisionRequired
    );
    assert_eq!(
        workspace.apply(replace(), Some(6)).unwrap_err(),
        IntentError::RevisionConflict {
            expected: 6,
            current: 7
        }
    );

    let outcome = workspace.apply(replace(), Some(7)).unwrap();

    assert_eq!(outcome.panes.added, [pane("solo", "main")]);
    assert_eq!(
        outcome.panes.removed,
        [
            pane("agent", "codex"),
            pane("dev", "pane-1"),
            pane("dev", "server"),
            pane("dev", "shell"),
        ]
    );
    assert_eq!(project(&workspace).selected_tab_id, "solo");
    assert_eq!(
        session(&workspace).group_items_containing(&WorkItemId::Terminal("dev".into())),
        None
    );
    assert_selection_consistent(&workspace);
}

#[test]
fn a_stale_expected_revision_rejects_any_intent() {
    let mut workspace = migrated();

    assert_eq!(
        workspace
            .apply(WorkspaceIntent::SelectCommandCenter, Some(3))
            .unwrap_err(),
        IntentError::RevisionConflict {
            expected: 3,
            current: 7
        }
    );
}

#[test]
fn create_tab_assigns_a_fresh_id_and_selects_it() {
    let mut workspace = migrated();

    let outcome = workspace
        .apply(
            WorkspaceIntent::CreateTab {
                project_id: project_id(),
                tab: NewTab::Shell {
                    command: "$SHELL".into(),
                },
                tab_id: None,
            },
            None,
        )
        .unwrap();

    let [added] = outcome.panes.added.as_slice() else {
        panic!("expected one added pane: {:?}", outcome.panes.added);
    };
    assert!(!["dev", "agent"].contains(&added.tab_id.as_str()));
    assert_eq!(project(&workspace).selected_tab_id, added.tab_id);
    assert_eq!(
        session(&workspace).active_work_item(),
        Some(&WorkItemId::Terminal(added.tab_id.clone()))
    );
}

#[test]
fn creating_an_agent_tab_keeps_its_resume_arguments_and_agent_semantics() {
    let mut workspace = migrated();

    let outcome = workspace
        .apply(
            WorkspaceIntent::CreateTab {
                project_id: project_id(),
                tab: NewTab::Agent {
                    title: "Restore auth session".into(),
                    command: "codex".into(),
                    args: vec!["resume".into(), "session-1".into()],
                },
                tab_id: None,
            },
            None,
        )
        .unwrap();

    let [added] = outcome.panes.added.as_slice() else {
        panic!("expected one added pane: {:?}", outcome.panes.added);
    };
    let tab = project(&workspace).layout.tab(&added.tab_id).unwrap();
    let pane = tab.layout.find_pane(&added.pane_id).unwrap();
    assert_eq!(tab.title, "Restore auth session");
    assert_eq!(pane.command, "codex");
    assert_eq!(pane.args, ["resume", "session-1"]);
    assert_eq!(pane.kind, PaneKind::Agent);
    assert_eq!(pane.execution_mode, TerminalExecutionMode::Command);
    assert!(pane.notify_on_exit);
}

#[test]
fn caller_chosen_ids_are_used_and_duplicates_change_nothing() {
    let mut workspace = migrated();
    let create = |tab_id: &str| WorkspaceIntent::CreateTab {
        project_id: project_id(),
        tab: NewTab::Shell {
            command: String::new(),
        },
        tab_id: Some(tab_id.into()),
    };
    let split = |new_pane_id: &str| WorkspaceIntent::SplitPane {
        project_id: project_id(),
        tab_id: "cli-tab".into(),
        pane_id: "shell".into(),
        direction: SplitDirection::Vertical,
        command: String::new(),
        new_pane_id: Some(new_pane_id.into()),
    };

    let created = workspace.apply(create("cli-tab"), None).unwrap();
    assert_eq!(created.panes.added, [pane("cli-tab", "shell")]);
    let split_outcome = workspace.apply(split("cli-pane"), None).unwrap();
    assert_eq!(split_outcome.panes.added, [pane("cli-tab", "cli-pane")]);

    let before = workspace.clone();
    assert!(workspace.apply(create("cli-tab"), None).is_err());
    assert!(workspace.apply(split("cli-pane"), None).is_err());
    assert_eq!(workspace.revision(), before.revision());
    assert_eq!(
        workspace.workspace().persisted_state(),
        before.workspace().persisted_state()
    );
}

#[test]
fn closing_a_project_with_running_panes_requires_confirmation() {
    let mut snapshot = fixture();
    snapshot["workspace"]["opened_projects"][0]["tab_states"][0]["pane_states"][0]["process_state"] =
        json!("Running");
    let mut workspace = migrate(snapshot).unwrap();
    let before = workspace.state();

    let error = workspace
        .apply(
            WorkspaceIntent::CloseProject {
                project_id: project_id(),
                confirmed_running: false,
            },
            None,
        )
        .unwrap_err();
    assert_eq!(error, IntentError::RunningProcesses);
    assert_eq!(workspace.state(), before);

    let outcome = workspace
        .apply(
            WorkspaceIntent::CloseProject {
                project_id: project_id(),
                confirmed_running: true,
            },
            None,
        )
        .unwrap();

    assert_eq!(outcome.panes.removed.len(), 4);
    assert!(workspace.workspace().opened_projects().is_empty());
    assert!(workspace.editor().session(&project_id()).is_none());
}

#[test]
fn open_project_creates_its_work_area_and_reports_its_panes() {
    let mut workspace = HostWorkspace::new();
    let other = ProjectId::new("other");

    let outcome = workspace
        .apply(
            WorkspaceIntent::OpenProject {
                descriptor: ProjectDescriptor::new(
                    other.clone(),
                    ProjectLocation::local("/host/projects/other"),
                ),
                layout: single_pane_layout("main", "shell"),
                panel: ProjectPanel {
                    visible: false,
                    width: 280.0,
                },
            },
            None,
        )
        .unwrap();

    assert_eq!(outcome.revision, 1);
    assert_eq!(
        outcome.panes.added,
        [PaneAddress {
            project_id: other.clone(),
            tab_id: "main".into(),
            pane_id: "shell".into(),
        }]
    );
    let session = workspace.editor().session(&other).unwrap();
    assert_eq!(
        session.active_work_item(),
        Some(&WorkItemId::Terminal("main".into()))
    );
    assert!(!session.project_panel_visible());
    assert_eq!(session.project_panel_width(), 280.0);
}

#[test]
fn move_item_requires_the_group_that_holds_it() {
    let mut workspace = migrated();
    let dev = WorkItemId::Terminal("dev".into());
    let move_dev = |source| WorkspaceIntent::MoveItem {
        project_id: project_id(),
        item: dev.clone(),
        source_group: TabGroupId::new(source),
        target_group: TabGroupId::new(2),
        placement: WorkAreaDropPlacement::TabIndex(0),
    };

    assert_eq!(
        workspace.apply(move_dev(2), None).unwrap_err(),
        IntentError::WorkItemNotFound("fixture-project".into())
    );

    workspace.apply(move_dev(1), None).unwrap();

    let group = session(&workspace).group_items_containing(&dev).unwrap();
    assert!(group.contains(&WorkItemId::File(main_rs())));
}

#[test]
fn closing_the_active_file_falls_back_to_a_terminal_and_selects_its_tab() {
    let mut workspace = migrated();

    workspace
        .apply(
            WorkspaceIntent::CloseFile {
                document: main_rs(),
            },
            None,
        )
        .unwrap();

    assert!(session(&workspace).file_ids().is_empty());
    assert!(matches!(
        session(&workspace).active_work_item(),
        Some(WorkItemId::Terminal(_))
    ));
    assert_selection_consistent(&workspace);
    assert_eq!(
        workspace
            .apply(
                WorkspaceIntent::CloseFile {
                    document: main_rs()
                },
                None
            )
            .unwrap_err(),
        IntentError::WorkItemNotFound("fixture-project".into())
    );
}

#[test]
fn relocating_a_file_must_stay_inside_the_target_project() {
    let mut workspace = migrated();
    let outside = DocumentId {
        project_id: project_id(),
        canonical_path: "/elsewhere/main.rs".into(),
    };

    assert_eq!(
        workspace
            .apply(
                WorkspaceIntent::RelocateFile {
                    from: main_rs(),
                    to: outside,
                },
                None,
            )
            .unwrap_err(),
        IntentError::FileOutsideProject("/elsewhere/main.rs".into())
    );

    let renamed = DocumentId {
        project_id: project_id(),
        canonical_path: PathBuf::from(ROOT).join("src/lib.rs"),
    };
    workspace
        .apply(
            WorkspaceIntent::RelocateFile {
                from: main_rs(),
                to: renamed.clone(),
            },
            None,
        )
        .unwrap();
    assert_eq!(
        session(&workspace).file_ids(),
        std::slice::from_ref(&renamed)
    );
    assert_eq!(
        session(&workspace).active_work_item(),
        Some(&WorkItemId::File(renamed))
    );
}

#[test]
fn resizing_an_unknown_work_area_split_is_rejected() {
    let mut workspace = migrated();
    let resize = |id: u64| WorkspaceIntent::ResizeWorkAreaSplit {
        project_id: project_id(),
        split_id: serde_json::from_value(json!(id)).unwrap(),
        delta: 0.1,
    };

    assert_eq!(
        workspace.apply(resize(99), None).unwrap_err(),
        IntentError::SplitNotFound(99)
    );
    workspace.apply(resize(1), None).unwrap();
    assert_eq!(workspace.revision(), 8);
}

#[test]
fn recorded_pane_runtime_never_changes_the_host_layout() {
    let mut host = migrated();
    let mut observed = host.workspace().clone();
    observed
        .mark_pane_running(&project_id(), "dev", "server")
        .unwrap();
    observed.select_project(&project_id()).unwrap();
    observed.close_tabs(&["agent".to_string()]).unwrap();
    let layout_before = project(&host).layout.clone();

    host.record_pane_runtime(&observed, 8);

    assert_eq!(host.revision(), 8);
    assert_eq!(project(&host).layout, layout_before);
    let process_state = |tab_id: &str, pane_id: &str| {
        project(&host)
            .tab_state(tab_id)
            .and_then(|tab| tab.pane_states.iter().find(|pane| pane.pane_id == pane_id))
            .map(|pane| pane.process_state)
            .unwrap()
    };
    assert_eq!(process_state("dev", "server"), PaneProcessState::Running);
    assert_eq!(process_state("agent", "codex"), PaneProcessState::Idle);
}
