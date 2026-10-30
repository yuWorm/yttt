use super::publication_tests::{register, setup};
use super::*;

fn agent_content(label: &str, messages: usize) -> Vec<u8> {
    use yttt_agent_core::{
        AgentBoardMessage, AgentBoardTask, AgentInstanceId, AgentMessageRole, AgentPlanItem,
        AgentPlanStatus, AgentReducer, ProviderId,
    };
    let mut agent = AgentReducer::new(
        AgentInstanceId::new(label).unwrap(),
        ProviderId::from_static("omp"),
        42,
    )
    .snapshot()
    .clone();
    agent.board.messages = (0..messages)
        .map(|index| AgentBoardMessage {
            id: format!("message-{index}"),
            execution_id: "run".into(),
            role: AgentMessageRole::Assistant,
            text: format!("{label}:{}", "x".repeat(16 * 1024 - label.len() - 1)),
            timestamp: index as u64 + 1,
            truncated: false,
        })
        .collect();
    agent.board.tasks.push(AgentBoardTask {
        item: AgentPlanItem {
            id: "accepted-task".into(),
            title: "Keep human acceptance".into(),
            phase: None,
            status: AgentPlanStatus::Completed,
            blocker: None,
        },
        execution_id: "run".into(),
        created_at: 1,
        updated_at: 2,
        accepted_at: Some(3),
        retired: false,
    });
    serde_json::to_vec(&agent).unwrap()
}

/// A workspace whose project has one Agent pane in each of `tabs` tabs.
fn agent_workspace(
    root: &Path,
    service: &WorkspaceService,
    client: &ClientInstanceId,
    name: &str,
    tabs: usize,
) -> WorkspaceId {
    let id = register(service, client, name);
    let project_root = root.join("project");
    fs::create_dir_all(&project_root).unwrap();
    let layout = serde_json::from_value(serde_json::json!({
        "project": { "name": "Agents" },
        "tabs": (0..tabs).map(|index| serde_json::json!({
            "id": format!("tab-{index}"),
            "title": "Agent",
            "layout": { "type": "pane", "id": "agent", "title": "Agent", "command": "omp", "kind": "agent" },
        })).collect::<Vec<_>>(),
    }))
    .unwrap();
    service
        .apply(
            id.clone(),
            WorkspaceIntent::OpenProject {
                descriptor: yttt_core::model::project::ProjectDescriptor::new(
                    yttt_core::model::ids::ProjectId::new("project"),
                    yttt_core::model::project::ProjectLocation::local(&project_root),
                ),
                layout,
                panel: yttt_core::model::host_workspace::ProjectPanel {
                    visible: true,
                    width: 240.0,
                },
            },
            None,
        )
        .unwrap();
    id
}

fn agent_pane(tab: usize) -> PaneAddress {
    PaneAddress {
        project_id: yttt_core::model::ids::ProjectId::new("project"),
        tab_id: format!("tab-{tab}"),
        pane_id: "agent".into(),
    }
}

fn record_agent(service: &WorkspaceService, id: &WorkspaceId, tab: usize, content: &[u8]) {
    let snapshot: AgentSnapshot = serde_json::from_slice(content).unwrap();
    service.update_agent_snapshot(id, &agent_pane(tab), |_| Some(snapshot));
}

fn read_agent(
    service: &WorkspaceService,
    client: &ClientInstanceId,
    id: &WorkspaceId,
    reference: &AgentStateRef,
) -> Result<Vec<u8>, ProtocolFailure> {
    match service.handle(
        client,
        WorkspaceRequest::GetAgentState {
            workspace_id: id.clone(),
            reference: reference.clone(),
        },
    )? {
        WorkspaceResponse::AgentState { content, .. } => Ok(content),
        response => panic!("unexpected response: {response:?}"),
    }
}

fn stored_snapshots(service: &WorkspaceService, id: &WorkspaceId) -> Vec<(PaneAddress, Vec<u8>)> {
    let state = service.state.lock();
    state.workspaces[id]
        .layout
        .agent_snapshots()
        .into_iter()
        .map(|(address, snapshot)| (address, serde_json::to_vec(snapshot).unwrap()))
        .collect()
}

fn opened_agent_states(
    service: &WorkspaceService,
    client: &ClientInstanceId,
    id: &WorkspaceId,
) -> (u64, Vec<AgentStateRef>) {
    let WorkspaceResponse::Opened {
        layout,
        agent_states,
        ..
    } = service
        .handle(
            client,
            WorkspaceRequest::Open {
                workspace_id: id.clone(),
            },
        )
        .unwrap()
    else {
        panic!("workspace")
    };
    (layout.revision, agent_states)
}

#[test]
fn host_recorded_agent_histories_survive_host_restart() {
    let root = tempfile::tempdir().unwrap();
    let (service, client) = setup(root.path());
    let id = agent_workspace(root.path(), &service, &client, "history", 5);
    let expected = (0..5)
        .map(|tab| agent_content(&format!("agent-{tab}"), 256))
        .collect::<Vec<_>>();
    for (tab, content) in expected.iter().enumerate() {
        record_agent(&service, &id, tab, content);
    }
    service.flush_agent_states();
    drop(service);

    let (service, client) = setup(root.path());
    let (_, references) = opened_agent_states(&service, &client, &id);
    assert_eq!(references.len(), 5);
    for reference in &references {
        let tab = reference
            .tab_id
            .trim_start_matches("tab-")
            .parse::<usize>()
            .unwrap();
        assert_eq!(
            read_agent(&service, &client, &id, reference).unwrap(),
            expected[tab]
        );
    }
    let restored = stored_snapshots(&service, &id);
    assert_eq!(
        restored.len(),
        5,
        "the restarted Host puts each history back on its pane"
    );
    for (address, content) in restored {
        let tab = address
            .tab_id
            .trim_start_matches("tab-")
            .parse::<usize>()
            .unwrap();
        assert_eq!(
            serde_json::from_slice::<AgentSnapshot>(&content).unwrap(),
            serde_json::from_slice::<AgentSnapshot>(&expected[tab]).unwrap()
        );
    }
}

#[test]
fn a_failed_agent_store_keeps_the_previous_generation_and_retries() {
    let root = tempfile::tempdir().unwrap();
    let (service, client) = setup(root.path());
    let id = agent_workspace(root.path(), &service, &client, "atomic", 1);
    let stored =
        |service: &WorkspaceService| service.state.lock().workspaces[&id].agent_states.clone();
    let original = agent_content("original", 80);
    record_agent(&service, &id, 0, &original);
    service.flush_agent_states();
    let original_reference = stored(&service).remove(0);

    let replacement = agent_content("replacement", 80);
    record_agent(&service, &id, 0, &replacement);
    let manifest = root.path().join("workspaces/atomic.json");
    let backup = root.path().join("previous-manifest");
    fs::rename(&manifest, &backup).unwrap();
    fs::create_dir(&manifest).unwrap();
    service.flush_agent_states();
    assert_eq!(
        read_agent(&service, &client, &id, &original_reference).unwrap(),
        original,
        "a store that could not be published keeps the published history"
    );

    fs::remove_dir(&manifest).unwrap();
    fs::rename(&backup, &manifest).unwrap();
    service.flush_agent_states();
    let references = stored(&service);
    assert_eq!(references.len(), 1);
    assert_ne!(
        references[0], original_reference,
        "the next flush stores the replacement"
    );
    assert_eq!(
        read_agent(&service, &client, &id, &references[0]).unwrap(),
        replacement
    );
    drop(service);

    let (service, client) = setup(root.path());
    assert_eq!(
        read_agent(&service, &client, &id, &references[0]).unwrap(),
        replacement
    );
    assert_eq!(
        read_agent(&service, &client, &id, &original_reference)
            .unwrap_err()
            .code,
        FailureCode::NotFound
    );
}

#[test]
fn legacy_agent_history_migrates_atomically_without_losing_acceptance_or_revision() {
    let root = tempfile::tempdir().unwrap();
    // A schema 2 record carrying the layout a Client published, with the
    // Agent snapshot inline on its pane.
    let layout = {
        let (service, client) = setup(root.path());
        let id = agent_workspace(root.path(), &service, &client, "legacy", 1);
        service.state.lock().workspaces[&id].layout.state()
    };
    fs::remove_dir_all(root.path().join("workspaces")).unwrap();
    fs::create_dir(root.path().join("workspaces")).unwrap();
    let content = agent_content("legacy", 80);
    let mut workspace = serde_json::to_value(&layout.workspace).unwrap();
    workspace["opened_projects"][0]["tab_states"][0]["pane_states"][0]["agent_snapshot"] =
        serde_json::from_slice(&content).unwrap();
    let legacy = serde_json::json!({
        "schema_version": 2, "revision": 7, "restore_on_startup": true,
        "snapshot": { "schema_version": 2, "workspace": workspace, "editor": layout.editor },
    });
    let manifest = root.path().join("workspaces/legacy.json");
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&manifest, &bytes).unwrap();
    let object_root = root.path().join("agent-states");
    fs::remove_dir_all(&object_root).ok();
    fs::write(&object_root, b"blocked").unwrap();
    assert!(
        WorkspaceService::new(
            root.path(),
            root.path().join("config"),
            WorkspaceProjectConfig::Project
        )
        .is_err()
    );
    assert_eq!(fs::read(&manifest).unwrap(), bytes);
    fs::remove_file(object_root).unwrap();

    let (service, client) = setup(root.path());
    let id = WorkspaceId::new("legacy").unwrap();
    let (revision, references) = opened_agent_states(&service, &client, &id);
    assert_eq!(revision, 7);
    assert_eq!(references.len(), 1);
    assert_eq!(
        read_agent(&service, &client, &id, &references[0]).unwrap(),
        content
    );
    assert_eq!(stored_snapshots(&service, &id).len(), 1);
    drop(service);
    let (service, client) = setup(root.path());
    assert_eq!(
        read_agent(&service, &client, &id, &references[0]).unwrap(),
        content
    );
}
