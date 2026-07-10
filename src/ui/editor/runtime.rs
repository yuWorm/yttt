use std::{collections::HashMap, path::PathBuf};

use gpui::{Entity, Subscription};

use crate::{model::ids::ProjectId, ui::project_tree::ProjectTreeView};

use super::{
    DocumentId, ProjectEditorDocument, ProjectEditorWorkspaceState, ProjectWorkItemSession,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectFileLoadRequest {
    pub document_id: DocumentId,
    pub project_root: PathBuf,
    pub relative_path: PathBuf,
    pub generation: u64,
}

#[derive(Default)]
pub struct ProjectEditorRuntime {
    workspace: ProjectEditorWorkspaceState,
    documents: HashMap<DocumentId, Entity<ProjectEditorDocument>>,
    document_subscriptions: HashMap<DocumentId, Subscription>,
    trees: HashMap<ProjectId, Entity<ProjectTreeView>>,
    tree_subscriptions: HashMap<ProjectId, Subscription>,
    pending_tree_generations: HashMap<ProjectId, u64>,
    pending_file_generations: HashMap<DocumentId, u64>,
    next_file_generation: u64,
}

impl ProjectEditorRuntime {
    pub fn workspace(&self) -> &ProjectEditorWorkspaceState {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut ProjectEditorWorkspaceState {
        &mut self.workspace
    }

    pub fn open_project(
        &mut self,
        project_id: ProjectId,
        root: impl Into<PathBuf>,
        selected_terminal_id: Option<String>,
        project_panel_visible: bool,
        project_panel_width: f32,
    ) -> bool {
        self.workspace.open_project(
            project_id,
            root,
            selected_terminal_id,
            project_panel_visible,
            project_panel_width,
        )
    }

    pub fn close_project(&mut self, project_id: &ProjectId) -> Option<ProjectWorkItemSession> {
        let session = self.workspace.close_project(project_id);
        self.documents
            .retain(|document_id, _| &document_id.project_id != project_id);
        self.document_subscriptions
            .retain(|document_id, _| &document_id.project_id != project_id);
        self.pending_file_generations
            .retain(|document_id, _| &document_id.project_id != project_id);
        self.trees.remove(project_id);
        self.tree_subscriptions.remove(project_id);
        self.pending_tree_generations.remove(project_id);
        session
    }

    pub fn document(&self, document_id: &DocumentId) -> Option<&Entity<ProjectEditorDocument>> {
        self.documents.get(document_id)
    }

    pub fn insert_document(
        &mut self,
        document_id: DocumentId,
        document: Entity<ProjectEditorDocument>,
        subscription: Subscription,
    ) -> Option<Entity<ProjectEditorDocument>> {
        self.document_subscriptions
            .insert(document_id.clone(), subscription);
        self.documents.insert(document_id, document)
    }

    pub fn remove_document(
        &mut self,
        document_id: &DocumentId,
    ) -> Option<Entity<ProjectEditorDocument>> {
        self.document_subscriptions.remove(document_id);
        self.pending_file_generations.remove(document_id);
        self.documents.remove(document_id)
    }

    pub fn documents_for_project(
        &self,
        project_id: &ProjectId,
    ) -> impl Iterator<Item = (&DocumentId, &Entity<ProjectEditorDocument>)> {
        self.documents
            .iter()
            .filter(move |(document_id, _)| &document_id.project_id == project_id)
    }

    pub fn tree(&self, project_id: &ProjectId) -> Option<&Entity<ProjectTreeView>> {
        self.trees.get(project_id)
    }

    pub fn insert_tree(
        &mut self,
        project_id: ProjectId,
        tree: Entity<ProjectTreeView>,
        subscription: Subscription,
    ) -> Option<Entity<ProjectTreeView>> {
        self.tree_subscriptions
            .insert(project_id.clone(), subscription);
        self.trees.insert(project_id, tree)
    }

    pub fn remove_tree(&mut self, project_id: &ProjectId) -> Option<Entity<ProjectTreeView>> {
        self.tree_subscriptions.remove(project_id);
        self.pending_tree_generations.remove(project_id);
        self.trees.remove(project_id)
    }

    pub fn track_tree_load(&mut self, project_id: ProjectId, generation: u64) {
        self.pending_tree_generations.insert(project_id, generation);
    }

    pub fn tree_load_is_current(&self, project_id: &ProjectId, generation: u64) -> bool {
        self.pending_tree_generations.get(project_id) == Some(&generation)
    }

    pub fn finish_tree_load(&mut self, project_id: &ProjectId, generation: u64) -> bool {
        if !self.tree_load_is_current(project_id, generation) {
            return false;
        }
        self.pending_tree_generations.remove(project_id);
        true
    }

    pub fn track_file_load(&mut self, document_id: DocumentId, generation: u64) {
        self.next_file_generation = self.next_file_generation.max(generation);
        self.pending_file_generations
            .insert(document_id, generation);
    }

    pub fn begin_file_load(&mut self, document_id: DocumentId) -> Option<u64> {
        if self.documents.contains_key(&document_id)
            || self.pending_file_generations.contains_key(&document_id)
        {
            return None;
        }
        self.next_file_generation = self.next_file_generation.wrapping_add(1).max(1);
        let generation = self.next_file_generation;
        self.pending_file_generations
            .insert(document_id, generation);
        Some(generation)
    }

    pub fn file_load_is_current(&self, document_id: &DocumentId, generation: u64) -> bool {
        self.pending_file_generations.get(document_id) == Some(&generation)
    }

    pub fn finish_file_load(&mut self, document_id: &DocumentId, generation: u64) -> bool {
        if !self.file_load_is_current(document_id, generation) {
            return false;
        }
        self.pending_file_generations.remove(document_id);
        true
    }
}
