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
        let next = {
            let Some(session) = self
                .project
                .project_editor_runtime
                .workspace_mut()
                .session_mut(&project_id)
            else {
                return Ok(false);
            };
            if session.active_group_id() == group_id {
                return Ok(false);
            }
            session.activate_group(group_id)
        };
        self.apply_work_area_active_change(previous, next)
    }

    pub(super) fn move_dragged_work_item_tab(
        &mut self,
        dragged: &DraggedWorkbenchTab,
        target_group: TabGroupId,
        target_index: usize,
    ) -> Result<bool, WorkbenchError> {
        self.work_area_drop_target = None;
        let Some((project_id, terminal_ids)) = self.selected_project_work_item_ids() else {
            return Ok(false);
        };
        if project_id != dragged.project_id {
            return Ok(false);
        }
        let previous = self.active_work_item();
        let changed = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
            .is_some_and(|session| {
                session.move_work_item_to_group(
                    &dragged.id,
                    dragged.source_group_id,
                    target_group,
                    target_index,
                    &terminal_ids,
                )
            });
        if !changed {
            return Ok(false);
        }
        let next = self.active_work_item();
        self.apply_work_area_active_change(previous, next)
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
        let Some((project_id, terminal_ids)) = self.selected_project_work_item_ids() else {
            return Ok(false);
        };
        if project_id != dragged.project_id {
            return Ok(false);
        }
        let previous = self.active_work_item();
        let changed = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
            .is_some_and(|session| {
                session.drop_work_item(
                    &dragged.id,
                    dragged.source_group_id,
                    target_group,
                    placement,
                    &terminal_ids,
                )
            });
        if !changed {
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
        self.workspace.select_tab(tab_id)?;
        self.workspace.focus_pane(pane_id)?;
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

    pub(super) fn reconcile_project_work_area(&mut self, project_id: &ProjectId) {
        let Some(terminal_ids) = self.workspace.project(project_id).map(|project| {
            project
                .layout
                .tabs
                .iter()
                .map(|tab| tab.id.clone())
                .collect::<Vec<_>>()
        }) else {
            return;
        };
        if let Some(session) = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)
        {
            session.reconcile_work_area(&terminal_ids);
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
