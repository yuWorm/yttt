//! Edit leases and drafts of the documents open in a workspace.
//!
//! One Client at a time holds a document's lease and uploads its unsaved
//! changes as the document's draft; the others show the document read-only
//! and read the draft. A lease is granted while the document has unsaved
//! changes and released once they are saved or discarded. It belongs to the
//! Client session: it survives a reconnect and ends when the session expires
//! or another Client takes the document over.

use yttt_core::model::{editor::DocumentId, ids::ClientInstanceId};
use yttt_protocol::{
    FailureCode, ProtocolFailure,
    workspace::{
        DocumentLease, DocumentState, DocumentUpdate, DraftBase, DraftContentRevision, DraftRef,
        WorkspaceId, WorkspaceResponse,
    },
};

use super::{
    ServiceState, StoredDocument, WorkspaceEvent, WorkspaceRecord, WorkspaceService, failure,
    filesystem_failure,
};

impl WorkspaceService {
    /// The lease and draft of every document of a workspace that has either.
    pub(super) fn document_states(
        state: &ServiceState,
        workspace_id: &WorkspaceId,
        record: &WorkspaceRecord,
    ) -> Vec<DocumentState> {
        let mut documents = record
            .documents
            .iter()
            .map(|stored| DocumentState {
                document: stored.document.clone(),
                lease: state
                    .leases
                    .get(&(workspace_id.clone(), stored.document.clone()))
                    .cloned(),
                draft: Some(stored.draft.clone()),
            })
            .collect::<Vec<_>>();
        for ((id, document), lease) in &state.leases {
            if id == workspace_id && record.draft(document).is_none() {
                documents.push(DocumentState {
                    document: document.clone(),
                    lease: Some(lease.clone()),
                    draft: None,
                });
            }
        }
        documents
    }

    pub(super) fn acquire_document(
        &self,
        state: &mut ServiceState,
        client_id: &ClientInstanceId,
        workspace_id: WorkspaceId,
        document: DocumentId,
        takeover: bool,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        open_document(state, &workspace_id, &document)?;
        let key = (workspace_id.clone(), document.clone());
        let grant = match state.leases.get(&key) {
            None => true,
            Some(lease) => &lease.holder != client_id && takeover,
        };
        if grant {
            let epoch = state.next_lease_epoch;
            state.next_lease_epoch += 1;
            state.leases.insert(
                key,
                DocumentLease {
                    holder: client_id.clone(),
                    epoch,
                },
            );
            Self::notify_document(state, &workspace_id, &document);
        }
        Ok(WorkspaceResponse::Document(document_state(
            state,
            &workspace_id,
            document,
        )))
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn put_document_draft(
        &self,
        state: &mut ServiceState,
        client_id: &ClientInstanceId,
        workspace_id: WorkspaceId,
        document: DocumentId,
        epoch: u64,
        revision: u64,
        base: DraftBase,
        content: Vec<u8>,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        let record = open_document(state, &workspace_id, &document)?;
        require_lease(state, &workspace_id, &document, client_id, epoch)?;
        let content = String::from_utf8(content).map_err(|_| {
            failure(
                FailureCode::InvalidRequest,
                "document draft is not UTF-8 text",
                false,
            )
        })?;
        let key = serde_json::to_vec(&document)
            .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?;
        let draft = DraftContentRevision {
            revision,
            base,
            content,
        };
        let reference = DraftRef::for_document(&key, &draft);
        let mut updated = record.clone();
        let previous = updated.draft_refs();
        updated
            .documents
            .retain(|stored| stored.document != document);
        updated.documents.push(StoredDocument {
            document: document.clone(),
            draft: reference,
        });
        let current = updated.draft_refs();
        self.drafts
            .put(
                &workspace_id,
                current.last().expect("just pushed"),
                draft.content.as_bytes(),
            )
            .map_err(|error| filesystem_failure("persist draft body", error))?;
        let stored = self
            .drafts
            .validate_references(&workspace_id, &current)
            .map_err(|error| filesystem_failure("verify document drafts", error))
            .and_then(|()| self.persist_workspace(state, &workspace_id, &updated));
        if let Err(error) = stored {
            self.drafts
                .collect_obsolete(&workspace_id, &current, &previous);
            return Err(error);
        }
        self.drafts
            .collect_obsolete(&workspace_id, &previous, &current);
        state.workspaces.insert(workspace_id.clone(), updated);
        Self::notify_document(state, &workspace_id, &document);
        Ok(WorkspaceResponse::Document(document_state(
            state,
            &workspace_id,
            document,
        )))
    }

    pub(super) fn get_document_draft(
        &self,
        state: &ServiceState,
        workspace_id: &WorkspaceId,
        document: &DocumentId,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        let reference = state
            .workspaces
            .get(workspace_id)
            .and_then(|record| record.draft(document))
            .cloned()
            .ok_or_else(|| failure(FailureCode::NotFound, "document has no draft", false))?;
        let content = self
            .drafts
            .read(workspace_id, &reference)
            .map_err(|error| filesystem_failure("read draft body", error))?;
        Ok(WorkspaceResponse::DocumentDraft { reference, content })
    }

    pub(super) fn release_document(
        &self,
        state: &mut ServiceState,
        client_id: &ClientInstanceId,
        workspace_id: WorkspaceId,
        document: DocumentId,
        epoch: u64,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        let key = (workspace_id.clone(), document.clone());
        let held = state
            .leases
            .get(&key)
            .is_some_and(|lease| &lease.holder == client_id && lease.epoch == epoch);
        if held {
            if let Some(record) = state.workspaces.get(&workspace_id)
                && record.draft(&document).is_some()
            {
                let mut updated = record.clone();
                let previous = updated.draft_refs();
                updated
                    .documents
                    .retain(|stored| stored.document != document);
                self.persist_workspace(state, &workspace_id, &updated)?;
                self.drafts
                    .collect_obsolete(&workspace_id, &previous, &updated.draft_refs());
                state.workspaces.insert(workspace_id.clone(), updated);
            }
            state.leases.remove(&key);
            Self::notify_document(state, &workspace_id, &document);
        }
        Ok(WorkspaceResponse::Document(document_state(
            state,
            &workspace_id,
            document,
        )))
    }

    /// Ends the leases of a Client whose session expired; their drafts stay
    /// for the next Client that edits the documents.
    pub(crate) fn release_client_documents(&self, client_id: &ClientInstanceId) {
        let mut state = self.state.lock();
        let released = state
            .leases
            .iter()
            .filter(|(_, lease)| &lease.holder == client_id)
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>();
        for (workspace_id, document) in released {
            state
                .leases
                .remove(&(workspace_id.clone(), document.clone()));
            Self::notify_document(&mut state, &workspace_id, &document);
        }
    }

    fn notify_document(
        state: &mut ServiceState,
        workspace_id: &WorkspaceId,
        document: &DocumentId,
    ) {
        let update = DocumentUpdate {
            workspace_id: workspace_id.clone(),
            state: document_state(state, workspace_id, document.clone()),
        };
        Self::notify(state, workspace_id, WorkspaceEvent::Document(&update));
    }
}

/// The workspace record of a document open in it.
fn open_document<'a>(
    state: &'a ServiceState,
    workspace_id: &WorkspaceId,
    document: &DocumentId,
) -> Result<&'a WorkspaceRecord, ProtocolFailure> {
    state
        .workspaces
        .get(workspace_id)
        .filter(|record| record.layout.contains_document(document))
        .ok_or_else(|| {
            failure(
                FailureCode::NotFound,
                "document is not open in the workspace",
                false,
            )
        })
}

fn require_lease(
    state: &ServiceState,
    workspace_id: &WorkspaceId,
    document: &DocumentId,
    client_id: &ClientInstanceId,
    epoch: u64,
) -> Result<(), ProtocolFailure> {
    let held = state
        .leases
        .get(&(workspace_id.clone(), document.clone()))
        .is_some_and(|lease| &lease.holder == client_id && lease.epoch == epoch);
    if held {
        Ok(())
    } else {
        Err(failure(
            FailureCode::Conflict,
            "the document's edit lease is no longer held",
            false,
        ))
    }
}

fn document_state(
    state: &ServiceState,
    workspace_id: &WorkspaceId,
    document: DocumentId,
) -> DocumentState {
    let key = (workspace_id.clone(), document);
    DocumentState {
        lease: state.leases.get(&key).cloned(),
        draft: state
            .workspaces
            .get(workspace_id)
            .and_then(|record| record.draft(&key.1))
            .cloned(),
        document: key.1,
    }
}

#[cfg(test)]
mod tests;
