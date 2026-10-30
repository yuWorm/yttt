use std::{fs, path::Path, sync::Arc};

use parking_lot::Mutex;
use yttt_core::model::{
    editor::DocumentId,
    host_workspace::{ProjectPanel, WorkspaceIntent},
    ids::{ClientInstanceId, ProjectId},
    project::{ProjectDescriptor, ProjectLocation},
};
use yttt_protocol::{
    FailureCode, ProtocolFailure,
    workspace::{
        DocumentState, DocumentUpdate, DraftBase, DraftContentRevision, DraftRef, WorkspaceId,
        WorkspaceProjectConfig, WorkspaceRequest, WorkspaceResponse,
    },
};

use super::super::{WorkspaceEvent, WorkspaceService};

fn service(root: &Path) -> WorkspaceService {
    WorkspaceService::new(root, root.join("config"), WorkspaceProjectConfig::Project).unwrap()
}

fn workspace() -> WorkspaceId {
    WorkspaceId::new("default").unwrap()
}

fn project_root(root: &Path) -> std::path::PathBuf {
    root.join("project")
}

/// Opens a project with one file open in its work area and returns the file.
fn open_file(host: &WorkspaceService, root: &Path, name: &str) -> DocumentId {
    let project_root = project_root(root);
    fs::create_dir_all(&project_root).unwrap();
    let project_id = ProjectId::new("project");
    if host.pane_addresses(&workspace()).is_empty() {
        let layout = toml::from_str(
            r#"
            [project]
            name = "Documents"

            [[tabs]]
            id = "shell"
            title = "Shell"
            layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }
            "#,
        )
        .unwrap();
        host.apply(
            workspace(),
            WorkspaceIntent::OpenProject {
                descriptor: ProjectDescriptor::new(
                    project_id.clone(),
                    ProjectLocation::local(&project_root),
                ),
                layout,
                panel: ProjectPanel {
                    visible: true,
                    width: 240.0,
                },
            },
            None,
        )
        .unwrap();
    }
    let path = project_root.join(name);
    host.apply(
        workspace(),
        WorkspaceIntent::OpenFile {
            project_id: project_id.clone(),
            path: path.clone(),
        },
        None,
    )
    .unwrap();
    let state = host.state.lock();
    state.workspaces[&workspace()]
        .layout
        .editor()
        .session(&project_id)
        .unwrap()
        .file_ids()
        .iter()
        .find(|document| document.canonical_path == path)
        .unwrap()
        .clone()
}

fn document_response(response: Result<WorkspaceResponse, ProtocolFailure>) -> DocumentState {
    match response.unwrap() {
        WorkspaceResponse::Document(state) => state,
        response => panic!("unexpected response: {response:?}"),
    }
}

fn acquire(
    host: &WorkspaceService,
    client: &ClientInstanceId,
    document: &DocumentId,
    takeover: bool,
) -> DocumentState {
    document_response(host.handle(
        client,
        WorkspaceRequest::AcquireDocument {
            workspace_id: workspace(),
            document: document.clone(),
            takeover,
        },
    ))
}

fn put(
    host: &WorkspaceService,
    client: &ClientInstanceId,
    document: &DocumentId,
    epoch: u64,
    revision: u64,
    content: &str,
) -> Result<WorkspaceResponse, ProtocolFailure> {
    host.handle(
        client,
        WorkspaceRequest::PutDocumentDraft {
            workspace_id: workspace(),
            document: document.clone(),
            epoch,
            revision,
            base: DraftBase::Json {
                metadata: serde_json::json!({ "base": "saved" }),
            },
            content: content.as_bytes().to_vec(),
        },
    )
}

fn release(
    host: &WorkspaceService,
    client: &ClientInstanceId,
    document: &DocumentId,
    epoch: u64,
) -> DocumentState {
    document_response(host.handle(
        client,
        WorkspaceRequest::ReleaseDocument {
            workspace_id: workspace(),
            document: document.clone(),
            epoch,
        },
    ))
}

fn draft_content(
    host: &WorkspaceService,
    reader: &ClientInstanceId,
    document: &DocumentId,
) -> Result<Vec<u8>, ProtocolFailure> {
    match host.handle(
        reader,
        WorkspaceRequest::GetDocumentDraft {
            workspace_id: workspace(),
            document: document.clone(),
        },
    )? {
        WorkspaceResponse::DocumentDraft { content, .. } => Ok(content),
        response => panic!("unexpected response: {response:?}"),
    }
}

fn opened_documents(host: &WorkspaceService, reader: &ClientInstanceId) -> Vec<DocumentState> {
    match host
        .handle(
            reader,
            WorkspaceRequest::Open {
                workspace_id: workspace(),
            },
        )
        .unwrap()
    {
        WorkspaceResponse::Opened { documents, .. } => documents,
        response => panic!("unexpected response: {response:?}"),
    }
}

#[test]
fn one_client_edits_a_document_until_another_takes_it_over() {
    let root = tempfile::tempdir().unwrap();
    let host = service(root.path());
    let document = open_file(&host, root.path(), "notes.md");
    let first = ClientInstanceId::new("first");
    let second = ClientInstanceId::new("second");

    let granted = acquire(&host, &first, &document, false);
    let first_lease = granted.lease.clone().unwrap();
    assert_eq!(first_lease.holder, first);
    put(&host, &first, &document, first_lease.epoch, 1, "first edit").unwrap();

    let refused = acquire(&host, &second, &document, false);
    assert_eq!(
        refused.lease,
        Some(first_lease.clone()),
        "without takeover the holder keeps the lease"
    );
    assert_eq!(
        put(&host, &second, &document, first_lease.epoch, 1, "intruder")
            .unwrap_err()
            .code,
        FailureCode::Conflict
    );
    assert_eq!(
        draft_content(&host, &second, &document).unwrap(),
        b"first edit",
        "a read-only Client sees the holder's draft"
    );

    let taken = acquire(&host, &second, &document, true);
    let second_lease = taken.lease.unwrap();
    assert_eq!(second_lease.holder, second);
    assert!(second_lease.epoch > first_lease.epoch);
    assert_eq!(
        put(&host, &first, &document, first_lease.epoch, 2, "late")
            .unwrap_err()
            .code,
        FailureCode::Conflict,
        "the previous holder's late upload is fenced by the epoch"
    );
    put(
        &host,
        &second,
        &document,
        second_lease.epoch,
        2,
        "second edit",
    )
    .unwrap();
    assert_eq!(
        draft_content(&host, &first, &document).unwrap(),
        b"second edit"
    );
}

#[test]
fn drafts_survive_a_host_restart_but_leases_do_not() {
    let root = tempfile::tempdir().unwrap();
    let client = ClientInstanceId::new("client");
    let document = {
        let host = service(root.path());
        let document = open_file(&host, root.path(), "notes.md");
        let lease = acquire(&host, &client, &document, false).lease.unwrap();
        put(&host, &client, &document, lease.epoch, 3, "unsaved").unwrap();
        document
    };

    let host = service(root.path());
    let documents = opened_documents(&host, &client);
    assert_eq!(documents.len(), 1);
    assert_eq!(documents[0].document, document);
    assert_eq!(documents[0].lease, None);
    assert_eq!(documents[0].draft.as_ref().unwrap().revision, 3);
    assert_eq!(
        draft_content(&host, &client, &document).unwrap(),
        b"unsaved"
    );
}

#[test]
fn releasing_a_document_discards_its_draft_and_ignores_stale_epochs() {
    let root = tempfile::tempdir().unwrap();
    let host = service(root.path());
    let document = open_file(&host, root.path(), "notes.md");
    let first = ClientInstanceId::new("first");
    let second = ClientInstanceId::new("second");
    let stale = acquire(&host, &first, &document, false).lease.unwrap();
    let current = acquire(&host, &second, &document, true).lease.unwrap();
    put(&host, &second, &document, current.epoch, 1, "kept").unwrap();

    let unchanged = release(&host, &first, &document, stale.epoch);
    assert_eq!(unchanged.lease, Some(current.clone()));
    assert!(unchanged.draft.is_some());

    let released = release(&host, &second, &document, current.epoch);
    assert_eq!((released.lease, released.draft), (None, None));
    assert_eq!(
        draft_content(&host, &second, &document).unwrap_err().code,
        FailureCode::NotFound
    );
    drop(host);
    assert!(opened_documents(&service(root.path()), &first).is_empty());
}

#[test]
fn closing_a_file_drops_its_draft_and_lease() {
    let root = tempfile::tempdir().unwrap();
    let host = service(root.path());
    let closed = open_file(&host, root.path(), "closed.md");
    let kept = open_file(&host, root.path(), "kept.md");
    let client = ClientInstanceId::new("client");
    for document in [&closed, &kept] {
        let lease = acquire(&host, &client, document, false).lease.unwrap();
        put(&host, &client, document, lease.epoch, 1, "draft").unwrap();
    }

    host.apply(
        workspace(),
        WorkspaceIntent::CloseFile {
            document: closed.clone(),
        },
        None,
    )
    .unwrap();

    let documents = opened_documents(&host, &client);
    assert_eq!(
        documents
            .iter()
            .map(|state| &state.document)
            .collect::<Vec<_>>(),
        [&kept]
    );
    assert_eq!(
        host.handle(
            &client,
            WorkspaceRequest::AcquireDocument {
                workspace_id: workspace(),
                document: closed,
                takeover: false,
            },
        )
        .unwrap_err()
        .code,
        FailureCode::NotFound,
        "a closed document cannot be leased"
    );
}

#[test]
fn an_expired_session_releases_its_leases_and_keeps_its_drafts() {
    let root = tempfile::tempdir().unwrap();
    let host = service(root.path());
    let document = open_file(&host, root.path(), "notes.md");
    let gone = ClientInstanceId::new("gone");
    let lease = acquire(&host, &gone, &document, false).lease.unwrap();
    put(&host, &gone, &document, lease.epoch, 1, "orphaned").unwrap();

    host.release_client_documents(&gone);

    let next = ClientInstanceId::new("next");
    let state = acquire(&host, &next, &document, false);
    assert_eq!(state.lease.unwrap().holder, next);
    assert_eq!(state.draft.unwrap().revision, 1);
    assert_eq!(draft_content(&host, &next, &document).unwrap(), b"orphaned");
}

#[test]
fn clients_that_opened_the_workspace_see_lease_and_draft_changes() {
    let root = tempfile::tempdir().unwrap();
    let host = Arc::new(service(root.path()));
    let document = open_file(&host, root.path(), "notes.md");
    let watcher = ClientInstanceId::new("watcher");
    let editor = ClientInstanceId::new("editor");
    let seen = Arc::new(Mutex::new(Vec::<DocumentUpdate>::new()));
    let _subscription = host.subscribe(watcher.clone(), {
        let seen = seen.clone();
        Box::new(move |event| {
            if let WorkspaceEvent::Document(update) = event {
                seen.lock().push(update.clone());
            }
            true
        })
    });
    opened_documents(&host, &watcher);

    let lease = acquire(&host, &editor, &document, false).lease.unwrap();
    put(&host, &editor, &document, lease.epoch, 1, "draft").unwrap();
    release(&host, &editor, &document, lease.epoch);

    let seen = seen.lock();
    let states = seen
        .iter()
        .map(|update| {
            (
                update
                    .state
                    .lease
                    .as_ref()
                    .map(|lease| lease.holder.clone()),
                update
                    .state
                    .draft
                    .as_ref()
                    .map(|draft: &DraftRef| draft.revision),
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(
        states,
        [
            (Some(editor.clone()), None),
            (Some(editor.clone()), Some(1)),
            (None, None),
        ]
    );
}

#[test]
fn schema_one_inline_drafts_become_document_drafts() {
    let root = tempfile::tempdir().unwrap();
    let layout = {
        let host = service(root.path());
        open_file(&host, root.path(), "notes.md");
        host.state.lock().workspaces[&workspace()].layout.state()
    };
    fs::remove_dir_all(root.path().join("workspaces")).unwrap();
    fs::create_dir(root.path().join("workspaces")).unwrap();
    let draft = DraftContentRevision {
        revision: 7,
        base: DraftBase::Json {
            metadata: serde_json::json!({ "original_base": 3 }),
        },
        content: "unsaved".to_string(),
    };
    let legacy = serde_json::json!({
        "schema_version": 1,
        "revision": 5,
        "snapshot": {
            "schema_version": 2,
            "workspace": layout.workspace,
            "editor": layout.editor,
            "documents": [
                { "project_id": "project", "relative_path": "notes.md", "draft": draft },
                { "project_id": "project", "relative_path": "closed.md", "draft": draft },
            ],
        },
    });
    fs::write(
        root.path().join("workspaces/default.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();

    let reader = ClientInstanceId::new("reader");
    let document = DocumentId {
        project_id: ProjectId::new("project"),
        canonical_path: project_root(root.path()).join("notes.md"),
    };
    for _restart in 0..2 {
        let host = service(root.path());
        let documents = opened_documents(&host, &reader);
        assert_eq!(
            documents
                .iter()
                .map(|state| (
                    &state.document,
                    state.draft.as_ref().map(|draft| draft.revision)
                ))
                .collect::<Vec<_>>(),
            [(&document, Some(7))],
            "only drafts of documents open in the layout are kept"
        );
        assert_eq!(
            draft_content(&host, &reader, &document).unwrap(),
            b"unsaved"
        );
        assert_eq!(host.state.lock().workspaces[&workspace()].revision(), 5);
    }
}
