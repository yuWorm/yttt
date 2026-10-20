use super::publication_tests::{register, setup};
use super::*;
use std::slice::from_ref;

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

fn agent_ref(pane: &str, content: &[u8]) -> AgentStateRef {
    AgentStateRef::for_content(
        yttt_core::model::ids::ProjectId::new("project"),
        "tab".into(),
        pane.into(),
        content,
    )
}

fn commit_agents(
    service: &WorkspaceService,
    client: &ClientInstanceId,
    id: &WorkspaceId,
    revision: u64,
    references: &[AgentStateRef],
) -> Result<WorkspaceResponse, ProtocolFailure> {
    service.handle(
        client,
        WorkspaceRequest::Commit {
            workspace_id: id.clone(),
            expected_revision: revision,
            operation_id: WorkspaceOperationId::new(format!("agents-{revision}")).unwrap(),
            snapshot: WorkspaceSnapshot::new(serde_json::json!({
                "layout": "split", "agent_states": references,
            }))
            .unwrap(),
            drafts: Vec::new(),
        },
    )
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

#[test]
fn agent_histories_larger_than_the_manifest_budget_survive_host_restart() {
    let root = tempfile::tempdir().unwrap();
    let (service, client) = setup(root.path());
    let id = register(&service, &client, "history");
    let mut references = Vec::new();
    let mut expected = Vec::new();
    for index in 0..5 {
        let pane = format!("agent-{index}");
        let content = agent_content(&pane, 256);
        let reference = agent_ref(&pane, &content);
        service
            .handle(
                &client,
                WorkspaceRequest::PutAgentState {
                    workspace_id: id.clone(),
                    reference: reference.clone(),
                    content: content.clone(),
                },
            )
            .unwrap();
        references.push(reference);
        expected.push(content);
    }
    assert!(
        expected.iter().map(Vec::len).sum::<usize>()
            > yttt_protocol::workspace::MAX_WORKSPACE_MANIFEST_BYTES
    );
    commit_agents(&service, &client, &id, 0, &references).unwrap();
    drop(service);
    let (service, client) = setup(root.path());
    let WorkspaceResponse::Opened {
        snapshot, revision, ..
    } = service
        .handle(
            &client,
            WorkspaceRequest::Open {
                workspace_id: id.clone(),
            },
        )
        .unwrap()
    else {
        panic!("workspace")
    };
    assert_eq!(revision, 1);
    assert_eq!(
        agent_state_references(snapshot.as_value()).unwrap(),
        references
    );
    for (reference, expected) in references.iter().zip(expected) {
        assert_eq!(
            read_agent(&service, &client, &id, reference).unwrap(),
            expected
        );
    }
}

#[test]
fn missing_foreign_or_mismatched_agent_objects_cannot_replace_published_history() {
    let root = tempfile::tempdir().unwrap();
    let (service, client) = setup(root.path());
    let id = register(&service, &client, "owner");
    let other = register(&service, &client, "other");
    let content = agent_content("original", 80);
    let original = agent_ref("pane", &content);
    assert!(commit_agents(&service, &client, &id, 0, from_ref(&original)).is_err());
    assert!(
        service
            .handle(
                &client,
                WorkspaceRequest::PutAgentState {
                    workspace_id: id.clone(),
                    reference: original.clone(),
                    content: b"wrong".to_vec(),
                }
            )
            .is_err()
    );
    service
        .handle(
            &client,
            WorkspaceRequest::PutAgentState {
                workspace_id: id.clone(),
                reference: original.clone(),
                content: content.clone(),
            },
        )
        .unwrap();
    assert!(commit_agents(&service, &client, &other, 0, from_ref(&original)).is_err());
    assert_eq!(
        read_agent(&service, &client, &id, &original)
            .unwrap_err()
            .code,
        FailureCode::NotFound
    );
    commit_agents(&service, &client, &id, 0, from_ref(&original)).unwrap();
    let replacement = agent_ref("pane", &agent_content("replacement", 80));
    assert!(commit_agents(&service, &client, &id, 1, &[replacement]).is_err());
    assert_eq!(
        read_agent(&service, &client, &id, &original).unwrap(),
        content
    );
    assert!(matches!(
        service
            .handle(&client, WorkspaceRequest::Open { workspace_id: id })
            .unwrap(),
        WorkspaceResponse::Opened { revision: 1, .. }
    ));
}

#[test]
fn failed_agent_publication_and_restart_keep_the_previous_generation() {
    let root = tempfile::tempdir().unwrap();
    let (service, client) = setup(root.path());
    let id = register(&service, &client, "atomic");
    let original_content = agent_content("original", 80);
    let original = agent_ref("pane", &original_content);
    service
        .handle(
            &client,
            WorkspaceRequest::PutAgentState {
                workspace_id: id.clone(),
                reference: original.clone(),
                content: original_content.clone(),
            },
        )
        .unwrap();
    commit_agents(&service, &client, &id, 0, from_ref(&original)).unwrap();
    let replacement_content = agent_content("replacement", 80);
    let replacement = agent_ref("pane", &replacement_content);
    service
        .handle(
            &client,
            WorkspaceRequest::PutAgentState {
                workspace_id: id.clone(),
                reference: replacement.clone(),
                content: replacement_content,
            },
        )
        .unwrap();
    let manifest = root.path().join("workspaces/atomic.json");
    let backup = root.path().join("previous-manifest");
    fs::rename(&manifest, &backup).unwrap();
    fs::create_dir(&manifest).unwrap();
    assert!(commit_agents(&service, &client, &id, 1, from_ref(&replacement)).is_err());
    assert_eq!(
        read_agent(&service, &client, &id, &original).unwrap(),
        original_content
    );
    fs::remove_dir(&manifest).unwrap();
    fs::rename(&backup, &manifest).unwrap();
    drop(service);
    let (service, client) = setup(root.path());
    assert_eq!(
        read_agent(&service, &client, &id, &original).unwrap(),
        original_content
    );
    assert_eq!(
        read_agent(&service, &client, &id, &replacement)
            .unwrap_err()
            .code,
        FailureCode::NotFound
    );
    assert!(service.agent_states.read(&id, &replacement).is_err());
}

#[test]
fn legacy_agent_history_migrates_atomically_without_losing_acceptance_or_revision() {
    let root = tempfile::tempdir().unwrap();
    fs::create_dir(root.path().join("workspaces")).unwrap();
    let content = agent_content("legacy", 80);
    let agent: serde_json::Value = serde_json::from_slice(&content).unwrap();
    let legacy = serde_json::json!({
        "schema_version": 2, "revision": 7, "restore_on_startup": true,
        "snapshot": {
            "schema_version": 1,
            "workspace": {"opened_projects": [{
                "id": "project", "tab_states": [{"tab_id": "tab", "pane_states": [{
                    "pane_id": "pane", "agent_snapshot": agent,
                }]}],
            }]},
        },
    });
    let manifest = root.path().join("workspaces/legacy.json");
    let bytes = serde_json::to_vec(&legacy).unwrap();
    fs::write(&manifest, &bytes).unwrap();
    let object_root = root.path().join("agent-states");
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
    let WorkspaceResponse::Opened {
        snapshot, revision, ..
    } = service
        .handle(
            &client,
            WorkspaceRequest::Open {
                workspace_id: id.clone(),
            },
        )
        .unwrap()
    else {
        panic!("workspace")
    };
    assert_eq!(revision, 7);
    assert!(snapshot.as_value()["workspace"]["opened_projects"][0]["tab_states"][0]
        ["pane_states"][0]["agent_snapshot"].is_null());
    let reference = agent_state_references(snapshot.as_value())
        .unwrap()
        .remove(0);
    assert_eq!(
        read_agent(&service, &client, &id, &reference).unwrap(),
        content
    );
    drop(service);
    let (service, client) = setup(root.path());
    assert_eq!(
        read_agent(&service, &client, &id, &reference).unwrap(),
        content
    );
}
