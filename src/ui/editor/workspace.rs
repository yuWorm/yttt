use std::{collections::HashMap, path::PathBuf};

use crate::{model::ids::ProjectId, ui::project_tree::ProjectFileTree};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DocumentId {
    pub project_id: ProjectId,
    pub canonical_path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum WorkItemId {
    Terminal(String),
    File(DocumentId),
}

#[derive(Clone, Debug)]
pub struct ProjectWorkItemSession {
    project_id: ProjectId,
    file_ids: Vec<DocumentId>,
    active_work_item: Option<WorkItemId>,
    activation_history: Vec<WorkItemId>,
    work_item_order: Option<Vec<WorkItemId>>,
    file_tree: ProjectFileTree,
    project_panel_visible: bool,
    project_panel_width: f32,
}

impl ProjectWorkItemSession {
    pub fn new(
        project_id: ProjectId,
        root: impl Into<PathBuf>,
        selected_terminal_id: Option<String>,
        project_panel_visible: bool,
        project_panel_width: f32,
    ) -> Self {
        let active_work_item = selected_terminal_id.map(WorkItemId::Terminal);
        let activation_history = active_work_item.iter().cloned().collect();
        Self {
            project_id,
            file_ids: Vec::new(),
            active_work_item,
            activation_history,
            work_item_order: None,
            file_tree: ProjectFileTree::new(root),
            project_panel_visible,
            project_panel_width,
        }
    }

    pub fn project_id(&self) -> &ProjectId {
        &self.project_id
    }

    pub fn file_ids(&self) -> &[DocumentId] {
        &self.file_ids
    }

    pub fn active_work_item(&self) -> Option<&WorkItemId> {
        self.active_work_item.as_ref()
    }

    pub fn activation_history(&self) -> &[WorkItemId] {
        &self.activation_history
    }

    pub fn open_file(&mut self, canonical_path: impl Into<PathBuf>) -> DocumentId {
        let id = DocumentId {
            project_id: self.project_id.clone(),
            canonical_path: canonical_path.into(),
        };
        let item = WorkItemId::File(id.clone());
        if !self.file_ids.contains(&id) {
            self.file_ids.push(id.clone());
            if let Some(order) = &mut self.work_item_order {
                order.push(item.clone());
            }
        }
        self.activate(item);
        id
    }

    pub fn ordered_items(&self, terminal_ids: &[String]) -> Vec<WorkItemId> {
        let available = terminal_ids
            .iter()
            .cloned()
            .map(WorkItemId::Terminal)
            .chain(self.file_ids.iter().cloned().map(WorkItemId::File))
            .collect::<Vec<_>>();
        let Some(custom_order) = &self.work_item_order else {
            return available;
        };

        let mut ordered = custom_order
            .iter()
            .filter(|item| available.contains(item))
            .cloned()
            .collect::<Vec<_>>();
        for item in available {
            if !ordered.contains(&item) {
                ordered.push(item);
            }
        }
        ordered
    }

    pub fn move_work_item(
        &mut self,
        item: &WorkItemId,
        to_index: usize,
        terminal_ids: &[String],
    ) -> bool {
        let mut ordered = self.ordered_items(terminal_ids);
        let Some(from_index) = ordered.iter().position(|candidate| candidate == item) else {
            return false;
        };
        if from_index == to_index {
            return false;
        }

        let moved = ordered.remove(from_index);
        ordered.insert(to_index.min(ordered.len()), moved);
        self.work_item_order = Some(ordered);
        true
    }

    pub fn reconcile_work_item_order(&mut self, terminal_ids: &[String]) {
        let file_ids = &self.file_ids;
        if let Some(order) = &mut self.work_item_order {
            order.retain(|item| match item {
                WorkItemId::Terminal(id) => terminal_ids.contains(id),
                WorkItemId::File(id) => file_ids.contains(id),
            });
        }
    }

    pub fn select_work_item(&mut self, item: WorkItemId, terminal_ids: &[String]) -> bool {
        let exists = match &item {
            WorkItemId::Terminal(id) => terminal_ids.contains(id),
            WorkItemId::File(id) => self.file_ids.contains(id),
        };
        if !exists {
            return false;
        }
        self.activate(item);
        true
    }

    pub fn select_next(&mut self, terminal_ids: &[String]) -> Option<WorkItemId> {
        self.select_relative(terminal_ids, 1)
    }

    pub fn select_previous(&mut self, terminal_ids: &[String]) -> Option<WorkItemId> {
        self.select_relative(terminal_ids, -1)
    }

    pub fn close_file(
        &mut self,
        document_id: &DocumentId,
        terminal_ids: &[String],
    ) -> Option<WorkItemId> {
        let closing = WorkItemId::File(document_id.clone());
        let before = self.ordered_items(terminal_ids);
        let position = before.iter().position(|item| item == &closing)?;
        let was_active = self.active_work_item.as_ref() == Some(&closing);

        self.file_ids.retain(|id| id != document_id);
        self.activation_history.retain(|item| item != &closing);
        if let Some(order) = &mut self.work_item_order {
            order.retain(|item| item != &closing);
        }
        if was_active {
            let after = self.ordered_items(terminal_ids);
            let next = after
                .get(position)
                .cloned()
                .or_else(|| after.last().cloned());
            self.active_work_item = None;
            if let Some(next) = next {
                self.activate(next);
            }
        }

        self.active_work_item.clone()
    }

    fn relocate_file_within_session(&mut self, old: &DocumentId, new: DocumentId) -> bool {
        if old.project_id != self.project_id
            || new.project_id != self.project_id
            || (old != &new && self.file_ids.contains(&new))
        {
            return false;
        }
        let Some(index) = self.file_ids.iter().position(|id| id == old) else {
            return false;
        };
        self.file_ids[index] = new.clone();
        let old_item = WorkItemId::File(old.clone());
        let new_item = WorkItemId::File(new);
        if self.active_work_item.as_ref() == Some(&old_item) {
            self.active_work_item = Some(new_item.clone());
        }
        for item in &mut self.activation_history {
            if item == &old_item {
                *item = new_item.clone();
            }
        }
        if let Some(order) = &mut self.work_item_order {
            for item in order {
                if item == &old_item {
                    *item = new_item.clone();
                }
            }
        }
        true
    }

    pub fn file_tree(&self) -> &ProjectFileTree {
        &self.file_tree
    }

    pub fn file_tree_mut(&mut self) -> &mut ProjectFileTree {
        &mut self.file_tree
    }

    pub fn project_panel_visible(&self) -> bool {
        self.project_panel_visible
    }

    pub fn set_project_panel_visible(&mut self, visible: bool) {
        self.project_panel_visible = visible;
    }

    pub fn toggle_project_panel(&mut self) -> bool {
        self.project_panel_visible = !self.project_panel_visible;
        self.project_panel_visible
    }

    pub fn project_panel_width(&self) -> f32 {
        self.project_panel_width
    }

    pub fn set_project_panel_width(&mut self, width: f32) {
        self.project_panel_width = width;
    }

    fn select_relative(&mut self, terminal_ids: &[String], offset: isize) -> Option<WorkItemId> {
        let items = self.ordered_items(terminal_ids);
        if items.is_empty() {
            self.active_work_item = None;
            return None;
        }
        let index = match self
            .active_work_item
            .as_ref()
            .and_then(|active| items.iter().position(|item| item == active))
        {
            Some(index) => (index as isize + offset).rem_euclid(items.len() as isize) as usize,
            None if offset < 0 => items.len() - 1,
            None => 0,
        };
        let next = items[index].clone();
        self.activate(next.clone());
        Some(next)
    }

    fn activate(&mut self, item: WorkItemId) {
        self.activation_history.retain(|existing| existing != &item);
        self.activation_history.push(item.clone());
        self.active_work_item = Some(item);
    }
}

#[derive(Clone, Debug, Default)]
pub struct ProjectEditorWorkspaceState {
    sessions: HashMap<ProjectId, ProjectWorkItemSession>,
}

impl ProjectEditorWorkspaceState {
    pub fn open_project(
        &mut self,
        project_id: ProjectId,
        root: impl Into<PathBuf>,
        selected_terminal_id: Option<String>,
        project_panel_visible: bool,
        project_panel_width: f32,
    ) -> bool {
        if self.sessions.contains_key(&project_id) {
            return false;
        }
        self.sessions.insert(
            project_id.clone(),
            ProjectWorkItemSession::new(
                project_id,
                root,
                selected_terminal_id,
                project_panel_visible,
                project_panel_width,
            ),
        );
        true
    }

    pub fn close_project(&mut self, project_id: &ProjectId) -> Option<ProjectWorkItemSession> {
        self.sessions.remove(project_id)
    }

    pub fn session(&self, project_id: &ProjectId) -> Option<&ProjectWorkItemSession> {
        self.sessions.get(project_id)
    }

    pub fn session_mut(&mut self, project_id: &ProjectId) -> Option<&mut ProjectWorkItemSession> {
        self.sessions.get_mut(project_id)
    }

    pub fn relocate_file(&mut self, old: &DocumentId, new: DocumentId) -> bool {
        if old == &new {
            return self
                .sessions
                .get(&old.project_id)
                .is_some_and(|session| session.file_ids.contains(old));
        }
        if old.project_id == new.project_id {
            return self
                .sessions
                .get_mut(&old.project_id)
                .is_some_and(|session| session.relocate_file_within_session(old, new));
        }
        let source_exists = self
            .sessions
            .get(&old.project_id)
            .is_some_and(|session| session.file_ids.contains(old));
        let destination_available = self
            .sessions
            .get(&new.project_id)
            .is_some_and(|session| !session.file_ids.contains(&new));
        if !source_exists || !destination_available {
            return false;
        }

        if let Some(source) = self.sessions.get_mut(&old.project_id) {
            let old_item = WorkItemId::File(old.clone());
            source.file_ids.retain(|id| id != old);
            source.activation_history.retain(|item| item != &old_item);
            if source.active_work_item.as_ref() == Some(&old_item) {
                source.active_work_item = source.activation_history.last().cloned();
            }
        }
        if let Some(destination) = self.sessions.get_mut(&new.project_id) {
            destination.file_ids.push(new.clone());
            destination.activate(WorkItemId::File(new));
        }
        true
    }

    pub fn len(&self) -> usize {
        self.sessions.len()
    }

    pub fn is_empty(&self) -> bool {
        self.sessions.is_empty()
    }
}
