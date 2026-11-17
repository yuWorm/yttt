use std::{collections::HashMap, path::PathBuf};

use crate::{
    model::{editor::DocumentId, ids::ProjectId},
    runtime::{git_status::ProjectGitStatus, project::ProjectServices},
    ui::{
        editor::ProjectEditorRuntime,
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
    Search,
    Git,
    AgentSessions,
    Ports,
}

impl ProjectPanelPage {
    const ALL: [Self; 5] = [
        Self::Files,
        Self::Search,
        Self::Git,
        Self::AgentSessions,
        Self::Ports,
    ];

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
    pub(in super::super) pending_project_search_focus: bool,
    pub(in super::super) project_editor_runtime: ProjectEditorRuntime,
    pub(in super::super) services: HashMap<ProjectId, ProjectServices>,
    pub(in super::super) pending_project_tree_loads: Vec<(ProjectId, DirectoryLoadRequest)>,
    pub(in super::super) project_git_statuses: HashMap<ProjectId, ProjectGitStatus>,
    pub(in super::super) project_tree_clipboard: Option<ProjectTreeClipboard>,
    pub(in super::super) active_panel_page: ProjectPanelPage,
    pub(in super::super) ports: ProjectPortsPanelState,
    pub(in super::super) search: ProjectSearchPanelState,
    /// Where to put the cursor once a file being opened shows.
    pub(in super::super) pending_file_reveal: Option<PendingFileReveal>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct PendingFileReveal {
    pub(in super::super) project_id: ProjectId,
    pub(in super::super) relative_path: PathBuf,
    /// 0-based.
    pub(in super::super) line: u32,
    /// 0-based, in UTF-16 code units.
    pub(in super::super) column: u32,
}

/// The project panel's Search page.
#[derive(Default)]
pub(in super::super) struct ProjectSearchPanelState {
    pub(in super::super) input: Option<gpui::Entity<gpui_base::input::InputState>>,
    pub(in super::super) input_subscription: Option<gpui::Subscription>,
    pub(in super::super) case_sensitive: bool,
    pub(in super::super) whole_word: bool,
    pub(in super::super) regex: bool,
    /// The project and query the shown results answer.
    pub(in super::super) key: Option<ProjectSearchKey>,
    pub(in super::super) results:
        Option<std::sync::Arc<yttt_protocol::project::ProjectSearchResults>>,
    pub(in super::super) error: Option<String>,
    pub(in super::super) unsupported: bool,
    pub(in super::super) loading: bool,
    pub(in super::super) generation: u64,
    pub(in super::super) task: Option<gpui::Task<()>>,
    pub(in super::super) collapsed: std::collections::BTreeSet<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct ProjectSearchKey {
    pub(in super::super) project_id: ProjectId,
    pub(in super::super) pattern: String,
    pub(in super::super) case_sensitive: bool,
    pub(in super::super) whole_word: bool,
    pub(in super::super) regex: bool,
}

/// What the Ports page shows for the selected project, refreshed while the
/// page is visible.
#[derive(Default)]
pub(in super::super) struct ProjectPortsPanelState {
    pub(in super::super) project_id: Option<ProjectId>,
    pub(in super::super) ports: Option<yttt_protocol::ports::ProjectPorts>,
    pub(in super::super) unsupported: bool,
    pub(in super::super) error: Option<String>,
    pub(in super::super) poll_task: Option<gpui::Task<()>>,
}
