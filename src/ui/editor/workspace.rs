use std::{collections::HashMap, path::PathBuf};

use crate::{model::ids::ProjectId, ui::project_tree::ProjectFileTree};

use super::work_area::{
    TabGroupId, WorkAreaDropPlacement, WorkAreaNode, WorkAreaSplitId, WorkAreaState,
};

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
    work_area: WorkAreaState,
    activation_history: Vec<WorkItemId>,
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
            work_area: WorkAreaState::new(active_work_item),
            activation_history,
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
        self.work_area.active_item()
    }

    pub fn active_group_id(&self) -> TabGroupId {
        self.work_area.active_group_id()
    }

    pub fn active_group_items(&self) -> &[WorkItemId] {
        self.work_area.active_group_items()
    }

    pub fn group_items_containing(&self, item: &WorkItemId) -> Option<&[WorkItemId]> {
        self.work_area.group_items_containing(item)
    }

    pub fn work_area(&self) -> &WorkAreaNode {
        self.work_area.root()
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
            self.work_area.append_item_to_active_group(item.clone());
        }
        self.activate(item);
        id
    }

    pub fn ordered_items(&self, terminal_ids: &[String]) -> Vec<WorkItemId> {
        self.work_area.ordered_items(terminal_ids, &self.file_ids)
    }

    pub fn move_work_item(
        &mut self,
        item: &WorkItemId,
        to_index: usize,
        terminal_ids: &[String],
    ) -> bool {
        self.reconcile_work_area(terminal_ids);
        let Some(source_group) = self.work_area.group_id_containing(item) else {
            return false;
        };
        self.work_area.move_item(
            item,
            source_group,
            source_group,
            WorkAreaDropPlacement::TabIndex(to_index),
        )
    }

    pub fn move_work_item_to_group(
        &mut self,
        item: &WorkItemId,
        source_group: TabGroupId,
        target_group: TabGroupId,
        to_index: usize,
        terminal_ids: &[String],
    ) -> bool {
        self.reconcile_work_area(terminal_ids);
        let changed = self.work_area.move_item(
            item,
            source_group,
            target_group,
            WorkAreaDropPlacement::TabIndex(to_index),
        );
        if changed {
            self.record_active_item();
        }
        changed
    }

    pub fn drop_work_item(
        &mut self,
        item: &WorkItemId,
        source_group: TabGroupId,
        target_group: TabGroupId,
        placement: WorkAreaDropPlacement,
        terminal_ids: &[String],
    ) -> bool {
        self.reconcile_work_area(terminal_ids);
        let changed = self
            .work_area
            .move_item(item, source_group, target_group, placement);
        if changed {
            self.record_active_item();
        }
        changed
    }

    pub fn resize_work_area_split(&mut self, split_id: WorkAreaSplitId, delta: f32) -> Option<f32> {
        self.work_area.resize_split(split_id, delta)
    }

    pub fn reconcile_work_item_order(&mut self, terminal_ids: &[String]) {
        self.reconcile_work_area(terminal_ids);
    }

    pub fn reconcile_work_area(&mut self, terminal_ids: &[String]) {
        self.work_area.reconcile(terminal_ids, &self.file_ids);
        let available = self.ordered_items(terminal_ids);
        self.activation_history
            .retain(|item| available.contains(item));
        self.record_active_item();
    }

    pub fn select_work_item(&mut self, item: WorkItemId, terminal_ids: &[String]) -> bool {
        let exists = match &item {
            WorkItemId::Terminal(id) => terminal_ids.contains(id),
            WorkItemId::File(id) => self.file_ids.contains(id),
        };
        if !exists {
            return false;
        }
        self.reconcile_work_area(terminal_ids);
        self.activate(item)
    }

    pub fn activate_group(&mut self, group_id: TabGroupId) -> Option<WorkItemId> {
        let active = self.work_area.activate_group(group_id)?;
        self.record_active_item();
        Some(active)
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
        if !self.file_ids.contains(document_id) {
            return self.active_work_item().cloned();
        }
        self.reconcile_work_area(terminal_ids);
        self.file_ids.retain(|id| id != document_id);
        self.activation_history.retain(|item| item != &closing);
        self.work_area.remove_item(&closing);
        self.record_active_item();
        self.active_work_item().cloned()
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
        self.work_area.replace_item(&old_item, &new_item);
        for item in &mut self.activation_history {
            if item == &old_item {
                *item = new_item.clone();
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
        self.reconcile_work_area(terminal_ids);
        let items = self.active_group_items().to_vec();
        if items.is_empty() {
            return None;
        }
        let index = match self
            .active_work_item()
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

    fn activate(&mut self, item: WorkItemId) -> bool {
        if !self.work_area.activate_item(&item) {
            return false;
        }
        self.activation_history.retain(|existing| existing != &item);
        self.activation_history.push(item);
        true
    }

    fn record_active_item(&mut self) {
        let Some(active) = self.active_work_item().cloned() else {
            return;
        };
        self.activation_history
            .retain(|existing| existing != &active);
        self.activation_history.push(active);
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
            source.work_area.remove_item(&old_item);
            source.record_active_item();
        }
        if let Some(destination) = self.sessions.get_mut(&new.project_id) {
            let new_item = WorkItemId::File(new.clone());
            destination.file_ids.push(new);
            destination
                .work_area
                .append_item_to_active_group(new_item.clone());
            destination.activate(new_item);
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
