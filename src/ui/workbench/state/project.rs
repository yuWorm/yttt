use std::{collections::HashMap, path::PathBuf};

use crate::{
    model::ids::ProjectId,
    runtime::{git_status::ProjectGitStatus, project::ProjectServices},
    ui::{
        editor::{DocumentId, ProjectEditorRuntime},
        project_tree::{DirectoryLoadRequest, ProjectEntryPasteMode},
    },
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct ProjectTreeClipboard {
    pub(in super::super) source_project_id: ProjectId,
    pub(in super::super) relative_path: PathBuf,
    pub(in super::super) mode: ProjectEntryPasteMode,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(in super::super) enum ProjectPanelPage {
    #[default]
    Files,
}

impl ProjectPanelPage {
    const ALL: [Self; 1] = [Self::Files];

    pub(in super::super) fn next(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|page| *page == self)
            .expect("active project panel page must be registered");
        Self::ALL[(index + 1) % Self::ALL.len()]
    }

    pub(in super::super) fn previous(self) -> Self {
        let index = Self::ALL
            .iter()
            .position(|page| *page == self)
            .expect("active project panel page must be registered");
        Self::ALL[(index + Self::ALL.len() - 1) % Self::ALL.len()]
    }
}

#[derive(Default)]
pub(in super::super) struct ProjectControllerState {
    pub(in super::super) layout_source_messages: HashMap<ProjectId, String>,
    pub(in super::super) pending_editor_focus_document_id: Option<DocumentId>,
    pub(in super::super) pending_project_tree_focus: bool,
    pub(in super::super) project_editor_runtime: ProjectEditorRuntime,
    pub(in super::super) services: HashMap<ProjectId, ProjectServices>,
    pub(in super::super) pending_project_tree_loads: Vec<(ProjectId, DirectoryLoadRequest)>,
    pub(in super::super) project_git_statuses: HashMap<ProjectId, ProjectGitStatus>,
    pub(in super::super) project_tree_clipboard: Option<ProjectTreeClipboard>,
    pub(in super::super) active_panel_page: ProjectPanelPage,
}
