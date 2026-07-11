use crate::{model::ids::ProjectId, ui::editor::DocumentId};

use super::super::{PendingDirtyClose, PendingFileConflict};

#[derive(Default)]
pub(in super::super) struct DocumentLifecycleState {
    pub(in super::super) pending_document_saves: Vec<DocumentId>,
    pub(in super::super) pending_focus_change_autosaves: Vec<DocumentId>,
    pub(in super::super) pending_file_close_requests: Vec<DocumentId>,
    pub(in super::super) pending_project_close_requests: Vec<ProjectId>,
    pub(in super::super) pending_file_conflict: Option<PendingFileConflict>,
    pub(in super::super) pending_dirty_close: Option<PendingDirtyClose>,
    pub(in super::super) allow_window_close_once: bool,
}
