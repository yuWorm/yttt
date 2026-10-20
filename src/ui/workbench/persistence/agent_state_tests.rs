use super::*;
use yttt_agent_core::{
    AgentBoardCommand, AgentBoardMessage, AgentInstanceId, AgentMessageRole, AgentQuestion,
    AgentQuestionAnswer, AgentQuestionOption, AgentQuestionRecord, AgentQuestionRequest,
    AgentQuestionState, AgentReducer, AgentSnapshot, ProviderId,
};

fn history() -> AgentSnapshot {
    let mut reducer = AgentReducer::new(
        AgentInstanceId::new("history-agent").unwrap(),
        ProviderId::from_static("omp"),
        42,
    );
    reducer.process_starting(1, 42);
    reducer.process_started(1, 42);
    let mut agent = reducer.snapshot().clone();
    agent.board.messages = (0..80)
        .map(|index| AgentBoardMessage {
            id: format!("message-{index}"),
            execution_id: "run".into(),
            role: AgentMessageRole::Assistant,
            text: "x".repeat(16 * 1024),
            timestamp: index + 1,
            truncated: false,
        })
        .collect();
    agent.board.questions.push(AgentQuestionRecord {
        request: AgentQuestionRequest {
            id: "pending-question".into(),
            execution_id: "run".into(),
            tool_call_id: "ask".into(),
            questions: vec![AgentQuestion {
                id: "choice".into(),
                question: "Continue?".into(),
                header: None,
                options: vec![AgentQuestionOption {
                    label: "Yes".into(),
                    description: None,
                    preview: None,
                }],
                multi: false,
                recommended: None,
            }],
            answerable: true,
        },
        state: AgentQuestionState::Pending,
        answers: Vec::new(),
        answer_source: None,
        created_at: 42,
        updated_at: 42,
    });
    agent
}

fn workspace(root: &Path, agent: AgentSnapshot) -> Workspace {
    let mut workspace = Workspace::new();
    let project = workspace
        .open_project(
            ProjectDescriptor::new(
                ProjectId::new("history-project"),
                ProjectLocation::local(root.to_path_buf()),
            ),
            dev_fixture_layout(),
        )
        .unwrap();
    workspace
        .mark_pane_running(&project, "dev", "shell")
        .unwrap();
    workspace
        .record_agent_snapshot(&project, "dev", "shell", agent)
        .unwrap();
    workspace
}

#[gpui::test]
fn large_agent_history_saves_through_client_and_restores_without_live_answer_channels(
    cx: &mut gpui::TestAppContext,
) {
    use crate::ui::terminal::pane::recovery_tests::RecoveryHost;
    cx.update(gpui_component::init);
    cx.background_executor.allow_parking();
    cx.skip_drawing();
    let host = RecoveryHost::start();
    let runtime = host.client("agent-history");
    let lease = runtime.claim_workspace_view(false).unwrap();
    let id = lease.id().clone();
    let agent = history();
    assert!(serde_json::to_vec(&agent).unwrap().len() > 1024 * 1024);
    let answer = AgentBoardCommand::AnswerQuestion {
        request_id: "pending-question".into(),
        answers: vec![AgentQuestionAnswer {
            question_id: "choice".into(),
            selected_options: vec!["Yes".into()],
            custom_input: None,
        }],
    };
    assert!(agent.board.clone().command(answer.clone(), 43).is_ok());
    let paths = AppConfigPaths::from_config_dir(host.root.path().join("ui-config"));
    let state = workspace(host.root.path(), agent.clone());
    let (view, cx) = cx.add_window_view(|_, _| {
        WorkbenchView::with_workspace_for_test_and_config_paths(state, paths)
    });
    view.update_in(cx, |root, window, cx| {
        root.terminal.host_runtime = Some(runtime.clone());
        root.workspace_persistence.view = Some(lease);
        root.workspace_persistence.revision = Some(0);
        root.workspace_persistence.host_epoch = remote_host_epoch(&runtime);
        root.workspace_persistence.control_context =
            runtime.control_status().map(|status| status.context);
        root.workspace_persistence.mode = WorkspacePersistenceMode::Active;
        root.commit_remote_workspace_if_changed(runtime.clone(), id.clone(), window, cx);
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        cx.run_until_parked();
        if view.read_with(cx, |root, _| root.workspace_persistence.revision == Some(1)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "save failed: {:?}",
            view.read_with(cx, |root, _| root.workspace_persistence.last_error.clone())
        );
        std::thread::sleep(Duration::from_millis(10));
    }
    let WorkspaceResponse::Opened {
        snapshot, revision, ..
    } = runtime
        .workspace_request(WorkspaceRequest::Open {
            workspace_id: id.clone(),
        })
        .unwrap()
    else {
        panic!("workspace")
    };
    let prepared = prepare_remote_restore(&runtime, &id, snapshot.into_value(), revision).unwrap();
    let pane = &prepared.snapshot.workspace.opened_projects[0]
        .tab_states
        .iter()
        .find(|tab| tab.tab_id == "dev")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "shell")
        .unwrap();
    assert_eq!(
        pane.process_state,
        crate::model::workspace::PaneProcessState::Restoring
    );
    let restored = pane.agent_snapshot.as_ref().unwrap();
    assert_eq!(restored.board.messages, agent.board.messages);
    assert_eq!(
        restored.board.questions[0].state,
        AgentQuestionState::Unavailable
    );
    assert!(restored.board.clone().command(answer, 44).is_err());
}

#[gpui::test]
fn unpublished_agent_history_is_self_contained_and_rejects_missing_or_corrupt_recovery_bodies(
    cx: &mut gpui::TestAppContext,
) {
    let temp = tempfile::tempdir().unwrap();
    let agent = history();
    let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
    let root = cx.new(|_| {
        WorkbenchView::with_workspace_for_test_and_config_paths(
            workspace(temp.path(), agent.clone()),
            paths,
        )
    });
    let (snapshot, uploads) = root.update(cx, |root, cx| {
        root.build_workspace_snapshot(cx, None).unwrap()
    });
    let mut recovery = RecoverableWorkspaceDraft {
        schema_version: RECOVERABLE_WORKSPACE_DRAFT_SCHEMA_VERSION,
        scope: RecoverableWorkspaceScope {
            profile_id: "device".into(),
            environment_id: "host".into(),
            workspace_id: WorkspaceId::new("workspace").unwrap(),
        },
        host_epoch: Some(1),
        snapshot,
        drafts: Vec::new(),
        agent_states: uploads
            .agent_states
            .into_iter()
            .map(|(reference, content)| RecoverableAgentState {
                reference,
                content: content.as_ref().clone(),
            })
            .collect(),
        pending_settings: None,
    };
    let path = temp.path().join("recovery.json");
    save_recoverable_workspace_draft(&path, &recovery).unwrap();
    let recovered = load_recoverable_workspace_draft(&path).unwrap();
    let materialized = materialize_recovery_snapshot(
        recovered.snapshot,
        WorkspaceUploads {
            drafts: Vec::new(),
            agent_states: recovered
                .agent_states
                .into_iter()
                .map(|body| (body.reference, Arc::new(body.content)))
                .collect(),
        },
    )
    .unwrap();
    let restored = RemoteWorkspaceSnapshot::from_value(materialized).unwrap();
    let pane = restored.workspace.opened_projects[0]
        .tab_states
        .iter()
        .find(|tab| tab.tab_id == "dev")
        .unwrap()
        .pane_states
        .iter()
        .find(|pane| pane.pane_id == "shell")
        .unwrap();
    assert_eq!(pane.agent_snapshot.as_ref().unwrap(), &agent);
    let body = recovery.agent_states.remove(0);
    save_recoverable_workspace_draft(&path, &recovery).unwrap();
    assert!(load_recoverable_workspace_draft(&path).is_err());
    recovery.agent_states.push(body);
    recovery.agent_states[0].content[0] ^= 1;
    save_recoverable_workspace_draft(&path, &recovery).unwrap();
    assert!(load_recoverable_workspace_draft(&path).is_err());
}
