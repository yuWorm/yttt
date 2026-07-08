use std::path::PathBuf;

use crate::model::{
    ids::ProjectId,
    layout::TabConfig,
    layout::{LayoutNode, PaneConfig, PaneKind, ProjectLayout, SplitConfig, SplitDirection},
    split_tree::{FocusDirection, ResizeDirection},
};

#[derive(Clone, Debug, Default)]
pub struct Workspace {
    opened_projects: Vec<OpenedProject>,
    selected_project_id: Option<ProjectId>,
}

impl Workspace {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn opened_projects(&self) -> &[OpenedProject] {
        &self.opened_projects
    }

    pub fn selected_project_id(&self) -> Option<&ProjectId> {
        self.selected_project_id.as_ref()
    }

    pub fn project(&self, project_id: &ProjectId) -> Option<&OpenedProject> {
        self.opened_projects
            .iter()
            .find(|project| &project.id == project_id)
    }

    pub fn open_project(
        &mut self,
        path: PathBuf,
        layout: ProjectLayout,
    ) -> Result<ProjectId, WorkspaceError> {
        layout.validate().map_err(WorkspaceError::InvalidLayout)?;

        let id = ProjectId::new(path.to_string_lossy().into_owned());
        let selected_tab_id = default_tab_id(&layout).unwrap_or_default();
        let tab_states = layout
            .tabs
            .iter()
            .map(|tab| {
                let pane_ids = pane_ids(&tab.layout);
                TabState {
                    tab_id: tab.id.clone(),
                    start_state: if tab.id == selected_tab_id {
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
                            agent_status: None,
                        })
                        .collect(),
                }
            })
            .collect();

        self.opened_projects.push(OpenedProject {
            id: id.clone(),
            path,
            layout,
            selected_tab_id,
            tab_states,
        });
        self.selected_project_id = Some(id.clone());

        Ok(id)
    }

    pub fn replace_selected_project_layout(
        &mut self,
        layout: ProjectLayout,
    ) -> Result<(), WorkspaceError> {
        layout.validate().map_err(WorkspaceError::InvalidLayout)?;
        let project = self.selected_project_mut()?;
        let selected_tab_id = if layout.tab(&project.selected_tab_id).is_some() {
            project.selected_tab_id.clone()
        } else {
            default_tab_id(&layout).unwrap_or_default()
        };
        let tab_states = tab_states_for_layout(&layout, &selected_tab_id);

        project.layout = layout;
        project.selected_tab_id = selected_tab_id;
        project.tab_states = tab_states;

        Ok(())
    }

    pub fn select_project(&mut self, project_id: &ProjectId) -> Result<(), WorkspaceError> {
        if self.project(project_id).is_none() {
            return Err(WorkspaceError::ProjectNotFound(
                project_id.as_str().to_string(),
            ));
        }

        self.selected_project_id = Some(project_id.clone());
        Ok(())
    }

    pub fn select_tab(&mut self, tab_id: &str) -> Result<(), WorkspaceError> {
        let project = self.selected_project_mut()?;
        if project.layout.tab(tab_id).is_none() {
            return Err(WorkspaceError::TabNotFound(tab_id.to_string()));
        }

        project.selected_tab_id = tab_id.to_string();
        let tab_state = project
            .tab_state_mut(tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.to_string()))?;
        tab_state.start_state = TabStartState::Started;
        if tab_state.focused_pane_id.is_none() {
            tab_state.focused_pane_id = tab_state
                .pane_states
                .first()
                .map(|pane| pane.pane_id.clone());
        }

        Ok(())
    }

    pub fn focus_pane(&mut self, pane_id: &str) -> Result<(), WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let tab = project
            .layout
            .tabs
            .iter()
            .find(|tab| tab.id == selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;

        if tab.layout.find_pane(pane_id).is_none() {
            return Err(WorkspaceError::PaneNotFound(pane_id.to_string()));
        }

        let tab_state = project
            .tab_state_mut(&selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;
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

    pub fn focus_pane_direction(
        &mut self,
        direction: FocusDirection,
    ) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let tab_index = project
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;
        let focused_pane_id = project
            .tab_state(&selected_tab_id)
            .and_then(|tab| tab.focused_pane_id.clone())
            .or_else(|| {
                pane_ids(&project.layout.tabs[tab_index].layout)
                    .into_iter()
                    .next()
            })
            .ok_or_else(|| WorkspaceError::PaneNotFound("focused".to_string()))?;
        let next_pane_id = adjacent_pane_id(
            &project.layout.tabs[tab_index].layout,
            &focused_pane_id,
            direction,
        )
        .ok_or_else(|| WorkspaceError::PaneNotFound(format!("{direction:?}")))?;

        let tab_state = project
            .tab_state_mut(&selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;
        tab_state.focused_pane_id = Some(next_pane_id.clone());

        Ok(next_pane_id)
    }

    pub fn resize_focused_split(
        &mut self,
        direction: ResizeDirection,
        delta: f32,
    ) -> Result<f32, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let tab_index = project
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;
        let focused_pane_id = project
            .tab_state(&selected_tab_id)
            .and_then(|tab| tab.focused_pane_id.clone())
            .or_else(|| {
                pane_ids(&project.layout.tabs[tab_index].layout)
                    .into_iter()
                    .next()
            })
            .ok_or_else(|| WorkspaceError::PaneNotFound("focused".to_string()))?;

        resize_pane_split(
            &mut project.layout.tabs[tab_index].layout,
            &focused_pane_id,
            direction,
            delta,
        )
        .ok_or_else(|| WorkspaceError::PaneNotFound(format!("{direction:?}")))
    }

    pub fn select_next_tab(&mut self) -> Result<(), WorkspaceError> {
        self.select_relative_tab(1)
    }

    pub fn select_prev_tab(&mut self) -> Result<(), WorkspaceError> {
        self.select_relative_tab(-1)
    }

    pub fn create_shell_tab(&mut self) -> Result<String, WorkspaceError> {
        self.create_shell_tab_with_command("$SHELL")
    }

    pub fn create_shell_tab_with_command(
        &mut self,
        command: impl Into<String>,
    ) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let (tab_id, title) = next_tab_identity(&project.layout);
        let pane_id = "shell".to_string();
        let command = command.into();

        project.layout.tabs.push(TabConfig {
            id: tab_id.clone(),
            title,
            layout: LayoutNode::Pane(PaneConfig {
                id: pane_id.clone(),
                title: "shell".to_string(),
                command,
                kind: PaneKind::Shell,
                notify_on_exit: false,
                detector: None,
            }),
        });
        project.selected_tab_id = tab_id.clone();
        project.tab_states.push(TabState {
            tab_id: tab_id.clone(),
            start_state: TabStartState::Started,
            focused_pane_id: Some(pane_id.clone()),
            pane_states: vec![PaneState {
                pane_id,
                process_state: PaneProcessState::Idle,
                agent_status: None,
            }],
        });

        Ok(tab_id)
    }

    pub fn close_selected_tab(&mut self) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        if project.layout.tabs.len() <= 1 {
            return Err(WorkspaceError::CannotCloseLastTab);
        }

        let selected_tab_id = project.selected_tab_id.clone();
        let tab_index = project
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;

        let removed_tab = project.layout.tabs.remove(tab_index);
        project
            .tab_states
            .retain(|tab| tab.tab_id != removed_tab.id);

        let next_index = tab_index.min(project.layout.tabs.len().saturating_sub(1));
        let next_tab_id = project.layout.tabs[next_index].id.clone();
        project.selected_tab_id = next_tab_id.clone();
        let next_tab_state = project
            .tab_state_mut(&next_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(next_tab_id.clone()))?;
        next_tab_state.start_state = TabStartState::Started;
        if next_tab_state.focused_pane_id.is_none() {
            next_tab_state.focused_pane_id = next_tab_state
                .pane_states
                .first()
                .map(|pane| pane.pane_id.clone());
        }

        Ok(removed_tab.id)
    }

    pub fn rename_selected_tab(&mut self, title: &str) -> Result<(), WorkspaceError> {
        let title = normalized_title(title)?;
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let tab = project
            .layout
            .tabs
            .iter_mut()
            .find(|tab| tab.id == selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;

        tab.title = title;
        Ok(())
    }

    pub fn split_focused_pane(
        &mut self,
        direction: SplitDirection,
    ) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let tab_index = project
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;
        let focused_pane_id = project
            .tab_state(&selected_tab_id)
            .and_then(|tab| tab.focused_pane_id.clone())
            .or_else(|| {
                pane_ids(&project.layout.tabs[tab_index].layout)
                    .into_iter()
                    .next()
            })
            .ok_or_else(|| WorkspaceError::PaneNotFound("focused".to_string()))?;
        let new_pane_number = pane_count(&project.layout.tabs[tab_index].layout) + 1;
        let new_pane_id = next_pane_id(&project.layout.tabs[tab_index].layout);
        let new_pane = PaneConfig {
            id: new_pane_id.clone(),
            title: format!("Pane {new_pane_number}"),
            command: "$SHELL".to_string(),
            kind: PaneKind::Shell,
            notify_on_exit: false,
            detector: None,
        };

        if !split_pane_node(
            &mut project.layout.tabs[tab_index].layout,
            &focused_pane_id,
            direction,
            new_pane,
        ) {
            return Err(WorkspaceError::PaneNotFound(focused_pane_id));
        }

        let tab_state = project
            .tab_state_mut(&selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;
        tab_state.pane_states.push(PaneState {
            pane_id: new_pane_id.clone(),
            process_state: PaneProcessState::Idle,
            agent_status: None,
        });
        tab_state.focused_pane_id = Some(new_pane_id.clone());

        Ok(new_pane_id)
    }

    pub fn close_focused_pane(&mut self) -> Result<String, WorkspaceError> {
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let tab_index = project
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;

        if pane_count(&project.layout.tabs[tab_index].layout) <= 1 {
            return Err(WorkspaceError::CannotCloseLastPane);
        }

        let focused_pane_id = project
            .tab_state(&selected_tab_id)
            .and_then(|tab| tab.focused_pane_id.clone())
            .or_else(|| {
                pane_ids(&project.layout.tabs[tab_index].layout)
                    .into_iter()
                    .next()
            })
            .ok_or_else(|| WorkspaceError::PaneNotFound("focused".to_string()))?;

        if !remove_pane_node(&mut project.layout.tabs[tab_index].layout, &focused_pane_id) {
            return Err(WorkspaceError::PaneNotFound(focused_pane_id));
        }

        let remaining_panes = pane_ids(&project.layout.tabs[tab_index].layout);
        let tab_state = project
            .tab_state_mut(&selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;
        tab_state
            .pane_states
            .retain(|pane| pane.pane_id != focused_pane_id);
        tab_state.focused_pane_id = remaining_panes.first().cloned();

        Ok(focused_pane_id)
    }

    pub fn close_pane_for_exit(
        &mut self,
        project_id: &ProjectId,
        tab_id: &str,
        pane_id: &str,
    ) -> Result<PaneExitCloseOutcome, WorkspaceError> {
        let project = self
            .opened_projects
            .iter_mut()
            .find(|project| &project.id == project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
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

        if project.layout.tabs.is_empty() {
            project.selected_tab_id.clear();
            return Ok(PaneExitCloseOutcome::ProjectEmptied);
        }

        if project.selected_tab_id == removed_tab_id {
            let next_index = tab_index.min(project.layout.tabs.len().saturating_sub(1));
            let next_tab_id = project.layout.tabs[next_index].id.clone();
            project.selected_tab_id = next_tab_id.clone();
            let next_tab_state = project
                .tab_state_mut(&next_tab_id)
                .ok_or_else(|| WorkspaceError::TabNotFound(next_tab_id.clone()))?;
            next_tab_state.start_state = TabStartState::Started;
            if next_tab_state.focused_pane_id.is_none() {
                next_tab_state.focused_pane_id = next_tab_state
                    .pane_states
                    .first()
                    .map(|pane| pane.pane_id.clone());
            }
        }

        Ok(PaneExitCloseOutcome::TabClosed)
    }

    pub fn rename_focused_pane(&mut self, title: &str) -> Result<(), WorkspaceError> {
        let title = normalized_title(title)?;
        let project = self.selected_project_mut()?;
        let selected_tab_id = project.selected_tab_id.clone();
        let tab_index = project
            .layout
            .tabs
            .iter()
            .position(|tab| tab.id == selected_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(selected_tab_id.clone()))?;
        let focused_pane_id = project
            .tab_state(&selected_tab_id)
            .and_then(|tab| tab.focused_pane_id.clone())
            .or_else(|| {
                pane_ids(&project.layout.tabs[tab_index].layout)
                    .into_iter()
                    .next()
            })
            .ok_or_else(|| WorkspaceError::PaneNotFound("focused".to_string()))?;
        let pane = project.layout.tabs[tab_index]
            .layout
            .find_pane_mut(&focused_pane_id)
            .ok_or_else(|| WorkspaceError::PaneNotFound(focused_pane_id.clone()))?;

        pane.title = title;
        Ok(())
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
        pane.agent_status = None;
        Ok(())
    }

    pub fn record_agent_status(
        &mut self,
        project_id: &ProjectId,
        tab_id: &str,
        pane_id: &str,
        status: AgentStatus,
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
        pane.agent_status = Some(status);
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

        project.selected_tab_id = next_tab_id.clone();
        let tab_state = project
            .tab_state_mut(&next_tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(next_tab_id.clone()))?;
        tab_state.start_state = TabStartState::Started;
        if tab_state.focused_pane_id.is_none() {
            tab_state.focused_pane_id = tab_state
                .pane_states
                .first()
                .map(|pane| pane.pane_id.clone());
        }

        Ok(())
    }

    fn selected_project_mut(&mut self) -> Result<&mut OpenedProject, WorkspaceError> {
        let project_id = self
            .selected_project_id
            .clone()
            .ok_or(WorkspaceError::NoSelectedProject)?;

        self.opened_projects
            .iter_mut()
            .find(|project| project.id == project_id)
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
        if self.selected_project_id.as_ref() == Some(&project.id) {
            self.selected_project_id = self
                .opened_projects
                .first()
                .map(|project| project.id.clone());
        }

        ClosedProject {
            project_id: project.id,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct OpenedProject {
    pub id: ProjectId,
    pub path: PathBuf,
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
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseProjectDecision {
    Closed(ClosedProject),
    NeedsConfirmation {
        project_id: ProjectId,
        running_pane_count: usize,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaneState {
    pub pane_id: String,
    pub process_state: PaneProcessState,
    pub agent_status: Option<AgentStatus>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AgentStatus {
    Running,
    Completed,
    Failed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TabStartState {
    Lazy,
    Started,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneProcessState {
    Idle,
    Running,
    Exited,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneExitCloseOutcome {
    PaneClosed,
    TabClosed,
    ProjectEmptied,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedProject {
    pub project_id: ProjectId,
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

fn tab_states_for_layout(layout: &ProjectLayout, selected_tab_id: &str) -> Vec<TabState> {
    layout
        .tabs
        .iter()
        .map(|tab| {
            let pane_ids = pane_ids(&tab.layout);
            TabState {
                tab_id: tab.id.clone(),
                start_state: if tab.id == selected_tab_id {
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
                        agent_status: None,
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
