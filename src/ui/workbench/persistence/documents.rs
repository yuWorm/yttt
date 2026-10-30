//! Keeps the unsaved edits of open documents on the Host under per-document
//! edit leases.
//!
//! A Client edits a document only while it holds the document's lease; it
//! uploads each new generation of unsaved text as the document's draft and
//! releases the lease once the text is saved or discarded. Other Clients
//! show the holder's draft read-only until they take the document over.
//! Edits this Client had not uploaded when it lost a lease are kept as a
//! Device copy, the only Client-side recovery that remains.
//!
//! Every tick reconciles each loaded document with the latest Host state;
//! Host change events only make that happen sooner.

use super::*;
use crate::ui::editor::HeldDocument;
use yttt_protocol::workspace::{DocumentLease, DocumentState};

/// What this Client knows about the Host side of the documents it shows.
#[derive(Default)]
pub(super) struct DocumentSyncState {
    listener: Option<DocumentListener>,
    entries: HashMap<DocumentId, DocumentSync>,
    error: Option<String>,
}

struct DocumentListener {
    workspace_id: WorkspaceId,
    changes: flume::Receiver<Option<DocumentState>>,
    _task: Task<()>,
}

#[derive(Default)]
struct DocumentSync {
    /// The latest lease and draft the Host reported.
    host: Option<DocumentState>,
    /// The lease epoch this Client edits under.
    lease_epoch: Option<u64>,
    /// The editor generation whose text the Host stores as the draft.
    uploaded: Option<u64>,
    /// The draft revision shown read-only or restored into the editor.
    shown_draft: Option<u64>,
    in_flight: bool,
}

enum DocumentStep {
    Acquire {
        takeover: bool,
    },
    Upload {
        generation: u64,
        draft: DraftContentRevision,
    },
    Release {
        epoch: u64,
    },
    ShowDraft,
    Recover,
}

/// What one document step learned: the Host's document state and, when the
/// step fetched it, the draft content to show.
struct DocumentOutcome {
    state: Option<DocumentState>,
    draft: Option<(DraftRef, String)>,
}

impl DocumentOutcome {
    fn state(state: DocumentState) -> Self {
        Self {
            state: Some(state),
            draft: None,
        }
    }
}

impl WorkbenchView {
    /// Listens for lease and draft changes of `workspace_id`'s documents.
    pub(super) fn ensure_document_listener(
        &mut self,
        runtime: &crate::host_runtime::DesktopHostRuntime,
        workspace_id: &WorkspaceId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .workspace_persistence
            .documents
            .listener
            .as_ref()
            .is_some_and(|listener| {
                &listener.workspace_id == workspace_id && !listener.changes.is_disconnected()
            })
        {
            return;
        }
        let changes = runtime.document_changes(workspace_id.clone());
        let receiver = changes.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            while let Ok(change) = receiver.recv_async().await {
                if this
                    .update_in(cx, |root, window, cx| match change {
                        Some(state) => {
                            let document_id = state.document.clone();
                            root.record_document_state(state);
                            root.reconcile_document(&document_id, window, cx);
                        }
                        // Dropped changes: the next tick opens the workspace
                        // again, which reports every document's state.
                        None => root.workspace_persistence.connection_sequence = None,
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        self.workspace_persistence.documents.listener = Some(DocumentListener {
            workspace_id: workspace_id.clone(),
            changes,
            _task: task,
        });
    }

    /// Replaces the Host states of all documents with `states`, as the
    /// Host reports them when the workspace is opened.
    pub(super) fn record_all_document_states(&mut self, states: Vec<DocumentState>) {
        for entry in self.workspace_persistence.documents.entries.values_mut() {
            entry.host = None;
        }
        for state in states {
            self.record_document_state(state);
        }
    }

    fn record_document_state(&mut self, state: DocumentState) {
        let entry = self
            .workspace_persistence
            .documents
            .entries
            .entry(state.document.clone())
            .or_default();
        entry.host = Some(state);
    }

    /// Reconciles every document with the Host, releases the leases of
    /// closed documents, and reports whether edits remain unstored.
    pub(super) fn sync_documents(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let loaded = self
            .workspace
            .opened_projects()
            .iter()
            .flat_map(|project| {
                self.project
                    .project_editor_runtime
                    .documents_for_project(&project.id)
                    .map(|(id, _)| id.clone())
            })
            .collect::<HashSet<_>>();
        for document_id in &loaded {
            self.reconcile_document(document_id, window, cx);
        }
        let closed = self
            .workspace_persistence
            .documents
            .entries
            .iter()
            .filter(|(id, entry)| !loaded.contains(*id) && !entry.in_flight)
            .map(|(id, entry)| (id.clone(), entry.lease_epoch))
            .collect::<Vec<_>>();
        for (document_id, epoch) in closed {
            match epoch {
                Some(epoch) => self.spawn_document_step(
                    document_id,
                    DocumentStep::Release { epoch },
                    window,
                    cx,
                ),
                None => {
                    self.workspace_persistence
                        .documents
                        .entries
                        .remove(&document_id);
                }
            }
        }
        self.report_document_sync(cx);
    }

    /// Whether edits of an open document are not stored on the Host yet.
    pub(super) fn has_unsynced_documents(&self, cx: &App) -> bool {
        self.workspace.opened_projects().iter().any(|project| {
            self.project
                .project_editor_runtime
                .documents_for_project(&project.id)
                .any(|(id, document)| {
                    let document = document.read(cx);
                    let model = document.model();
                    model.is_dirty()
                        && document.held_elsewhere().is_none()
                        && self
                            .workspace_persistence
                            .documents
                            .entries
                            .get(id)
                            .is_none_or(|entry| {
                                entry.lease_epoch.is_none()
                                    || entry.uploaded != Some(model.generation())
                            })
                })
        })
    }

    pub(super) fn document_sync_error(&self) -> Option<&str> {
        self.workspace_persistence.documents.error.as_deref()
    }

    fn report_document_sync(&self, cx: &App) {
        if let Some(view) = &self.workspace_persistence.view {
            view.report_documents(
                self.has_unsynced_documents(cx),
                self.workspace_persistence.documents.error.clone(),
            );
        }
    }

    /// Takes the edit lease of a document another Client holds and continues
    /// from its latest draft.
    pub(in crate::ui::workbench) fn take_over_document(
        &mut self,
        document_id: DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.spawn_document_step(
            document_id,
            DocumentStep::Acquire { takeover: true },
            window,
            cx,
        );
    }

    /// Brings one loaded document in line with its Host state: shows another
    /// holder's draft read-only, restores a draft no Client is editing,
    /// acquires the lease for new edits, uploads them, and releases the
    /// lease once the document is clean.
    fn reconcile_document(
        &mut self,
        document_id: &DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.workspace_persistence.mode != WorkspacePersistenceMode::Active {
            return;
        }
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        if !runtime.is_connected() {
            return;
        }
        let Some(document) = self
            .project
            .project_editor_runtime
            .document(document_id)
            .cloned()
        else {
            return;
        };
        let me = runtime.client_id();
        let entry = self
            .workspace_persistence
            .documents
            .entries
            .entry(document_id.clone())
            .or_default();
        if entry.in_flight {
            return;
        }
        let lease = entry.host.as_ref().and_then(|state| state.lease.clone());
        let draft_revision = entry
            .host
            .as_ref()
            .and_then(|state| state.draft.as_ref())
            .map(|draft| draft.revision);

        if let Some(DocumentLease { holder, .. }) = &lease
            && holder != &me
        {
            if entry.lease_epoch.is_some() {
                self.lose_document_lease(document_id, &document, window, cx);
            }
            let entry = self
                .workspace_persistence
                .documents
                .entries
                .get_mut(document_id)
                .expect("reconciled document");
            match draft_revision {
                Some(revision) if entry.shown_draft != Some(revision) => {
                    self.spawn_document_step(
                        document_id.clone(),
                        DocumentStep::ShowDraft,
                        window,
                        cx,
                    );
                }
                Some(_) => {}
                None => {
                    entry.shown_draft = None;
                    document.update(cx, |document, cx| {
                        document.set_held_elsewhere(
                            Some(HeldDocument {
                                draft: None,
                                draft_revision: 0,
                            }),
                            cx,
                        );
                    });
                }
            }
            return;
        }

        document.update(cx, |document, cx| document.set_held_elsewhere(None, cx));
        let (dirty, generation) = {
            let model = document.read(cx).model();
            (model.is_dirty(), model.generation())
        };
        match &lease {
            Some(lease) => {
                if entry.lease_epoch != Some(lease.epoch) {
                    // A lease this Client holds but did not track, such as
                    // after reopening the workspace: its draft is the latest
                    // text, unless the editor already holds newer edits.
                    entry.lease_epoch = Some(lease.epoch);
                    entry.uploaded = None;
                    if !dirty && draft_revision.is_some() {
                        self.spawn_document_step(
                            document_id.clone(),
                            DocumentStep::Recover,
                            window,
                            cx,
                        );
                        return;
                    }
                }
            }
            None => {
                entry.lease_epoch = None;
                entry.uploaded = None;
                if !dirty
                    && draft_revision.is_some_and(|revision| entry.shown_draft != Some(revision))
                {
                    // A draft left by a Client whose session ended.
                    self.spawn_document_step(
                        document_id.clone(),
                        DocumentStep::Recover,
                        window,
                        cx,
                    );
                    return;
                }
            }
        }

        let entry = self
            .workspace_persistence
            .documents
            .entries
            .get(document_id)
            .expect("reconciled document");
        let step = match (dirty, entry.lease_epoch) {
            (true, None) => Some(DocumentStep::Acquire { takeover: false }),
            (true, Some(_)) if entry.uploaded != Some(generation) => {
                match self.document_draft(&document, cx) {
                    Ok(draft) => Some(DocumentStep::Upload { generation, draft }),
                    Err(error) => {
                        self.workspace_persistence.documents.error = Some(error);
                        None
                    }
                }
            }
            (false, Some(epoch)) => Some(DocumentStep::Release { epoch }),
            _ => None,
        };
        if let Some(step) = step {
            self.spawn_document_step(document_id.clone(), step, window, cx);
        }
    }

    fn document_draft(
        &self,
        document: &Entity<ProjectEditorDocument>,
        cx: &App,
    ) -> Result<DraftContentRevision, String> {
        let document = document.read(cx);
        let model = document.model();
        let content = document.current_text(cx);
        if content.len() > MAX_DRAFT_CONTENT_BYTES {
            return Err(self
                .ui_text
                .get(UiTextKey::PersistenceDraftDocumentLimit)
                .to_string());
        }
        Ok(DraftContentRevision {
            revision: model.generation(),
            base: DraftBase::File {
                path: HostPath::from_client_path(&model.document_id().canonical_path)
                    .map_err(|error| error.to_string())?,
                base_fingerprint: disk_fingerprint_to_host(model.disk_fingerprint()),
            },
            content,
        })
    }

    fn spawn_document_step(
        &mut self,
        document_id: DocumentId,
        step: DocumentStep,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(runtime) = self.terminal.host_runtime.clone() else {
            return;
        };
        let Some(workspace_id) = self
            .workspace_persistence
            .view
            .as_ref()
            .map(|view| view.id().clone())
        else {
            return;
        };
        let entry = self
            .workspace_persistence
            .documents
            .entries
            .entry(document_id.clone())
            .or_default();
        if entry.in_flight {
            return;
        }
        entry.in_flight = true;
        let epoch = entry.lease_epoch;
        let me = runtime.client_id();
        let request_document = document_id.clone();
        let task = cx.background_spawn(async move {
            run_document_step(&runtime, workspace_id, request_document, epoch, &me, step)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                if let Some(entry) = root
                    .workspace_persistence
                    .documents
                    .entries
                    .get_mut(&document_id)
                {
                    entry.in_flight = false;
                }
                match result {
                    Ok((outcome, uploaded)) => {
                        root.workspace_persistence.documents.error = None;
                        root.apply_document_outcome(&document_id, outcome, uploaded, window, cx);
                    }
                    Err(error) => {
                        root.workspace_persistence.documents.error = Some(
                            root.ui_text
                                .get(UiTextKey::PersistenceDocumentSyncFailed)
                                .replace(
                                    "{path}",
                                    &document_id.canonical_path.display().to_string(),
                                )
                                .replace("{error}", &error),
                        );
                    }
                }
                root.report_document_sync(cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_document_outcome(
        &mut self,
        document_id: &DocumentId,
        outcome: DocumentOutcome,
        uploaded: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let me = self
            .terminal
            .host_runtime
            .as_ref()
            .map(|runtime| runtime.client_id());
        let document = self
            .project
            .project_editor_runtime
            .document(document_id)
            .cloned();
        let DocumentOutcome { state, draft } = outcome;
        if let Some(state) = state {
            let mine = state
                .lease
                .as_ref()
                .filter(|lease| Some(&lease.holder) == me.as_ref())
                .map(|lease| lease.epoch);
            self.record_document_state(state);
            let entry = self
                .workspace_persistence
                .documents
                .entries
                .get_mut(document_id)
                .expect("recorded document");
            if let Some(epoch) = mine {
                if entry.lease_epoch != Some(epoch) {
                    entry.uploaded = None;
                }
                entry.lease_epoch = Some(epoch);
                if let Some(document) = &document {
                    document.update(cx, |document, cx| document.set_held_elsewhere(None, cx));
                }
            }
            if uploaded.is_some() {
                entry.uploaded = uploaded;
            }
            if entry
                .host
                .as_ref()
                .is_some_and(|state| state.lease.is_none())
                && entry.lease_epoch.is_some()
                && document
                    .as_ref()
                    .is_none_or(|document| !document.read(cx).model().is_dirty())
            {
                entry.lease_epoch = None;
                entry.uploaded = None;
            }
        }
        if let Some((reference, text)) = draft
            && let Some(document) = document.clone()
        {
            let entry = self
                .workspace_persistence
                .documents
                .entries
                .entry(document_id.clone())
                .or_default();
            entry.shown_draft = Some(reference.revision);
            let editing = entry.lease_epoch.is_some();
            if editing {
                let conflict =
                    (!draft_base_matches(&reference.base, document.read(cx))).then(|| {
                        self.ui_text
                            .get(UiTextKey::PersistenceDraftConflict)
                            .replace("{path}", &document_id.canonical_path.display().to_string())
                    });
                document.update(cx, |document, document_cx| {
                    let selection = document.selection_snapshot(document_cx);
                    document.restore_recovered_draft(
                        text,
                        reference.revision,
                        &selection,
                        conflict,
                        window,
                        document_cx,
                    );
                });
                let generation = document.read(cx).model().generation();
                let entry = self
                    .workspace_persistence
                    .documents
                    .entries
                    .get_mut(document_id)
                    .expect("recorded document");
                // The Host already stores this text.
                entry.uploaded = (generation == reference.revision).then_some(generation);
            } else {
                document.update(cx, |document, cx| {
                    document.set_held_elsewhere(
                        Some(HeldDocument {
                            draft: Some(text),
                            draft_revision: reference.revision,
                        }),
                        cx,
                    );
                });
            }
        }
        self.reconcile_document(document_id, window, cx);
    }

    /// Another Client took over a document this Client was editing: keeps
    /// the edits the Host does not store as a Device copy and returns the
    /// editor to the file on disk.
    fn lose_document_lease(
        &mut self,
        document_id: &DocumentId,
        document: &Entity<ProjectEditorDocument>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let entry = self
            .workspace_persistence
            .documents
            .entries
            .get_mut(document_id)
            .expect("reconciled document");
        let uploaded = entry.uploaded;
        entry.lease_epoch = None;
        entry.uploaded = None;
        entry.shown_draft = None;
        let (dirty, generation) = {
            let model = document.read(cx).model();
            (model.is_dirty(), model.generation())
        };
        if !dirty {
            return;
        }
        if uploaded != Some(generation) {
            self.preserve_document_copies(std::slice::from_ref(document_id), window, cx);
        }
        self.spawn_project_file_reload(document_id.clone(), None, window, cx);
    }

    /// Writes the current text of each document to a Device copy and tells
    /// the user where it is.
    pub(super) fn preserve_document_copies(
        &mut self,
        document_ids: &[DocumentId],
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for document_id in document_ids {
            let Some(document) = self.project.project_editor_runtime.document(document_id) else {
                continue;
            };
            let text = document.read(cx).current_text(cx);
            let path = document_id.canonical_path.display().to_string();
            match write_document_copy(&self.config_paths, document_id, &text) {
                Ok(copy) => self.prompt_document_copy(path, copy, window, cx),
                Err(error) => self.set_workspace_persistence_error(
                    self.ui_text
                        .get(UiTextKey::PersistenceDocumentSyncFailed)
                        .replace("{path}", &path)
                        .replace("{error}", &error),
                ),
            }
        }
    }

    fn prompt_document_copy(
        &mut self,
        path: String,
        copy: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let body = self
            .ui_text
            .get(UiTextKey::DocumentLeaseLostBody)
            .replace("{path}", &path)
            .replace("{copy}", &copy.display().to_string());
        let answer = window.prompt(
            gpui::PromptLevel::Warning,
            self.ui_text.get(UiTextKey::DocumentLeaseLostTitle),
            Some(&body),
            &[
                self.ui_text.get(UiTextKey::DocumentLeaseLostKeep),
                self.ui_text.get(UiTextKey::DocumentLeaseLostDiscard),
            ],
            cx,
        );
        cx.spawn(async move |_, _| {
            if matches!(answer.await, Ok(1)) {
                let _ = fs::remove_file(copy);
            }
        })
        .detach();
    }
}

/// Runs one document step against the Host. Returns the outcome and, for an
/// upload, the generation the Host now stores.
fn run_document_step(
    runtime: &crate::host_runtime::DesktopHostRuntime,
    workspace_id: WorkspaceId,
    document: DocumentId,
    epoch: Option<u64>,
    me: &yttt_core::model::ids::ClientInstanceId,
    step: DocumentStep,
) -> Result<(DocumentOutcome, Option<u64>), String> {
    let request = |request: WorkspaceRequest| runtime.workspace_request(request);
    let state = |response: WorkspaceResponse| match response {
        WorkspaceResponse::Document(state) => Ok(state),
        response => Err(format!("unexpected document response: {response:?}")),
    };
    let acquire = |takeover: bool| {
        request(WorkspaceRequest::AcquireDocument {
            workspace_id: workspace_id.clone(),
            document: document.clone(),
            takeover,
        })
        .and_then(state)
    };
    let draft = || match request(WorkspaceRequest::GetDocumentDraft {
        workspace_id: workspace_id.clone(),
        document: document.clone(),
    })? {
        WorkspaceResponse::DocumentDraft { reference, content } => String::from_utf8(content)
            .map(|text| (reference, text))
            .map_err(|error| error.to_string()),
        response => Err(format!("unexpected draft response: {response:?}")),
    };
    let held = |state: &DocumentState| {
        state
            .lease
            .as_ref()
            .is_some_and(|lease| &lease.holder == me)
    };
    match step {
        DocumentStep::Acquire { takeover } => {
            let state = acquire(takeover)?;
            if takeover && held(&state) && state.draft.is_some() {
                let draft = draft()?;
                return Ok((
                    DocumentOutcome {
                        state: Some(state),
                        draft: Some(draft),
                    },
                    None,
                ));
            }
            Ok((DocumentOutcome::state(state), None))
        }
        DocumentStep::Upload { generation, draft } => {
            let Some(epoch) = epoch else {
                return Ok((DocumentOutcome::state(acquire(false)?), None));
            };
            match runtime.request_blocking_typed(yttt_protocol::Request::Workspace(
                WorkspaceRequest::PutDocumentDraft {
                    workspace_id: workspace_id.clone(),
                    document: document.clone(),
                    epoch,
                    revision: draft.revision,
                    base: draft.base,
                    content: draft.content.into_bytes(),
                },
            )) {
                Ok(yttt_protocol::Response::Workspace(response)) => {
                    Ok((DocumentOutcome::state(state(response)?), Some(generation)))
                }
                // The lease ended: the current holder, if any, decides what
                // happens to these edits.
                Err(yttt_client_core::ClientCoreError::Protocol(failure))
                    if failure.code == yttt_protocol::FailureCode::Conflict =>
                {
                    Ok((DocumentOutcome::state(acquire(false)?), None))
                }
                Ok(response) => Err(format!("unexpected draft response: {response:?}")),
                Err(error) => Err(error.to_string()),
            }
        }
        DocumentStep::Release { epoch } => request(WorkspaceRequest::ReleaseDocument {
            workspace_id: workspace_id.clone(),
            document: document.clone(),
            epoch,
        })
        .and_then(state)
        .map(|state| (DocumentOutcome::state(state), None)),
        DocumentStep::ShowDraft => Ok((
            DocumentOutcome {
                state: None,
                draft: Some(draft()?),
            },
            None,
        )),
        DocumentStep::Recover => {
            let state = acquire(false)?;
            let draft = if held(&state) && state.draft.is_some() {
                Some(draft()?)
            } else {
                None
            };
            Ok((
                DocumentOutcome {
                    state: Some(state),
                    draft,
                },
                None,
            ))
        }
    }
}

fn draft_base_matches(base: &DraftBase, document: &ProjectEditorDocument) -> bool {
    let DraftBase::File {
        path,
        base_fingerprint,
    } = base
    else {
        return false;
    };
    HostPath::from_client_path(&document.model().document_id().canonical_path)
        .ok()
        .as_ref()
        == Some(path)
        && disk_fingerprint_to_host(document.model().disk_fingerprint()) == *base_fingerprint
}

fn disk_fingerprint_to_host(fingerprint: &DiskFingerprint) -> ProjectFileFingerprint {
    ProjectFileFingerprint {
        exists: fingerprint.exists,
        byte_len: fingerprint.byte_len,
        modified_nanos: fingerprint.modified.and_then(|modified| {
            modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_nanos())
        }),
        content_hash: fingerprint.content_hash,
        revision: ContentRevision {
            workspace_epoch: fingerprint.workspace_epoch,
            revision_number: fingerprint.revision_number,
            content_sha256: fingerprint.content_sha256,
        },
    }
}

/// Writes `text` to a new Device-private file named after the document.
fn write_document_copy(
    paths: &AppConfigPaths,
    document_id: &DocumentId,
    text: &str,
) -> Result<PathBuf, String> {
    let directory = paths
        .device_state_dir()
        .ok_or("Device state storage is unavailable")?
        .join("document-copies");
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let name = document_id
        .canonical_path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "document".into());
    let millis = std::time::SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_millis());
    let path = directory.join(format!("{millis}-{name}"));
    write_device_private_file(&path, text.as_bytes()).map_err(|error| error.to_string())?;
    Ok(path)
}

/// Creates `path` readable only by the current user.
fn write_device_private_file(path: &Path, source: &[u8]) -> io::Result<()> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(path)?;
    file.write_all(source)?;
    file.sync_all()
}
