use std::collections::HashMap;

use crate::{
    model::ids::ProjectId,
    runtime::git_status::ProjectGitStatus,
    ui::{
        editor::{DocumentId, ProjectEditorRuntime},
        project_tree::DirectoryLoadRequest,
    },
};

#[derive(Default)]
pub(in super::super) struct ProjectControllerState {
    pub(in super::super) layout_source_messages: HashMap<ProjectId, String>,
    pub(in super::super) pending_editor_focus_document_id: Option<DocumentId>,
    pub(in super::super) project_editor_runtime: ProjectEditorRuntime,
    pub(in super::super) pending_project_tree_loads: Vec<(ProjectId, DirectoryLoadRequest)>,
    pub(in super::super) project_git_statuses: HashMap<ProjectId, ProjectGitStatus>,
}
