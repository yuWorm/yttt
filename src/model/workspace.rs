use std::path::PathBuf;

use crate::model::{
    ids::ProjectId,
    layout::{LayoutNode, ProjectLayout},
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
            .map(|tab| TabState {
                tab_id: tab.id.clone(),
                start_state: if tab.id == selected_tab_id {
                    TabStartState::Started
                } else {
                    TabStartState::Lazy
                },
                pane_states: pane_ids(&tab.layout)
                    .into_iter()
                    .map(|pane_id| PaneState {
                        pane_id,
                        process_state: PaneProcessState::Idle,
                    })
                    .collect(),
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

    pub fn select_project(&mut self, project_id: &ProjectId) -> Result<(), WorkspaceError> {
        if self.project(project_id).is_none() {
            return Err(WorkspaceError::ProjectNotFound(project_id.as_str().to_string()));
        }

        self.selected_project_id = Some(project_id.clone());
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
        Ok(())
    }

    pub fn close_project(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<ClosedProject, CloseProjectError> {
        let index = self
            .opened_projects
            .iter()
            .position(|project| &project.id == project_id)
            .ok_or_else(|| CloseProjectError::ProjectNotFound(project_id.as_str().to_string()))?;

        if self.opened_projects[index].has_running_panes() {
            return Err(CloseProjectError::RunningProcesses);
        }

        let project = self.opened_projects.remove(index);
        if self.selected_project_id.as_ref() == Some(project_id) {
            self.selected_project_id = self.opened_projects.first().map(|project| project.id.clone());
        }

        Ok(ClosedProject {
            project_id: project.id,
        })
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
        self.tab_states.iter().any(|tab| {
            tab.pane_states
                .iter()
                .any(|pane| pane.process_state == PaneProcessState::Running)
        })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabState {
    pub tab_id: String,
    pub start_state: TabStartState,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClosedProject {
    pub project_id: ProjectId,
}

#[derive(Debug, thiserror::Error)]
pub enum WorkspaceError {
    #[error("{0}")]
    InvalidLayout(#[from] crate::model::layout::LayoutError),
    #[error("project not found: {0}")]
    ProjectNotFound(String),
    #[error("tab not found: {0}")]
    TabNotFound(String),
    #[error("pane not found: {0}")]
    PaneNotFound(String),
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum CloseProjectError {
    #[error("project has running processes")]
    RunningProcesses,
    #[error("project not found: {0}")]
    ProjectNotFound(String),
}

fn default_tab_id(layout: &ProjectLayout) -> Option<String> {
    layout
        .project
        .default_tab
        .clone()
        .or_else(|| layout.tabs.first().map(|tab| tab.id.clone()))
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
