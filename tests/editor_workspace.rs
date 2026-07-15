use std::path::{Path, PathBuf};

use yttt::{
    model::ids::ProjectId,
    ui::editor::{ProjectEditorWorkspaceState, ProjectWorkItemSession, WorkItemId},
};

#[test]
fn opening_the_same_file_twice_reuses_one_work_item() {
    let project_id = ProjectId::new("project-a");
    let mut session = ProjectWorkItemSession::new(
        project_id.clone(),
        "/project-a",
        Some("dev".to_string()),
        true,
        280.0,
    );
    let terminals = vec!["dev".to_string(), "logs".to_string()];

    let first = session.open_file("/project-a/src/main.rs");
    let second = session.open_file("/project-a/src/main.rs");

    assert_eq!(first, second);
    assert_eq!(session.file_ids(), &[first.clone()]);
    assert_eq!(
        session.ordered_items(&terminals),
        vec![
            WorkItemId::Terminal("dev".to_string()),
            WorkItemId::Terminal("logs".to_string()),
            WorkItemId::File(first.clone()),
        ]
    );
    assert_eq!(session.active_work_item(), Some(&WorkItemId::File(first)));
}

#[test]
fn selecting_files_keeps_terminal_ids_authoritative() {
    let project_id = ProjectId::new("project-a");
    let mut session = ProjectWorkItemSession::new(
        project_id,
        "/project-a",
        Some("dev".to_string()),
        true,
        280.0,
    );
    let terminals = vec!["dev".to_string(), "logs".to_string()];
    let file = session.open_file("/project-a/src/main.rs");

    assert!(session.select_work_item(WorkItemId::File(file), &terminals));

    assert_eq!(terminals, ["dev", "logs"]);
    assert_eq!(
        session.ordered_items(&terminals)[..2],
        [
            WorkItemId::Terminal("dev".to_string()),
            WorkItemId::Terminal("logs".to_string())
        ]
    );
}

#[test]
fn dragging_work_items_persists_one_mixed_tab_order() {
    let mut session = ProjectWorkItemSession::new(
        ProjectId::new("project-a"),
        "/project-a",
        Some("dev".to_string()),
        true,
        280.0,
    );
    let terminals = vec!["dev".to_string(), "logs".to_string()];
    let file = session.open_file("/project-a/src/main.rs");
    let file_item = WorkItemId::File(file);

    assert!(session.move_work_item(&file_item, 0, &terminals));
    assert_eq!(
        session.ordered_items(&terminals),
        vec![
            file_item.clone(),
            WorkItemId::Terminal("dev".to_string()),
            WorkItemId::Terminal("logs".to_string()),
        ]
    );

    let dev = WorkItemId::Terminal("dev".to_string());
    assert!(session.move_work_item(&dev, 2, &terminals));
    assert_eq!(
        session.ordered_items(&terminals),
        vec![
            file_item.clone(),
            WorkItemId::Terminal("logs".to_string()),
            dev.clone(),
        ]
    );

    session.reconcile_work_item_order(&["dev".to_string()]);
    assert_eq!(
        session.ordered_items(&["dev".to_string(), "logs".to_string()]),
        vec![file_item, dev, WorkItemId::Terminal("logs".to_string()),]
    );
}

#[test]
fn projects_keep_independent_file_tree_panel_and_active_file_state() {
    let mut workspace = ProjectEditorWorkspaceState::default();
    let alpha = ProjectId::new("alpha");
    let beta = ProjectId::new("beta");
    assert!(workspace.open_project(
        alpha.clone(),
        "/projects/alpha",
        Some("dev".to_string()),
        true,
        280.0,
    ));
    assert!(workspace.open_project(
        beta.clone(),
        "/projects/beta",
        Some("shell".to_string()),
        false,
        340.0,
    ));

    let alpha_file = workspace
        .session_mut(&alpha)
        .unwrap()
        .open_file("/projects/alpha/src/main.rs");
    workspace
        .session_mut(&alpha)
        .unwrap()
        .file_tree_mut()
        .request_expand(Path::new("src"));
    let beta_file = workspace
        .session_mut(&beta)
        .unwrap()
        .open_file("/projects/beta/README.md");

    let alpha_session = workspace.session(&alpha).unwrap();
    assert_eq!(
        alpha_session.active_work_item(),
        Some(&WorkItemId::File(alpha_file))
    );
    assert!(alpha_session.file_tree().is_expanded(Path::new("src")));
    assert!(alpha_session.project_panel_visible());
    assert_eq!(alpha_session.project_panel_width(), 280.0);

    let beta_session = workspace.session(&beta).unwrap();
    assert_eq!(
        beta_session.active_work_item(),
        Some(&WorkItemId::File(beta_file))
    );
    assert!(!beta_session.file_tree().is_expanded(Path::new("src")));
    assert!(!beta_session.project_panel_visible());
    assert_eq!(beta_session.project_panel_width(), 340.0);

    assert!(workspace.close_project(&alpha).is_some());
    assert!(workspace.session(&alpha).is_none());
    assert!(workspace.session(&beta).is_some());
}

#[test]
fn closing_active_file_selects_right_then_left_neighbor() {
    let mut session = ProjectWorkItemSession::new(
        ProjectId::new("project-a"),
        "/project-a",
        Some("dev".to_string()),
        true,
        280.0,
    );
    let terminals = vec!["dev".to_string(), "logs".to_string()];
    let first = session.open_file("/project-a/first.rs");
    let middle = session.open_file("/project-a/middle.rs");
    let last = session.open_file("/project-a/last.rs");

    assert!(session.select_work_item(WorkItemId::File(middle.clone()), &terminals));
    assert_eq!(
        session.close_file(&middle, &terminals),
        Some(WorkItemId::File(last.clone()))
    );
    assert_eq!(
        session.close_file(&last, &terminals),
        Some(WorkItemId::File(first.clone()))
    );
    assert_eq!(
        session.close_file(&first, &terminals),
        Some(WorkItemId::Terminal("logs".to_string()))
    );
}

#[test]
fn relative_navigation_wraps_across_terminal_and_file_items() {
    let mut session = ProjectWorkItemSession::new(
        ProjectId::new("project-a"),
        "/project-a",
        Some("dev".to_string()),
        true,
        280.0,
    );
    let terminals = vec!["dev".to_string(), "logs".to_string()];
    let file = session.open_file(PathBuf::from("/project-a/main.rs"));

    assert_eq!(
        session.select_next(&terminals),
        Some(WorkItemId::Terminal("dev".to_string()))
    );
    assert_eq!(
        session.select_previous(&terminals),
        Some(WorkItemId::File(file))
    );
}

#[test]
fn relocating_file_in_place_rekeys_active_work_item() {
    let project_id = ProjectId::new("project-a");
    let mut workspace = ProjectEditorWorkspaceState::default();
    assert!(workspace.open_project(project_id.clone(), "/project-a", None, true, 280.0,));
    let old = workspace
        .session_mut(&project_id)
        .unwrap()
        .open_file("/project-a/src/old.rs");
    let mut new = old.clone();
    new.canonical_path = PathBuf::from("/project-a/src/new.rs");

    assert!(workspace.relocate_file(&old, new.clone()));

    let session = workspace.session(&project_id).unwrap();
    assert_eq!(session.file_ids(), &[new.clone()]);
    assert_eq!(session.active_work_item(), Some(&WorkItemId::File(new)));
}

#[test]
fn relocating_file_between_projects_moves_session_ownership() {
    let source_project_id = ProjectId::new("project-a");
    let destination_project_id = ProjectId::new("project-b");
    let mut workspace = ProjectEditorWorkspaceState::default();
    assert!(workspace.open_project(source_project_id.clone(), "/project-a", None, true, 280.0,));
    assert!(workspace.open_project(
        destination_project_id.clone(),
        "/project-b",
        None,
        true,
        280.0,
    ));
    let old = workspace
        .session_mut(&source_project_id)
        .unwrap()
        .open_file("/project-a/src/file.rs");
    let new = yttt::ui::editor::DocumentId {
        project_id: destination_project_id.clone(),
        canonical_path: PathBuf::from("/project-b/file.rs"),
    };

    assert!(workspace.relocate_file(&old, new.clone()));

    assert!(
        workspace
            .session(&source_project_id)
            .unwrap()
            .file_ids()
            .is_empty()
    );
    let destination = workspace.session(&destination_project_id).unwrap();
    assert_eq!(destination.file_ids(), &[new.clone()]);
    assert_eq!(destination.active_work_item(), Some(&WorkItemId::File(new)));
}
