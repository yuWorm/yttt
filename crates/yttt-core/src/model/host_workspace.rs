//! The Host-owned workspace layout and the intents that change it.
//!
//! A [`HostWorkspace`] combines the project/tab/pane layout with the editor
//! work area of every opened project. Clients never replace it wholesale:
//! they submit a [`WorkspaceIntent`] that names its targets by stable ID, and
//! the Host applies intents one at a time in arrival order. Every accepted
//! intent advances the revision by one.

use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
};

use crate::model::{
    editor::{DocumentId, ProjectEditorWorkspaceSnapshot, ProjectEditorWorkspaceState, WorkItemId},
    ids::ProjectId,
    layout::{ProjectLayout, SplitDirection},
    project::ProjectDescriptor,
    split_tree::{FocusDirection, ResizeDirection},
    work_area::{TabGroupId, WorkAreaDropPlacement, WorkAreaSplitId},
    workspace::{
        CloseProjectDecision, CloseProjectError, Workspace, WorkspaceError, WorkspaceRestoreError,
        WorkspaceState, agent_tab_config, shell_tab_config,
    },
};

/// The serialized form of a [`HostWorkspace`], used for persistence and on the wire.
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct HostWorkspaceState {
    pub revision: u64,
    pub workspace: WorkspaceState,
    pub editor: ProjectEditorWorkspaceSnapshot,
}

/// The layout portion of a Client-published workspace snapshot (schemas 1 and 2).
///
/// Documents and Agent state references in the same JSON object are owned by
/// other Host stores and are ignored here.
#[derive(Clone, Debug, serde::Deserialize)]
pub struct LegacyWorkspaceSnapshot {
    pub schema_version: u16,
    pub workspace: WorkspaceState,
    pub editor: ProjectEditorWorkspaceSnapshot,
}

/// How a project's file panel is shown when the project is opened.
#[derive(Clone, Copy, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
pub struct ProjectPanel {
    pub visible: bool,
    pub width: f32,
}

/// A tab created by the Host, which assigns its ID and title.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum NewTab {
    Shell {
        command: String,
    },
    Agent {
        title: String,
        command: String,
        args: Vec<String>,
    },
}

/// One requested change to a Host workspace. Targets are named by stable ID;
/// none of these act on "whatever is currently selected".
#[derive(Clone, Debug, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "intent", rename_all = "snake_case")]
pub enum WorkspaceIntent {
    OpenProject {
        descriptor: ProjectDescriptor,
        layout: ProjectLayout,
        panel: ProjectPanel,
    },
    /// Closing a project with running panes requires `confirmed_running`.
    CloseProject {
        project_id: ProjectId,
        confirmed_running: bool,
    },
    SelectProject {
        project_id: ProjectId,
    },
    SelectCommandCenter,
    CreateTab {
        project_id: ProjectId,
        tab: NewTab,
    },
    CloseTabs {
        project_id: ProjectId,
        tab_ids: Vec<String>,
    },
    RenameTab {
        project_id: ProjectId,
        tab_id: String,
        title: String,
    },
    SplitPane {
        project_id: ProjectId,
        tab_id: String,
        pane_id: String,
        direction: SplitDirection,
        command: String,
    },
    ClosePane {
        project_id: ProjectId,
        tab_id: String,
        pane_id: String,
    },
    FocusPane {
        project_id: ProjectId,
        tab_id: String,
        pane_id: String,
    },
    FocusAdjacentPane {
        project_id: ProjectId,
        tab_id: String,
        direction: FocusDirection,
    },
    ResizePaneSplit {
        project_id: ProjectId,
        tab_id: String,
        pane_id: String,
        direction: ResizeDirection,
        delta: f32,
    },
    RenamePane {
        project_id: ProjectId,
        tab_id: String,
        pane_id: String,
        title: String,
    },
    /// Replaces a whole project layout; requires an expected revision.
    ReplaceProjectLayout {
        project_id: ProjectId,
        layout: ProjectLayout,
    },
    OpenFile {
        project_id: ProjectId,
        path: PathBuf,
    },
    CloseFile {
        document: DocumentId,
    },
    RelocateFile {
        from: DocumentId,
        to: DocumentId,
    },
    /// Activates a file or terminal tab; a terminal tab also becomes the selected tab.
    ActivateItem {
        project_id: ProjectId,
        item: WorkItemId,
    },
    ActivateGroup {
        project_id: ProjectId,
        group_id: TabGroupId,
    },
    MoveItem {
        project_id: ProjectId,
        item: WorkItemId,
        source_group: TabGroupId,
        target_group: TabGroupId,
        placement: WorkAreaDropPlacement,
    },
    ResizeWorkAreaSplit {
        project_id: ProjectId,
        split_id: WorkAreaSplitId,
        delta: f32,
    },
    SetProjectPanel {
        project_id: ProjectId,
        panel: ProjectPanel,
    },
}

/// A pane by its stable address. Its terminal session ID is
/// `"{project_id}:{tab_id}:{pane_id}"`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaneAddress {
    pub project_id: ProjectId,
    pub tab_id: String,
    pub pane_id: String,
}

impl PaneAddress {
    fn cmp_key(left: &Self, right: &Self) -> std::cmp::Ordering {
        (left.project_id.as_str(), &left.tab_id, &left.pane_id).cmp(&(
            right.project_id.as_str(),
            &right.tab_id,
            &right.pane_id,
        ))
    }
}

/// The result of an accepted intent.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct IntentOutcome {
    pub revision: u64,
    /// Panes that exist after the intent but did not before, sorted.
    pub added_panes: Vec<PaneAddress>,
    /// Panes that existed before the intent but no longer do, sorted. Their
    /// terminals must be terminated.
    pub removed_panes: Vec<PaneAddress>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum HostWorkspaceError {
    #[error("workspace snapshot schema {0} is unsupported")]
    UnsupportedLegacySchema(u16),
    #[error(transparent)]
    InvalidWorkspace(#[from] WorkspaceRestoreError),
    #[error("project {0} does not have a Host filesystem root")]
    MissingProjectRoot(String),
    #[error("editor state is invalid: {0}")]
    InvalidEditor(String),
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum IntentError {
    #[error("workspace revision is {current}, not the expected {expected}")]
    RevisionConflict { expected: u64, current: u64 },
    #[error("this intent requires an expected revision")]
    RevisionRequired,
    #[error(transparent)]
    Workspace(#[from] WorkspaceError),
    #[error("project has running processes")]
    RunningProcesses,
    #[error("project {0} does not have a Host filesystem root")]
    MissingProjectRoot(String),
    #[error("file {0} is outside its project root")]
    FileOutsideProject(String),
    #[error("work item is not open in project {0}")]
    WorkItemNotFound(String),
    #[error("tab group {0} does not exist")]
    GroupNotFound(u64),
    #[error("work-area split {0} does not exist")]
    SplitNotFound(u64),
}

impl From<CloseProjectError> for IntentError {
    fn from(error: CloseProjectError) -> Self {
        match error {
            CloseProjectError::RunningProcesses => Self::RunningProcesses,
            CloseProjectError::ProjectNotFound(id) => {
                Self::Workspace(WorkspaceError::ProjectNotFound(id))
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct HostWorkspace {
    revision: u64,
    workspace: Workspace,
    editor: ProjectEditorWorkspaceState,
}

impl HostWorkspace {
    pub fn new() -> Self {
        Self::default()
    }

    /// Restores and validates a persisted workspace.
    pub fn restore(state: HostWorkspaceState) -> Result<Self, HostWorkspaceError> {
        let mut workspace_state = state.workspace;
        repair_stale_default_tabs(&mut workspace_state);
        let workspace = Workspace::restore_persisted_state(workspace_state)?;
        let mut project_roots = HashMap::new();
        for project in workspace.opened_projects() {
            let root = project.location.local_path().ok_or_else(|| {
                HostWorkspaceError::MissingProjectRoot(project.id.as_str().to_string())
            })?;
            project_roots.insert(project.id.clone(), root.clone());
        }
        let editor = ProjectEditorWorkspaceState::from_snapshot(
            state.editor,
            &project_roots,
            &tab_ids_by_project(&workspace),
        )
        .map_err(HostWorkspaceError::InvalidEditor)?;
        Ok(Self {
            revision: state.revision,
            workspace,
            editor,
        })
    }

    /// Migrates the layout of a Client-published snapshot, keeping its revision.
    pub fn from_legacy(
        snapshot: LegacyWorkspaceSnapshot,
        revision: u64,
    ) -> Result<Self, HostWorkspaceError> {
        if !matches!(snapshot.schema_version, 1 | 2) {
            return Err(HostWorkspaceError::UnsupportedLegacySchema(
                snapshot.schema_version,
            ));
        }
        Self::restore(HostWorkspaceState {
            revision,
            workspace: snapshot.workspace,
            editor: snapshot.editor,
        })
    }

    pub fn state(&self) -> HostWorkspaceState {
        HostWorkspaceState {
            revision: self.revision,
            workspace: self.workspace.persisted_state(),
            editor: self.editor.snapshot(),
        }
    }

    pub fn revision(&self) -> u64 {
        self.revision
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn editor(&self) -> &ProjectEditorWorkspaceState {
        &self.editor
    }

    /// Applies one intent. Only `ReplaceProjectLayout` requires
    /// `expected_revision`; any intent that carries one is rejected when it is
    /// stale. A rejected intent leaves the workspace and revision unchanged.
    pub fn apply(
        &mut self,
        intent: WorkspaceIntent,
        expected_revision: Option<u64>,
    ) -> Result<IntentOutcome, IntentError> {
        if let Some(expected) = expected_revision
            && expected != self.revision
        {
            return Err(IntentError::RevisionConflict {
                expected,
                current: self.revision,
            });
        }
        if matches!(intent, WorkspaceIntent::ReplaceProjectLayout { .. })
            && expected_revision.is_none()
        {
            return Err(IntentError::RevisionRequired);
        }

        let before = self.pane_addresses();
        self.apply_intent(intent)?;
        self.revision += 1;
        let after = self.pane_addresses();
        let mut added_panes = after.difference(&before).cloned().collect::<Vec<_>>();
        let mut removed_panes = before.difference(&after).cloned().collect::<Vec<_>>();
        added_panes.sort_by(PaneAddress::cmp_key);
        removed_panes.sort_by(PaneAddress::cmp_key);
        Ok(IntentOutcome {
            revision: self.revision,
            added_panes,
            removed_panes,
        })
    }

    fn apply_intent(&mut self, intent: WorkspaceIntent) -> Result<(), IntentError> {
        match intent {
            WorkspaceIntent::OpenProject {
                descriptor,
                layout,
                panel,
            } => {
                let root =
                    descriptor.location.local_path().cloned().ok_or_else(|| {
                        IntentError::MissingProjectRoot(descriptor.id.to_string())
                    })?;
                let project_id = self.workspace.open_project(descriptor, layout)?;
                let selected_tab = self
                    .workspace
                    .project(&project_id)
                    .filter(|project| project.layout.tab(&project.selected_tab_id).is_some())
                    .map(|project| project.selected_tab_id.clone());
                self.editor.open_project(
                    project_id.clone(),
                    root,
                    selected_tab,
                    panel.visible,
                    panel.width,
                );
                self.sync_project(&project_id)?;
            }
            WorkspaceIntent::CloseProject {
                project_id,
                confirmed_running,
            } => {
                if confirmed_running {
                    self.workspace.confirm_close_project(&project_id)?;
                } else if let CloseProjectDecision::NeedsConfirmation { .. } =
                    self.workspace.request_close_project(&project_id)?
                {
                    return Err(IntentError::RunningProcesses);
                }
                self.editor.close_project(&project_id);
            }
            WorkspaceIntent::SelectProject { project_id } => {
                self.workspace.select_project(&project_id)?;
            }
            WorkspaceIntent::SelectCommandCenter => self.workspace.select_command_center(),
            WorkspaceIntent::CreateTab { project_id, tab } => {
                let project = self.workspace.project_mut(&project_id)?;
                let (tab_id, default_title) = project.next_tab_identity();
                let config = match tab {
                    NewTab::Shell { command } => shell_tab_config(tab_id, default_title, command),
                    NewTab::Agent {
                        title,
                        command,
                        args,
                    } => agent_tab_config(tab_id, title, command, args),
                };
                let tab_id = project.add_tab(config)?;
                self.activate_item(&project_id, WorkItemId::Terminal(tab_id))?;
            }
            WorkspaceIntent::CloseTabs {
                project_id,
                tab_ids,
            } => {
                let project = self.workspace.project_mut(&project_id)?;
                if let Some(missing) = tab_ids.iter().find(|id| project.layout.tab(id).is_none()) {
                    return Err(WorkspaceError::TabNotFound(missing.clone()).into());
                }
                project.close_tabs(&tab_ids)?;
                self.sync_project(&project_id)?;
            }
            WorkspaceIntent::RenameTab {
                project_id,
                tab_id,
                title,
            } => self
                .workspace
                .project_mut(&project_id)?
                .rename_tab(&tab_id, &title)?,
            WorkspaceIntent::SplitPane {
                project_id,
                tab_id,
                pane_id,
                direction,
                command,
            } => {
                let project = self.workspace.project_mut(&project_id)?;
                let new_pane_id = project.next_pane_id(&tab_id)?;
                project.split_pane(&tab_id, &pane_id, direction, new_pane_id, command)?;
            }
            WorkspaceIntent::ClosePane {
                project_id,
                tab_id,
                pane_id,
            } => self
                .workspace
                .project_mut(&project_id)?
                .close_pane(&tab_id, &pane_id)?,
            WorkspaceIntent::FocusPane {
                project_id,
                tab_id,
                pane_id,
            } => self
                .workspace
                .project_mut(&project_id)?
                .focus_pane(&tab_id, &pane_id)?,
            WorkspaceIntent::FocusAdjacentPane {
                project_id,
                tab_id,
                direction,
            } => {
                self.workspace
                    .project_mut(&project_id)?
                    .focus_adjacent_pane(&tab_id, direction)?;
            }
            WorkspaceIntent::ResizePaneSplit {
                project_id,
                tab_id,
                pane_id,
                direction,
                delta,
            } => {
                self.workspace
                    .project_mut(&project_id)?
                    .resize_pane_split(&tab_id, &pane_id, direction, delta)?;
            }
            WorkspaceIntent::RenamePane {
                project_id,
                tab_id,
                pane_id,
                title,
            } => self
                .workspace
                .project_mut(&project_id)?
                .rename_pane(&tab_id, &pane_id, &title)?,
            WorkspaceIntent::ReplaceProjectLayout { project_id, layout } => {
                self.workspace
                    .project_mut(&project_id)?
                    .replace_layout(layout)?;
                self.sync_project(&project_id)?;
            }
            WorkspaceIntent::OpenFile { project_id, path } => {
                let root = self.project_root(&project_id)?;
                if !path.starts_with(&root) {
                    return Err(IntentError::FileOutsideProject(path.display().to_string()));
                }
                let document = self.session_mut(&project_id)?.open_file(path);
                self.activate_item(&project_id, WorkItemId::File(document))?;
            }
            WorkspaceIntent::CloseFile { document } => {
                let project_id = document.project_id.clone();
                let tab_ids = self.tab_ids(&project_id)?;
                let session = self.session_mut(&project_id)?;
                if !session.file_ids().contains(&document) {
                    return Err(IntentError::WorkItemNotFound(project_id.to_string()));
                }
                session.close_file(&document, &tab_ids);
                self.sync_project(&project_id)?;
            }
            WorkspaceIntent::RelocateFile { from, to } => {
                let root = self.project_root(&to.project_id)?;
                if !to.canonical_path.starts_with(&root) {
                    return Err(IntentError::FileOutsideProject(
                        to.canonical_path.display().to_string(),
                    ));
                }
                if !self.editor.relocate_file(&from, to.clone()) {
                    return Err(IntentError::WorkItemNotFound(from.project_id.to_string()));
                }
                self.sync_project(&from.project_id)?;
                if to.project_id != from.project_id {
                    self.sync_project(&to.project_id)?;
                }
            }
            WorkspaceIntent::ActivateItem { project_id, item } => {
                self.activate_item(&project_id, item)?;
            }
            WorkspaceIntent::ActivateGroup {
                project_id,
                group_id,
            } => {
                self.session_mut(&project_id)?
                    .activate_group(group_id)
                    .ok_or(IntentError::GroupNotFound(group_id.raw()))?;
                self.sync_project(&project_id)?;
            }
            WorkspaceIntent::MoveItem {
                project_id,
                item,
                source_group,
                target_group,
                placement,
            } => {
                let tab_ids = self.tab_ids(&project_id)?;
                let session = self.session_mut(&project_id)?;
                if session.group_id_containing(&item) != Some(source_group) {
                    return Err(IntentError::WorkItemNotFound(project_id.to_string()));
                }
                session.drop_work_item(&item, source_group, target_group, placement, &tab_ids);
                self.sync_project(&project_id)?;
            }
            WorkspaceIntent::ResizeWorkAreaSplit {
                project_id,
                split_id,
                delta,
            } => {
                self.session_mut(&project_id)?
                    .resize_work_area_split(split_id, delta)
                    .ok_or(IntentError::SplitNotFound(split_id.raw()))?;
            }
            WorkspaceIntent::SetProjectPanel { project_id, panel } => {
                let session = self.session_mut(&project_id)?;
                session.set_project_panel_visible(panel.visible);
                session.set_project_panel_width(panel.width);
            }
        }
        Ok(())
    }

    /// Activates a work item; a terminal tab also becomes the project's selected tab.
    fn activate_item(
        &mut self,
        project_id: &ProjectId,
        item: WorkItemId,
    ) -> Result<(), IntentError> {
        let tab_ids = self.tab_ids(project_id)?;
        if !self
            .session_mut(project_id)?
            .select_work_item(item, &tab_ids)
        {
            return Err(IntentError::WorkItemNotFound(project_id.to_string()));
        }
        self.sync_project(project_id)
    }

    /// Restores the work area's view of the project's tabs and keeps the
    /// selected tab equal to the active terminal item.
    fn sync_project(&mut self, project_id: &ProjectId) -> Result<(), IntentError> {
        let tab_ids = self.tab_ids(project_id)?;
        let session = self.session_mut(project_id)?;
        session.reconcile_work_area(&tab_ids);
        if let Some(WorkItemId::Terminal(tab_id)) = session.active_work_item().cloned() {
            let project = self.workspace.project_mut(project_id)?;
            if project.selected_tab_id != tab_id {
                project.select_tab(&tab_id)?;
            }
        }
        Ok(())
    }

    fn tab_ids(&self, project_id: &ProjectId) -> Result<Vec<String>, IntentError> {
        let project = self
            .workspace
            .project(project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.to_string()))?;
        Ok(project
            .layout
            .tabs
            .iter()
            .map(|tab| tab.id.clone())
            .collect())
    }

    fn project_root(&self, project_id: &ProjectId) -> Result<PathBuf, IntentError> {
        self.editor
            .session(project_id)
            .map(|session| session.root().to_path_buf())
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.to_string()).into())
    }

    fn session_mut(
        &mut self,
        project_id: &ProjectId,
    ) -> Result<&mut crate::model::editor::ProjectWorkItemSession, IntentError> {
        self.editor
            .session_mut(project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.to_string()).into())
    }

    fn pane_addresses(&self) -> HashSet<PaneAddress> {
        self.workspace
            .opened_projects()
            .iter()
            .flat_map(|project| {
                project
                    .pane_addresses()
                    .map(move |(tab_id, pane_id)| PaneAddress {
                        project_id: project.id.clone(),
                        tab_id: tab_id.to_string(),
                        pane_id: pane_id.to_string(),
                    })
            })
            .collect()
    }
}

/// Snapshots written before tab removal repaired the default tab may name a
/// tab that no longer exists; point it at the selected or first tab.
fn repair_stale_default_tabs(workspace: &mut WorkspaceState) {
    for project in &mut workspace.opened_projects {
        let Some(default_tab) = project.layout.project.default_tab.as_ref() else {
            continue;
        };
        if project.layout.tab(default_tab).is_some() {
            continue;
        }
        project.layout.project.default_tab = project
            .layout
            .tab(&project.selected_tab_id)
            .map(|tab| tab.id.clone())
            .or_else(|| project.layout.tabs.first().map(|tab| tab.id.clone()));
    }
}

fn tab_ids_by_project(workspace: &Workspace) -> HashMap<ProjectId, Vec<String>> {
    workspace
        .opened_projects()
        .iter()
        .map(|project| {
            (
                project.id.clone(),
                project
                    .layout
                    .tabs
                    .iter()
                    .map(|tab| tab.id.clone())
                    .collect(),
            )
        })
        .collect()
}
