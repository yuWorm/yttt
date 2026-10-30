use super::*;

impl WorkbenchView {
    pub(super) fn activate_work_area_group(
        &mut self,
        group_id: TabGroupId,
    ) -> Result<bool, WorkbenchError> {
        let Some(project_id) = self.workspace.selected_project_id().cloned() else {
            return Ok(false);
        };
        let previous = self.active_work_item();
        let Some(active_group) = self
            .project
            .project_editor_runtime
            .workspace()
            .session(&project_id)
            .map(|session| session.active_group_id())
        else {
            return Ok(false);
        };
        if active_group == group_id {
            return Ok(false);
        }
        self.apply_workspace_intent(WorkspaceIntent::ActivateGroup {
            project_id,
            group_id,
        })?;
        let next = self.active_work_item();
        self.apply_work_area_active_change(previous, next)
    }

    pub(super) fn focus_adjacent_work_area_group(
        &mut self,
        edge: WorkAreaDropEdge,
    ) -> Result<bool, WorkbenchError> {
        let Some(project_id) = self.workspace.selected_project_id() else {
            return Ok(false);
        };
        let Some(group_id) = self
            .project
            .project_editor_runtime
            .workspace()
            .session(project_id)
            .and_then(|session| session.adjacent_group_id(edge))
        else {
            return Ok(false);
        };
        self.activate_work_area_group(group_id)
    }

    pub(super) fn move_dragged_work_item_tab(
        &mut self,
        dragged: &DraggedWorkbenchTab,
        target_group: TabGroupId,
        target_index: usize,
    ) -> Result<bool, WorkbenchError> {
        self.work_area_drop_target = None;
        self.move_work_item_between_groups(
            dragged,
            target_group,
            WorkAreaDropPlacement::TabIndex(target_index),
        )
    }

    pub(super) fn drop_work_item_on_group(
        &mut self,
        dragged: &DraggedWorkbenchTab,
        target_group: TabGroupId,
    ) -> Result<bool, WorkbenchError> {
        let placement = self
            .work_area_drop_target
            .take()
            .filter(|target| {
                target.project_id == dragged.project_id && target.group_id == target_group
            })
            .and_then(|target| target.edge)
            .map(WorkAreaDropPlacement::Edge)
            .unwrap_or(WorkAreaDropPlacement::Center);
        self.move_work_item_between_groups(dragged, target_group, placement)
    }

    fn move_work_item_between_groups(
        &mut self,
        dragged: &DraggedWorkbenchTab,
        target_group: TabGroupId,
        placement: WorkAreaDropPlacement,
    ) -> Result<bool, WorkbenchError> {
        if self.workspace.selected_project_id() != Some(&dragged.project_id) {
            return Ok(false);
        }
        let previous = self.active_work_item();
        let before = self.project.project_editor_runtime.workspace().snapshot();
        match self.apply_workspace_intent(WorkspaceIntent::MoveItem {
            project_id: dragged.project_id.clone(),
            item: dragged.id.clone(),
            source_group: dragged.source_group_id,
            target_group,
            placement,
        }) {
            Ok(_) => {}
            Err(WorkbenchError::Intent(IntentError::WorkItemNotFound(_))) => return Ok(false),
            Err(error) => return Err(error),
        }
        if self.project.project_editor_runtime.workspace().snapshot() == before {
            return Ok(false);
        }
        let next = self.active_work_item();
        self.apply_work_area_active_change(previous, next)
    }

    pub(super) fn focus_work_area_terminal_pane(
        &mut self,
        group_id: TabGroupId,
        project_id: &ProjectId,
        tab_id: &str,
        pane_id: &str,
    ) -> Result<(), WorkbenchError> {
        if self.workspace.selected_project_id() != Some(project_id) {
            return Ok(());
        }
        self.activate_work_area_group(group_id)?;
        self.apply_workspace_intent(WorkspaceIntent::ActivateItem {
            project_id: project_id.clone(),
            item: WorkItemId::Terminal(tab_id.to_string()),
        })?;
        self.apply_workspace_intent(WorkspaceIntent::FocusPane {
            project_id: project_id.clone(),
            tab_id: tab_id.to_string(),
            pane_id: pane_id.to_string(),
        })?;
        self.queue_terminal_focus_target(
            project_id.clone(),
            tab_id.to_string(),
            pane_id.to_string(),
        );
        self.sync_input_owner_state();
        Ok(())
    }

    pub(super) fn update_work_area_drop_target(
        &mut self,
        group_id: TabGroupId,
        event: &DragMoveEvent<DraggedWorkbenchTab>,
        cx: &mut Context<Self>,
    ) {
        let dragged = event.drag(cx);
        let Some(project_id) = self.workspace.selected_project_id().cloned() else {
            return;
        };
        if project_id != dragged.project_id {
            if self.work_area_drop_target.take().is_some() {
                cx.notify();
            }
            return;
        }

        let edge_size = event.bounds.size.width.min(event.bounds.size.height) * 0.2;
        let relative_cursor = Point::new(
            event.event.position.x - event.bounds.left(),
            event.event.position.y - event.bounds.top(),
        );
        let mut edge = if relative_cursor.x < edge_size
            || relative_cursor.x > event.bounds.size.width - edge_size
            || relative_cursor.y < edge_size
            || relative_cursor.y > event.bounds.size.height - edge_size
        {
            [
                WorkAreaDropEdge::Top,
                WorkAreaDropEdge::Right,
                WorkAreaDropEdge::Bottom,
                WorkAreaDropEdge::Left,
            ]
            .into_iter()
            .min_by_key(|edge| match edge {
                WorkAreaDropEdge::Top => relative_cursor.y,
                WorkAreaDropEdge::Right => event.bounds.size.width - relative_cursor.x,
                WorkAreaDropEdge::Bottom => event.bounds.size.height - relative_cursor.y,
                WorkAreaDropEdge::Left => relative_cursor.x,
            })
        } else {
            None
        };

        if dragged.source_group_id == group_id
            && self
                .project
                .project_editor_runtime
                .workspace()
                .session(&project_id)
                .and_then(|session| session.group_items_containing(&dragged.id))
                .is_some_and(|items| items.len() == 1)
        {
            edge = None;
        }

        let target = WorkAreaDropTarget {
            project_id,
            group_id,
            edge,
        };
        if self.work_area_drop_target.as_ref() != Some(&target) {
            self.work_area_drop_target = Some(target);
            cx.notify();
        }
    }

    fn apply_work_area_active_change(
        &mut self,
        previous: Option<WorkItemId>,
        next: Option<WorkItemId>,
    ) -> Result<bool, WorkbenchError> {
        if previous == next {
            return Ok(true);
        }
        if let Some(WorkItemId::File(document_id)) = previous {
            self.queue_focus_change_autosave(document_id);
        }
        if let Some(next) = next {
            self.apply_active_work_item(&next)?;
        } else {
            self.sync_input_owner_state();
        }
        Ok(true)
    }
}
