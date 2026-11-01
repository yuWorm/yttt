use super::*;
use crate::model::workspace::WorkspaceNavigation;

impl WorkbenchView {
    pub fn command_center_is_active(&self) -> bool {
        matches!(
            self.workspace.navigation(),
            WorkspaceNavigation::CommandCenter
        )
    }

    pub fn select_command_center(&mut self) {
        if self.workspace.opened_projects().is_empty() {
            return;
        }
        if let Some(project_id) = self.workspace.selected_project_id().cloned() {
            self.command_center.previous_project = Some(project_id);
        }
        if let Some(WorkItemId::File(document_id)) = self.active_work_item() {
            self.queue_focus_change_autosave(document_id);
        }
        if let Err(error) = self.apply_workspace_intent(WorkspaceIntent::SelectCommandCenter) {
            self.load_error = Some(error.to_string());
            return;
        }
        self.command_center.pending_terminal_termination = None;
        self.active_sidebar_resize_drag = None;
        self.active_split_resize_drag = None;
        self.active_work_area_resize_drag = None;
        self.work_area_drop_target = None;
        self.queue_projects_focus();
        self.sync_input_owner_state();
    }

    pub(super) fn on_command_center_toggle(
        &mut self,
        _: &CommandCenterToggle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.command_center_is_active() {
            let project_id = self
                .command_center
                .previous_project
                .as_ref()
                .filter(|id| self.workspace.project(id).is_some())
                .cloned()
                .or_else(|| {
                    self.workspace
                        .opened_projects()
                        .first()
                        .map(|project| project.id.clone())
                });
            if let Some(project_id) = project_id {
                if let Err(error) = self.select_project(&project_id) {
                    self.load_error = Some(error.to_string());
                }
                window.blur(cx);
            }
        } else {
            self.select_command_center();
            self.workbench_focus_handle(cx).focus(window, cx);
        }
        cx.notify();
    }
}
