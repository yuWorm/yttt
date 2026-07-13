use std::{collections::HashMap, path::PathBuf};

use crate::{
    model::ids::ProjectId,
    runtime::git_status::ProjectGitStatus,
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

#[derive(Default)]
pub(in super::super) struct ProjectControllerState {
    pub(in super::super) layout_source_messages: HashMap<ProjectId, String>,
    pub(in super::super) pending_editor_focus_document_id: Option<DocumentId>,
    pub(in super::super) project_editor_runtime: ProjectEditorRuntime,
    pub(in super::super) pending_project_tree_loads: Vec<(ProjectId, DirectoryLoadRequest)>,
    pub(in super::super) project_git_statuses: HashMap<ProjectId, ProjectGitStatus>,
    pub(in super::super) project_tree_clipboard: Option<ProjectTreeClipboard>,
}
