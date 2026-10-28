use std::collections::HashSet;

use yttt_agent_core::AgentSnapshot;

use crate::model::{
    ids::{ProjectId, ProjectInstanceId},
    layout::{
        LayoutNode, PaneConfig, PaneKind, ProcessExitBehavior, ProjectLayout, SplitConfig,
        SplitDirection, TabConfig, TabStartup, TerminalExecutionMode,
    },
    project::{ProjectDescriptor, ProjectLocation},
    split_tree::{FocusDirection, ResizeDirection},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum WorkspaceNavigation {
    #[default]
    Dashboard,
    CommandCenter,
    Project(ProjectId),
}

#[derive(Clone, Debug, Default)]
pub struct Workspace {
    opened_projects: Vec<OpenedProject>,
    navigation: WorkspaceNavigation,
}

/// The validated, serializable state needed to restore one workspace exactly.
///
/// This deliberately excludes runtime entities. Callers must validate it through
/// [`Workspace::restore_persisted_state`] before making it live.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct WorkspaceState {
    pub opened_projects: Vec<OpenedProject>,
    pub selected_project_id: Option<ProjectId>,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn persisted_state(&self) -> WorkspaceState {
        WorkspaceState {
            opened_projects: self.opened_projects.clone(),
            selected_project_id: self.selected_project_id().cloned(),
        }
    }

    /// Captures the manifest without copying Agent histories, which are separate objects.
    pub fn persisted_layout_state(&self) -> WorkspaceState {
        WorkspaceState {
            selected_project_id: self.selected_project_id().cloned(),
            opened_projects: self
                .opened_projects
                .iter()
                .map(|project| OpenedProject {
                    id: project.id.clone(),
                    instance_id: project.instance_id.clone(),
                    location: project.location.clone(),
                    layout: project.layout.clone(),
                    selected_tab_id: project.selected_tab_id.clone(),
                    tab_states: project
                        .tab_states
                        .iter()
                        .map(|tab| TabState {
                            tab_id: tab.tab_id.clone(),
                            start_state: tab.start_state,
                            focused_pane_id: tab.focused_pane_id.clone(),
                            pane_states: tab
                                .pane_states
                                .iter()
                                .map(|pane| PaneState {
                                    pane_id: pane.pane_id.clone(),
                                    process_state: pane.process_state,
                                    agent_snapshot: None,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    pub fn restore_persisted_state(state: WorkspaceState) -> Result<Self, WorkspaceRestoreError> {
        let mut project_ids = HashSet::new();
        for project in &state.opened_projects {
            if !project_ids.insert(project.id.clone()) {
                return Err(WorkspaceRestoreError::DuplicateProject(
                    project.id.as_str().to_string(),
                ));
            }
            project
                .layout
                .validate()
                .map_err(WorkspaceRestoreError::InvalidLayout)?;
            validate_restored_project(project)?;
        }
        if let Some(selected_project_id) = &state.selected_project_id
            && !project_ids.contains(selected_project_id)
        {
            return Err(WorkspaceRestoreError::SelectedProjectMissing(
                selected_project_id.as_str().to_string(),
            ));
        }
        let navigation = match state.selected_project_id {
            Some(project_id) => WorkspaceNavigation::Project(project_id),
            None if state.opened_projects.is_empty() => WorkspaceNavigation::Dashboard,
            None => WorkspaceNavigation::CommandCenter,
        };
        Ok(Self {
            opened_projects: state.opened_projects,
            navigation,
        })
    }

    /// Reconciles persisted pane activity with the authoritative Host catalog.
    ///
    /// Missing running panes retain their place and become pending recovery.
    /// Exited Agent panes with saved sessions also become pending recovery.
    /// Other exited panes and never-started panes are not restarted.
    pub fn reconcile_host_resources(
        &mut self,
        available_terminal_sessions: &HashSet<String>,
    ) -> Vec<RemoteResourceLoss> {
        let mut losses = Vec::new();
        for project in &mut self.opened_projects {
            for tab in &mut project.tab_states {
                for pane in &mut tab.pane_states {
                    let session_id = format!("{}:{}:{}", project.id, tab.tab_id, pane.pane_id);
                    if available_terminal_sessions.contains(&session_id) {
                        pane.process_state = PaneProcessState::Running;
                        continue;
                    }
                    let was_running = pane.process_state == PaneProcessState::Running;
                    let has_exited_agent_session = pane.process_state == PaneProcessState::Exited
                        && pane
                            .agent_snapshot
                            .as_ref()
                            .is_some_and(|snapshot| snapshot.session.is_some());
                    if was_running || has_exited_agent_session {
                        pane.process_state = PaneProcessState::Restoring;
                        if let Some(snapshot) = pane.agent_snapshot.as_mut() {
                            snapshot.mark_disconnected();
                        }
                        if was_running {
                            losses.push(RemoteResourceLoss {
                                project_id: project.id.clone(),
                                tab_id: tab.tab_id.clone(),
                                pane_id: pane.pane_id.clone(),
                            });
                        }
                    }
                }
            }
        }
        losses
    }

    pub fn opened_projects(&self) -> &[OpenedProject] {
        &self.opened_projects
    }

    pub fn selected_project_id(&self) -> Option<&ProjectId> {
        match &self.navigation {
            WorkspaceNavigation::Dashboard | WorkspaceNavigation::CommandCenter => None,
            WorkspaceNavigation::Project(project_id) => Some(project_id),
        }
    }

    pub fn navigation(&self) -> &WorkspaceNavigation {
        &self.navigation
    }

    pub fn select_command_center(&mut self) {
        if !self.opened_projects.is_empty() {
            self.navigation = WorkspaceNavigation::CommandCenter;
        }
    }

    pub fn project(&self, project_id: &ProjectId) -> Option<&OpenedProject> {
        self.opened_projects
            .iter()
            .find(|project| &project.id == project_id)
    }

    pub fn open_project(
        &mut self,
        descriptor: ProjectDescriptor,
        layout: ProjectLayout,
    ) -> Result<ProjectId, WorkspaceError> {
        layout.validate().map_err(WorkspaceError::InvalidLayout)?;

        let ProjectDescriptor {
            id,
            instance_id,
            location,
        } = descriptor;
        if self.project(&id).is_some() {
            self.navigation = WorkspaceNavigation::Project(id.clone());
            return Ok(id);
        }
        let selected_tab_id = default_tab_id(&layout).unwrap_or_default();
        let tab_states = tab_states_for_layout(&layout, &selected_tab_id);

        self.opened_projects.push(OpenedProject {
            id: id.clone(),
            instance_id,
            location,
            layout,
            selected_tab_id,
            tab_states,
        });
        self.navigation = WorkspaceNavigation::Project(id.clone());

        Ok(id)
    }

    pub fn replace_selected_project_layout(
        &mut self,
        layout: ProjectLayout,
    ) -> Result<(), WorkspaceError> {
        self.selected_project_mut()?.replace_layout(layout)
    }

    pub fn select_project(&mut self, project_id: &ProjectId) -> Result<(), WorkspaceError> {
        if self.project(project_id).is_none() {
            return Err(WorkspaceError::ProjectNotFound(
                project_id.as_str().to_string(),
            ));
        }

        self.navigation = WorkspaceNavigation::Project(project_id.clone());
        Ok(())
    }

    pub fn select_tab(&mut self, tab_id: &str) -> Result<(), WorkspaceError> {
        self.selected_project_mut()?.select_tab(tab_id)
    }

    pub fn focus_pane(&mut self, pane_id: &str) -> Result<(), WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        project.focus_pane(&selected_tab_id, pane_id)
    }

    pub fn focus_pane_direction(
        &mut self,
        direction: FocusDirection,
    ) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        project.focus_adjacent_pane(&selected_tab_id, direction)
    }

    pub fn resize_focused_split(
        &mut self,
        direction: ResizeDirection,
        delta: f32,
    ) -> Result<f32, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let focused_pane_id = project.focused_or_first_pane_id(&selected_tab_id)?;
        project.resize_pane_split(&selected_tab_id, &focused_pane_id, direction, delta)
    }

    pub fn select_next_tab(&mut self) -> Result<(), WorkspaceError> {
        self.select_relative_tab(1)
    }

    pub fn select_prev_tab(&mut self) -> Result<(), WorkspaceError> {
        self.select_relative_tab(-1)
    }

    pub fn create_shell_tab(&mut self) -> Result<String, WorkspaceError> {
        self.create_shell_tab_with_command("")
    }

    pub fn create_shell_tab_with_command(
        &mut self,
        command: impl Into<String>,
    ) -> Result<String, WorkspaceError> {
        let (tab_id, _) = next_tab_identity(&self.selected_project_mut()?.layout);
        self.create_shell_tab_with_id(tab_id, command)
    }

    pub fn create_shell_tab_with_id(
        &mut self,
        tab_id: String,
        command: impl Into<String>,
    ) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let (_, title) = next_tab_identity(&project.layout);
        project.add_tab(shell_tab_config(tab_id, title, command.into()))
    }

    pub fn create_agent_tab(
        &mut self,
        title: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
    ) -> Result<String, WorkspaceError> {
        let (tab_id, _) = next_tab_identity(&self.selected_project_mut()?.layout);
        self.create_agent_tab_with_id(tab_id, title, command, args)
    }

    pub fn create_agent_tab_with_id(
        &mut self,
        tab_id: String,
        title: impl Into<String>,
        command: impl Into<String>,
        args: Vec<String>,
    ) -> Result<String, WorkspaceError> {
        self.selected_project_mut()?.add_tab(agent_tab_config(
            tab_id,
            title.into(),
            command.into(),
            args,
        ))
    }

    pub fn close_selected_tab(&mut self) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        if project.layout.tabs.len() <= 1 {
            return Err(WorkspaceError::CannotCloseLastTab);
        }
        let selected_tab_id = project.selected_tab_id.clone();
        if project.layout.tab(&selected_tab_id).is_none() {
            return Err(WorkspaceError::TabNotFound(selected_tab_id));
        }
        project.close_tabs(std::slice::from_ref(&selected_tab_id))?;
        Ok(selected_tab_id)
    }

    pub fn close_tabs(&mut self, tab_ids: &[String]) -> Result<Vec<String>, WorkspaceError> {
        if tab_ids.is_empty() {
            return Ok(Vec::new());
        }
        self.selected_project_mut()?.close_tabs(tab_ids)
    }

    pub fn rename_selected_tab(&mut self, title: &str) -> Result<(), WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        project.rename_tab(&selected_tab_id, title)
    }

    pub fn split_focused_pane(
        &mut self,
        direction: SplitDirection,
    ) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let tab = project
            .layout
            .tab(&project.selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(project.selected_tab_id.clone()))?;
        let pane_id = next_pane_id(&tab.layout);
        self.split_focused_pane_with_id(direction, pane_id, String::new())
    }

    pub fn split_focused_pane_with_id(
        &mut self,
        direction: SplitDirection,
        new_pane_id: String,
        command: String,
    ) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let focused_pane_id = project.focused_or_first_pane_id(&selected_tab_id)?;
        project.split_pane(
            &selected_tab_id,
            &focused_pane_id,
            direction,
            new_pane_id,
            command,
        )
    }

    pub fn close_focused_pane(&mut self) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let focused_pane_id = project.focused_or_first_pane_id(&selected_tab_id)?;
        project.close_pane(&selected_tab_id, &focused_pane_id)?;
        Ok(focused_pane_id)
    }

    pub fn close_pane_for_exit(
        &mut self,
        project_id: &ProjectId,
        tab_id: &str,
        pane_id: &str,
    ) -> Result<PaneExitCloseOutcome, WorkspaceError> {
        let project = self.project_mut(project_id)?;
        let tab_index = project
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;

        if project.layout.tabs[tab_index]
            .layout
            .find_pane(pane_id)
            .is_none()
        {
            return Err(WorkspaceError::PaneNotFound(pane_id.to_string()));
        }

        if pane_count(&project.layout.tabs[tab_index].layout) > 1 {
            if !remove_pane_node(&mut project.layout.tabs[tab_index].layout, pane_id) {
                return Err(WorkspaceError::PaneNotFound(pane_id.to_string()));
            }

            let remaining_panes = pane_ids(&project.layout.tabs[tab_index].layout);
            let tab_state = project
                .tab_state_mut(tab_id)
                .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
            tab_state.pane_states.retain(|pane| pane.pane_id != pane_id);
            if tab_state.focused_pane_id.as_deref() == Some(pane_id)
                || tab_state
                    .focused_pane_id
                    .as_ref()
                    .is_none_or(|focused| !remaining_panes.iter().any(|pane| pane == focused))
            {
                tab_state.focused_pane_id = remaining_panes.first().cloned();
            }

            return Ok(PaneExitCloseOutcome::PaneClosed);
        }

        let removed_tab_id = project.layout.tabs.remove(tab_index).id;
        project
            .tab_states
            .retain(|tab| tab.tab_id != removed_tab_id);
        repair_default_tab_after_removal(&mut project.layout);

        if project.layout.tabs.is_empty() {
            project.selected_tab_id.clear();
            return Ok(PaneExitCloseOutcome::ProjectEmptied);
        }

        if project.selected_tab_id == removed_tab_id {
            let next_index = tab_index.min(project.layout.tabs.len().saturating_sub(1));
            let next_tab_id = project.layout.tabs[next_index].id.clone();
            project.select_tab(&next_tab_id)?;
        }

        Ok(PaneExitCloseOutcome::TabClosed)
    }

    pub fn rename_focused_pane(&mut self, title: &str) -> Result<(), WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let focused_pane_id = project.focused_or_first_pane_id(&selected_tab_id)?;
        project.rename_pane(&selected_tab_id, &focused_pane_id, title)
    }

    pub fn mark_pane_running(
        &mut self,
        project_id: &ProjectId,
        tab_id: &str,
        pane_id: &str,
    ) -> Result<(), WorkspaceError> {
        let project = self
            .opened_projects
            .iter_mut()
            .find(|project| &project.id == project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        let tab = project
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        let pane = tab
            .pane_state_mut(pane_id)
            .ok_or_else(|| WorkspaceError::PaneNotFound(pane_id.to_string()))?;

        pane.process_state = PaneProcessState::Running;
        Ok(())
    }

    pub fn record_agent_snapshot(
        &mut self,
        project_id: &ProjectId,
        tab_id: &str,
        pane_id: &str,
        snapshot: AgentSnapshot,
    ) -> Result<(), WorkspaceError> {
        let project = self
            .opened_projects
            .iter_mut()
            .find(|project| &project.id == project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        let tab = project
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        let pane = tab
            .pane_state_mut(pane_id)
            .ok_or_else(|| WorkspaceError::PaneNotFound(pane_id.to_string()))?;

        pane.agent_snapshot = Some(snapshot);
        Ok(())
    }
    pub fn clear_agent_snapshot(
        &mut self,
        project_id: &ProjectId,
        tab_id: &str,
        pane_id: &str,
    ) -> Result<(), WorkspaceError> {
        let project = self
            .opened_projects
            .iter_mut()
            .find(|project| &project.id == project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        let tab = project
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        let pane = tab
            .pane_state_mut(pane_id)
            .ok_or_else(|| WorkspaceError::PaneNotFound(pane_id.to_string()))?;

        pane.agent_snapshot = None;
        Ok(())
    }

    pub fn decay_stale_agent_activity(&mut self, now: u64, stale_after_millis: u64) -> usize {
        let mut changed = 0;
        for project in &mut self.opened_projects {
            for tab in &mut project.tab_states {
                for pane in &mut tab.pane_states {
                    if pane.agent_snapshot.as_mut().is_some_and(|snapshot| {
                        snapshot.decay_stale_activity(now, stale_after_millis)
                    }) {
                        changed += 1;
                    }
                }
            }
        }
        changed
    }

    pub fn record_pane_exited(
        &mut self,
        project_id: &ProjectId,
        tab_id: &str,
        pane_id: &str,
    ) -> Result<(), WorkspaceError> {
        let project = self
            .opened_projects
            .iter_mut()
            .find(|project| &project.id == project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        let tab = project
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        let pane = tab
            .pane_state_mut(pane_id)
            .ok_or_else(|| WorkspaceError::PaneNotFound(pane_id.to_string()))?;

        pane.process_state = PaneProcessState::Exited;
        if let Some(snapshot) = pane.agent_snapshot.as_mut() {
            snapshot.mark_disconnected();
        }
        Ok(())
    }

    pub fn close_project(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<ClosedProject, CloseProjectError> {
        let index = self.project_index(project_id)?;

        if self.opened_projects[index].has_running_panes() {
            return Err(CloseProjectError::RunningProcesses);
        }

        Ok(self.remove_project_at(index))
    }

    pub fn request_close_project(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<CloseProjectDecision, CloseProjectError> {
        let index = self.project_index(project_id)?;
        let running_pane_count = self.opened_projects[index].running_pane_count();
        if running_pane_count > 0 {
            return Ok(CloseProjectDecision::NeedsConfirmation {
                project_id: project_id.clone(),
                running_pane_count,
            });
        }

        Ok(CloseProjectDecision::Closed(self.remove_project_at(index)))
    }

    pub fn confirm_close_project(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<ClosedProject, CloseProjectError> {
        let index = self.project_index(project_id)?;

        Ok(self.remove_project_at(index))
    }

    fn select_relative_tab(&mut self, offset: isize) -> Result<(), WorkspaceError> {
        let project = self.selected_project_mut()?;
        let tab_count = project.layout.tabs.len();
        if tab_count == 0 {
            return Err(WorkspaceError::NoTabs);
        }

        let current_index = project
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == project.selected_tab_id)
            .unwrap_or(0);
        let next_index = (current_index as isize + offset).rem_euclid(tab_count as isize) as usize;
        let next_tab_id = project.layout.tabs[next_index].id.clone();

        project.select_tab(&next_tab_id)
    }

    fn selected_project_mut(&mut self) -> Result<&mut OpenedProject, WorkspaceError> {
        let project_id = self
            .selected_project_id()
            .cloned()
            .ok_or(WorkspaceError::NoSelectedProject)?;

        self.project_mut(&project_id)
    }

    pub(crate) fn project_mut(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<&mut OpenedProject, WorkspaceError> {
        self.opened_projects
            .iter_mut()
            .find(|project| &project.id == project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))
    }

    fn project_index(&self, project_id: &ProjectId) -> Result<usize, CloseProjectError> {
        self.opened_projects
            .iter()
            .position(|project| &project.id == project_id)
            .ok_or_else(|| CloseProjectError::ProjectNotFound(project_id.as_str().to_string()))
    }

    fn remove_project_at(&mut self, index: usize) -> ClosedProject {
        let project = self.opened_projects.remove(index);
        if self.opened_projects.is_empty() {
            self.navigation = WorkspaceNavigation::Dashboard;
        } else if self.selected_project_id() == Some(&project.id) {
            self.navigation = self
                .opened_projects
                .first()
                .map(|project| WorkspaceNavigation::Project(project.id.clone()))
                .unwrap_or_default();
        }

        ClosedProject {
            project_id: project.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct OpenedProject {
    pub id: ProjectId,
    pub instance_id: ProjectInstanceId,
    pub location: ProjectLocation,
    pub layout: ProjectLayout,
    pub selected_tab_id: String,
    pub tab_states: Vec<TabState>,
}

impl OpenedProject {
    pub fn tab_state(&self, tab_id: &str) -> Option<&TabState> {
        self.tab_states.iter().find(|tab| tab.tab_id == tab_id)
    }

    fn tab_state_mut(&mut self, tab_id: &str) -> Option<&mut TabState> {
        self.tab_states.iter_mut().find(|tab| tab.tab_id == tab_id)
    }

    fn has_running_panes(&self) -> bool {
        self.running_pane_count() > 0
    }

    fn running_pane_count(&self) -> usize {
        self.tab_states
            .iter()
            .map(|tab| {
                tab.pane_states
                    .iter()
                    .filter(|pane| pane.process_state == PaneProcessState::Running)
                    .count()
            })
            .sum()
    }

    fn tab_index(&self, tab_id: &str) -> Result<usize, WorkspaceError> {
        self.layout
            .tabs
            .iter()
            .position(|tab| tab.id == tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))
    }

    /// The tab's focused pane, falling back to its first pane.
    pub(crate) fn focused_or_first_pane_id(&self, tab_id: &str) -> Result<String, WorkspaceError> {
        let tab_index = self.tab_index(tab_id)?;
        self.tab_state(tab_id)
            .and_then(|tab| tab.focused_pane_id.clone())
            .or_else(|| first_pane_id(&self.layout.tabs[tab_index].layout))
            .ok_or_else(|| WorkspaceError::PaneNotFound("focused".to_string()))
    }

    /// Selects a tab, starts it and gives it a focused pane when it has none.
    pub(crate) fn select_tab(&mut self, tab_id: &str) -> Result<(), WorkspaceError> {
        self.tab_index(tab_id)?;
        let tab_state = self
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        tab_state.start_state = TabStartState::Started;
        if tab_state.focused_pane_id.is_none() {
            tab_state.focused_pane_id = tab_state
                .pane_states
                .first()
                .map(|pane| pane.pane_id.clone());
        }
        self.selected_tab_id = tab_id.to_string();
        Ok(())
    }

    pub(crate) fn focus_pane(&mut self, tab_id: &str, pane_id: &str) -> Result<(), WorkspaceError> {
        let tab_index = self.tab_index(tab_id)?;
        if self.layout.tabs[tab_index]
            .layout
            .find_pane(pane_id)
            .is_none()
        {
            return Err(WorkspaceError::PaneNotFound(pane_id.to_string()));
        }
        let tab_state = self
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        if !tab_state
            .pane_states
            .iter()
            .any(|pane| pane.pane_id == pane_id)
        {
            return Err(WorkspaceError::PaneNotFound(pane_id.to_string()));
        }
        tab_state.focused_pane_id = Some(pane_id.to_string());
        Ok(())
    }

    pub(crate) fn focus_adjacent_pane(
        &mut self,
        tab_id: &str,
        direction: FocusDirection,
    ) -> Result<String, WorkspaceError> {
        let tab_index = self.tab_index(tab_id)?;
        let focused_pane_id = self.focused_or_first_pane_id(tab_id)?;
        let next_pane_id = adjacent_pane_id(
            &self.layout.tabs[tab_index].layout,
            &focused_pane_id,
            direction,
        )
        .ok_or_else(|| WorkspaceError::PaneNotFound(format!("{direction:?}")))?;
        let tab_state = self
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        tab_state.focused_pane_id = Some(next_pane_id.clone());
        Ok(next_pane_id)
    }

    pub(crate) fn resize_pane_split(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        direction: ResizeDirection,
        delta: f32,
    ) -> Result<f32, WorkspaceError> {
        let tab_index = self.tab_index(tab_id)?;
        resize_pane_split(
            &mut self.layout.tabs[tab_index].layout,
            pane_id,
            direction,
            delta,
        )
        .ok_or_else(|| WorkspaceError::PaneNotFound(format!("{direction:?}")))
    }

    /// Appends a started tab with idle panes and selects it.
    pub(crate) fn add_tab(&mut self, tab: TabConfig) -> Result<String, WorkspaceError> {
        if self.layout.tab(&tab.id).is_some() {
            return Err(crate::model::layout::LayoutError::DuplicateTabId(tab.id).into());
        }
        let pane_ids = pane_ids(&tab.layout);
        let tab_id = tab.id.clone();
        self.tab_states.push(TabState {
            tab_id: tab_id.clone(),
            start_state: TabStartState::Started,
            focused_pane_id: pane_ids.first().cloned(),
            pane_states: pane_ids
                .into_iter()
                .map(|pane_id| PaneState {
                    pane_id,
                    process_state: PaneProcessState::Idle,
                    agent_snapshot: None,
                })
                .collect(),
        });
        self.layout.tabs.push(tab);
        self.selected_tab_id = tab_id.clone();
        Ok(tab_id)
    }

    /// Removes the named tabs; when the selected tab is removed, the tab that
    /// takes its position becomes selected.
    pub(crate) fn close_tabs(&mut self, tab_ids: &[String]) -> Result<Vec<String>, WorkspaceError> {
        let selected_index = self
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == self.selected_tab_id);
        let selected_removed = tab_ids.contains(&self.selected_tab_id);
        let removed_before_selected = selected_index
            .map(|selected_index| {
                self.layout.tabs[..selected_index]
                    .iter()
                    .filter(|tab| tab_ids.contains(&tab.id))
                    .count()
            })
            .unwrap_or_default();
        let mut removed = Vec::new();
        self.layout.tabs.retain(|tab| {
            if tab_ids.contains(&tab.id) {
                removed.push(tab.id.clone());
                false
            } else {
                true
            }
        });
        if removed.is_empty() {
            return Ok(removed);
        }
        self.tab_states.retain(|tab| !removed.contains(&tab.tab_id));
        repair_default_tab_after_removal(&mut self.layout);

        if selected_removed {
            if self.layout.tabs.is_empty() {
                self.selected_tab_id.clear();
            } else {
                let next_index = selected_index
                    .unwrap_or_default()
                    .saturating_sub(removed_before_selected)
                    .min(self.layout.tabs.len() - 1);
                let next_tab_id = self.layout.tabs[next_index].id.clone();
                self.select_tab(&next_tab_id)?;
            }
        }

        Ok(removed)
    }

    pub(crate) fn rename_tab(&mut self, tab_id: &str, title: &str) -> Result<(), WorkspaceError> {
        let title = normalized_title(title)?;
        let tab_index = self.tab_index(tab_id)?;
        self.layout.tabs[tab_index].title = title;
        Ok(())
    }

    /// Splits `target_pane_id` and focuses the new shell pane.
    pub(crate) fn split_pane(
        &mut self,
        tab_id: &str,
        target_pane_id: &str,
        direction: SplitDirection,
        new_pane_id: String,
        command: String,
    ) -> Result<String, WorkspaceError> {
        let tab_index = self.tab_index(tab_id)?;
        let tab_layout = &mut self.layout.tabs[tab_index].layout;
        if tab_layout.find_pane(&new_pane_id).is_some() {
            return Err(crate::model::layout::LayoutError::DuplicatePaneId {
                tab_id: tab_id.to_string(),
                pane_id: new_pane_id,
            }
            .into());
        }
        let new_pane = PaneConfig {
            id: new_pane_id.clone(),
            title: format!("Pane {}", pane_count(tab_layout) + 1),
            command,
            args: Vec::new(),
            execution_mode: TerminalExecutionMode::Shell,
            exit_behavior: ProcessExitBehavior::Close,
            kind: PaneKind::Shell,
            notify_on_exit: false,
            detector: None,
        };
        if !split_pane_node(tab_layout, target_pane_id, direction, new_pane) {
            return Err(WorkspaceError::PaneNotFound(target_pane_id.to_string()));
        }

        let tab_state = self
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        tab_state.pane_states.push(PaneState {
            pane_id: new_pane_id.clone(),
            process_state: PaneProcessState::Idle,
            agent_snapshot: None,
        });
        tab_state.focused_pane_id = Some(new_pane_id.clone());
        Ok(new_pane_id)
    }

    /// Removes one pane from a multi-pane tab and focuses the first remaining pane.
    pub(crate) fn close_pane(&mut self, tab_id: &str, pane_id: &str) -> Result<(), WorkspaceError> {
        let tab_index = self.tab_index(tab_id)?;
        let tab_layout = &mut self.layout.tabs[tab_index].layout;
        if pane_count(tab_layout) <= 1 {
            return Err(WorkspaceError::CannotCloseLastPane);
        }
        if !remove_pane_node(tab_layout, pane_id) {
            return Err(WorkspaceError::PaneNotFound(pane_id.to_string()));
        }
        let first_remaining = first_pane_id(tab_layout);
        let tab_state = self
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        tab_state.pane_states.retain(|pane| pane.pane_id != pane_id);
        tab_state.focused_pane_id = first_remaining;
        Ok(())
    }

    pub(crate) fn rename_pane(
        &mut self,
        tab_id: &str,
        pane_id: &str,
        title: &str,
    ) -> Result<(), WorkspaceError> {
        let title = normalized_title(title)?;
        let tab_index = self.tab_index(tab_id)?;
        let pane = self.layout.tabs[tab_index]
            .layout
            .find_pane_mut(pane_id)
            .ok_or_else(|| WorkspaceError::PaneNotFound(pane_id.to_string()))?;
        pane.title = title;
        Ok(())
    }

    /// Replaces the layout, keeping the selected tab when it still exists.
    /// Tab and pane runtime state is rebuilt from the new layout.
    pub(crate) fn replace_layout(&mut self, layout: ProjectLayout) -> Result<(), WorkspaceError> {
        layout.validate().map_err(WorkspaceError::InvalidLayout)?;
        let selected_tab_id = if layout.tab(&self.selected_tab_id).is_some() {
            self.selected_tab_id.clone()
        } else {
            default_tab_id(&layout).unwrap_or_default()
        };
        self.tab_states = tab_states_for_layout(&layout, &selected_tab_id);
        self.layout = layout;
        self.selected_tab_id = selected_tab_id;
        Ok(())
    }

    /// The next free `tab-N` ID and its default `Tab N` title.
    pub(crate) fn next_tab_identity(&self) -> (String, String) {
        next_tab_identity(&self.layout)
    }

    /// The next free `pane-N` ID within one tab.
    pub(crate) fn next_pane_id(&self, tab_id: &str) -> Result<String, WorkspaceError> {
        let tab_index = self.tab_index(tab_id)?;
        Ok(next_pane_id(&self.layout.tabs[tab_index].layout))
    }

    pub(crate) fn pane_addresses(&self) -> impl Iterator<Item = (&str, &str)> {
        self.tab_states.iter().flat_map(|tab| {
            tab.pane_states
                .iter()
                .map(|pane| (tab.tab_id.as_str(), pane.pane_id.as_str()))
        })
    }
}

pub(crate) fn shell_tab_config(id: String, title: String, command: String) -> TabConfig {
    TabConfig {
        id,
        title,
        cwd: None,
        startup: TabStartup::Lazy,
        layout: LayoutNode::Pane(PaneConfig {
            id: "shell".to_string(),
            title: "shell".to_string(),
            command,
            args: Vec::new(),
            execution_mode: TerminalExecutionMode::Shell,
            exit_behavior: ProcessExitBehavior::Close,
            kind: PaneKind::Shell,
            notify_on_exit: false,
            detector: None,
        }),
    }
}

pub(crate) fn agent_tab_config(
    id: String,
    title: String,
    command: String,
    args: Vec<String>,
) -> TabConfig {
    TabConfig {
        id,
        title: title.clone(),
        cwd: None,
        startup: TabStartup::Lazy,
        layout: LayoutNode::Pane(PaneConfig {
            id: "agent".to_string(),
            title,
            command,
            args,
            execution_mode: TerminalExecutionMode::Command,
            exit_behavior: ProcessExitBehavior::ManualRestart,
            kind: PaneKind::Agent,
            notify_on_exit: true,
            detector: None,
        }),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseProjectDecision {
    Closed(ClosedProject),
    NeedsConfirmation {
        project_id: ProjectId,
        running_pane_count: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct TabState {
    pub tab_id: String,
    pub start_state: TabStartState,
    pub focused_pane_id: Option<String>,
    pub pane_states: Vec<PaneState>,
}

impl TabState {
    fn pane_state_mut(&mut self, pane_id: &str) -> Option<&mut PaneState> {
        self.pane_states
            .iter_mut()
            .find(|pane| pane.pane_id == pane_id)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct PaneState {
    pub pane_id: String,
    pub process_state: PaneProcessState,
    pub agent_snapshot: Option<AgentSnapshot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum TabStartState {
    Lazy,
    Started,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub enum PaneProcessState {
    Idle,
    Restoring,
    Running,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneExitCloseOutcome {
    PaneKept,
    PaneClosed,
    TabClosed,
    ProjectEmptied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedProject {
    pub project_id: ProjectId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteResourceLoss {
    pub project_id: ProjectId,
    pub tab_id: String,
    pub pane_id: String,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WorkspaceRestoreError {
    #[error("duplicate project in persisted workspace: {0}")]
    DuplicateProject(String),
    #[error("the selected persisted project does not exist: {0}")]
    SelectedProjectMissing(String),
    #[error("project {project_id} has no selected tab")]
    MissingSelectedTab { project_id: String },
    #[error("project {project_id} selected tab does not exist: {tab_id}")]
    SelectedTabMissing { project_id: String, tab_id: String },
    #[error("project {project_id} has invalid persisted tab state: {message}")]
    InvalidTabState { project_id: String, message: String },
    #[error(transparent)]
    InvalidLayout(#[from] crate::model::layout::LayoutError),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum WorkspaceError {
    #[error("{0}")]
    InvalidLayout(#[from] crate::model::layout::LayoutError),
    #[error("project not found: {0}")]
    ProjectNotFound(String),
    #[error("tab not found: {0}")]
    TabNotFound(String),
    #[error("pane not found: {0}")]
    PaneNotFound(String),
    #[error("no selected project")]
    NoSelectedProject,
    #[error("selected project has no tabs")]
    NoTabs,
    #[error("cannot close the last pane in a tab")]
    CannotCloseLastPane,
    #[error("cannot close the last tab in a project")]
    CannotCloseLastTab,
    #[error("title cannot be empty")]
    EmptyTitle,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CloseProjectError {
    #[error("project has running processes")]
    RunningProcesses,
    #[error("project not found: {0}")]
    ProjectNotFound(String),
}

fn validate_restored_project(project: &OpenedProject) -> Result<(), WorkspaceRestoreError> {
    if project.layout.tabs.is_empty() {
        if !project.selected_tab_id.is_empty() || !project.tab_states.is_empty() {
            return Err(WorkspaceRestoreError::InvalidTabState {
                project_id: project.id.as_str().to_string(),
                message: "a project without tabs must not retain selected or tab state".to_string(),
            });
        }
        return Ok(());
    }
    if project.selected_tab_id.is_empty() {
        return Err(WorkspaceRestoreError::MissingSelectedTab {
            project_id: project.id.as_str().to_string(),
        });
    }
    if project.layout.tab(&project.selected_tab_id).is_none() {
        return Err(WorkspaceRestoreError::SelectedTabMissing {
            project_id: project.id.as_str().to_string(),
            tab_id: project.selected_tab_id.clone(),
        });
    }
    if project.tab_states.len() != project.layout.tabs.len() {
        return Err(WorkspaceRestoreError::InvalidTabState {
            project_id: project.id.as_str().to_string(),
            message: "tab state count does not match the layout".to_string(),
        });
    }

    let mut state_ids = HashSet::new();
    for state in &project.tab_states {
        if !state_ids.insert(state.tab_id.clone()) {
            return Err(WorkspaceRestoreError::InvalidTabState {
                project_id: project.id.as_str().to_string(),
                message: format!("duplicate tab state: {}", state.tab_id),
            });
        }
        let Some(tab) = project.layout.tab(&state.tab_id) else {
            return Err(WorkspaceRestoreError::InvalidTabState {
                project_id: project.id.as_str().to_string(),
                message: format!("state references an unknown tab: {}", state.tab_id),
            });
        };
        let layout_panes = pane_ids(&tab.layout);
        if state.pane_states.len() != layout_panes.len() {
            return Err(WorkspaceRestoreError::InvalidTabState {
                project_id: project.id.as_str().to_string(),
                message: format!("pane state count does not match tab {}", state.tab_id),
            });
        }
        let mut pane_state_ids = HashSet::new();
        for pane in &state.pane_states {
            if !pane_state_ids.insert(pane.pane_id.clone()) || !layout_panes.contains(&pane.pane_id)
            {
                return Err(WorkspaceRestoreError::InvalidTabState {
                    project_id: project.id.as_str().to_string(),
                    message: format!(
                        "invalid pane state {} in tab {}",
                        pane.pane_id, state.tab_id
                    ),
                });
            }
        }
        if state
            .focused_pane_id
            .as_ref()
            .is_some_and(|pane_id| !layout_panes.contains(pane_id))
        {
            return Err(WorkspaceRestoreError::InvalidTabState {
                project_id: project.id.as_str().to_string(),
                message: format!("focused pane is not in tab {}", state.tab_id),
            });
        }
    }
    Ok(())
}

fn tab_states_for_layout(layout: &ProjectLayout, selected_tab_id: &str) -> Vec<TabState> {
    layout
        .tabs
        .iter()
        .map(|tab| {
            let pane_ids = pane_ids(&tab.layout);
            TabState {
                tab_id: tab.id.clone(),
                start_state: if tab.id == selected_tab_id || tab.startup.is_eager() {
                    TabStartState::Started
                } else {
                    TabStartState::Lazy
                },
                focused_pane_id: pane_ids.first().cloned(),
                pane_states: pane_ids
                    .into_iter()
                    .map(|pane_id| PaneState {
                        pane_id,
                        process_state: PaneProcessState::Idle,
                        agent_snapshot: None,
                    })
                    .collect(),
            }
        })
        .collect()
}

fn default_tab_id(layout: &ProjectLayout) -> Option<String> {
    layout
        .project
        .default_tab
        .clone()
        .or_else(|| layout.tabs.first().map(|tab| tab.id.clone()))
}

fn repair_default_tab_after_removal(layout: &mut ProjectLayout) {
    if layout
        .project
        .default_tab
        .as_ref()
        .is_some_and(|default_tab| layout.tab(default_tab).is_none())
    {
        layout.project.default_tab = layout.tabs.first().map(|tab| tab.id.clone());
    }
}

fn normalized_title(title: &str) -> Result<String, WorkspaceError> {
    let title = title.trim();
    if title.is_empty() {
        return Err(WorkspaceError::EmptyTitle);
    }

    Ok(title.to_string())
}

fn pane_ids(layout: &LayoutNode) -> Vec<String> {
    match layout {
        LayoutNode::Pane(pane) => vec![pane.id.clone()],
        LayoutNode::Split(split) => {
            let mut ids = pane_ids(&split.left);
            ids.extend(pane_ids(&split.right));
            ids
        }
    }
}

fn pane_count(layout: &LayoutNode) -> usize {
    match layout {
        LayoutNode::Pane(_) => 1,
        LayoutNode::Split(split) => pane_count(&split.left) + pane_count(&split.right),
    }
}

fn next_pane_id(layout: &LayoutNode) -> String {
    let existing = pane_ids(layout);
    for index in 1.. {
        let candidate = format!("pane-{index}");
        if !existing.iter().any(|pane_id| pane_id == &candidate) {
            return candidate;
        }
    }

    unreachable!("unbounded pane id search should always produce a candidate")
}

fn next_tab_identity(layout: &ProjectLayout) -> (String, String) {
    for index in 1.. {
        let id = format!("tab-{index}");
        if !layout.tabs.iter().any(|tab| tab.id == id) {
            return (id, format!("Tab {index}"));
        }
    }

    unreachable!("unbounded tab id search should always produce a candidate")
}

fn split_pane_node(
    layout: &mut LayoutNode,
    target_pane_id: &str,
    direction: SplitDirection,
    new_pane: PaneConfig,
) -> bool {
    match layout {
        LayoutNode::Pane(pane) if pane.id == target_pane_id => {
            let existing_pane = LayoutNode::Pane(pane.clone());
            *layout = LayoutNode::Split(SplitConfig {
                direction,
                ratio: 0.5,
                left: Box::new(existing_pane),
                right: Box::new(LayoutNode::Pane(new_pane)),
            });
            true
        }
        LayoutNode::Pane(_) => false,
        LayoutNode::Split(split) => {
            split_pane_node(&mut split.left, target_pane_id, direction, new_pane.clone())
                || split_pane_node(&mut split.right, target_pane_id, direction, new_pane)
        }
    }
}

fn remove_pane_node(layout: &mut LayoutNode, target_pane_id: &str) -> bool {
    match layout {
        LayoutNode::Pane(_) => false,
        LayoutNode::Split(split) => {
            let replacement = if split.left.pane_id() == Some(target_pane_id) {
                Some((*split.right).clone())
            } else if split.right.pane_id() == Some(target_pane_id) {
                Some((*split.left).clone())
            } else {
                None
            };

            if let Some(replacement) = replacement {
                *layout = replacement;
                return true;
            }

            remove_pane_node(&mut split.left, target_pane_id)
                || remove_pane_node(&mut split.right, target_pane_id)
        }
    }
}

fn adjacent_pane_id(
    layout: &LayoutNode,
    target_pane_id: &str,
    focus_direction: FocusDirection,
) -> Option<String> {
    match layout {
        LayoutNode::Pane(_) => None,
        LayoutNode::Split(split) => {
            if layout_contains_pane(&split.left, target_pane_id) {
                adjacent_pane_id(&split.left, target_pane_id, focus_direction).or_else(|| {
                    match (split.direction, focus_direction) {
                        (SplitDirection::Horizontal, FocusDirection::Right)
                        | (SplitDirection::Vertical, FocusDirection::Down) => {
                            first_pane_id(&split.right)
                        }
                        _ => None,
                    }
                })
            } else if layout_contains_pane(&split.right, target_pane_id) {
                adjacent_pane_id(&split.right, target_pane_id, focus_direction).or_else(|| {
                    match (split.direction, focus_direction) {
                        (SplitDirection::Horizontal, FocusDirection::Left)
                        | (SplitDirection::Vertical, FocusDirection::Up) => {
                            last_pane_id(&split.left)
                        }
                        _ => None,
                    }
                })
            } else {
                None
            }
        }
    }
}

fn layout_contains_pane(layout: &LayoutNode, target_pane_id: &str) -> bool {
    match layout {
        LayoutNode::Pane(pane) => pane.id == target_pane_id,
        LayoutNode::Split(split) => {
            layout_contains_pane(&split.left, target_pane_id)
                || layout_contains_pane(&split.right, target_pane_id)
        }
    }
}

fn first_pane_id(layout: &LayoutNode) -> Option<String> {
    match layout {
        LayoutNode::Pane(pane) => Some(pane.id.clone()),
        LayoutNode::Split(split) => first_pane_id(&split.left),
    }
}

fn last_pane_id(layout: &LayoutNode) -> Option<String> {
    match layout {
        LayoutNode::Pane(pane) => Some(pane.id.clone()),
        LayoutNode::Split(split) => last_pane_id(&split.right),
    }
}

fn resize_pane_split(
    layout: &mut LayoutNode,
    target_pane_id: &str,
    resize_direction: ResizeDirection,
    delta: f32,
) -> Option<f32> {
    match layout {
        LayoutNode::Pane(_) => None,
        LayoutNode::Split(split) => {
            if let Some(ratio) =
                resize_pane_split(&mut split.left, target_pane_id, resize_direction, delta)
            {
                return Some(ratio);
            }
            if let Some(ratio) =
                resize_pane_split(&mut split.right, target_pane_id, resize_direction, delta)
            {
                return Some(ratio);
            }

            let target_in_left = layout_contains_pane(&split.left, target_pane_id);
            let target_in_right = layout_contains_pane(&split.right, target_pane_id);
            if !target_in_left && !target_in_right {
                return None;
            }

            let adjustment = match (split.direction, resize_direction) {
                (SplitDirection::Horizontal, ResizeDirection::Right)
                | (SplitDirection::Vertical, ResizeDirection::Down) => delta,
                (SplitDirection::Horizontal, ResizeDirection::Left)
                | (SplitDirection::Vertical, ResizeDirection::Up) => -delta,
                _ => return None,
            };
            split.ratio = (split.ratio + adjustment).clamp(0.1, 0.9);
            Some(split.ratio)
        }
    }
}
