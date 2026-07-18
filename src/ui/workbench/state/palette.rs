use std::{path::PathBuf, sync::Arc};

use gpui::{Entity, ScrollHandle, Subscription, UniformListScrollHandle};
use gpui_component::input::InputState;

use crate::{
    model::{ids::ProjectId, project::ProjectLocation},
    palette::{ActivePalette, RecentProject},
    runtime::{
        file_search::{FileSearchCandidate, FileSearchMatch},
        git_status::GitBranch,
    },
    ui::editor::ReadonlyCodeRow,
};

pub(in super::super) struct PaletteControllerState {
    pub(in super::super) active_palette: Option<ActivePalette>,
    pub(in super::super) recent_projects: Vec<RecentProject>,
    pub(in super::super) input: Option<Entity<InputState>>,
    pub(in super::super) input_subscription: Option<Subscription>,
    pub(in super::super) input_needs_focus: bool,
    pub(in super::super) scroll_handle: ScrollHandle,
    pub(in super::super) file_candidates: Arc<Vec<FileSearchCandidate>>,
    pub(in super::super) file_matches: Vec<FileSearchMatch>,
    pub(in super::super) file_search_generation: u64,
    pub(in super::super) file_match_generation: u64,
    pub(in super::super) file_search_loading: bool,
    pub(in super::super) file_search_error: Option<String>,
    pub(in super::super) pending_file_search_load: bool,
    pub(in super::super) file_preview_generation: u64,
    pub(in super::super) file_preview_loading: bool,
    pub(in super::super) file_preview_path: Option<(ProjectId, PathBuf)>,
    pub(in super::super) file_preview_rows: Arc<Vec<ReadonlyCodeRow>>,
    pub(in super::super) file_preview_error: Option<String>,
    pub(in super::super) file_preview_vertical_scroll: UniformListScrollHandle,
    pub(in super::super) file_preview_horizontal_scroll: ScrollHandle,
    pub(in super::super) git_branch_project_id: Option<ProjectId>,
    pub(in super::super) git_branch_generation: u64,
    pub(in super::super) git_branches: Vec<GitBranch>,
    pub(in super::super) git_branch_loading: bool,
    pub(in super::super) git_branch_switching: bool,
    pub(in super::super) git_branch_error: Option<String>,
    pub(in super::super) pending_git_branch_load: Option<(ProjectId, ProjectLocation, u64)>,
    pub(in super::super) pending_git_branch_switch:
        Option<(ProjectId, ProjectLocation, GitBranch, u64)>,
}

impl PaletteControllerState {
    pub(in super::super) fn new(recent_projects: Vec<RecentProject>) -> Self {
        Self {
            active_palette: None,
            recent_projects,
            input: None,
            input_subscription: None,
            input_needs_focus: false,
            scroll_handle: ScrollHandle::new(),
            file_candidates: Arc::new(Vec::new()),
            file_matches: Vec::new(),
            file_search_generation: 0,
            file_match_generation: 0,
            file_search_loading: false,
            file_search_error: None,
            pending_file_search_load: false,
            file_preview_generation: 0,
            file_preview_loading: false,
            file_preview_path: None,
            file_preview_rows: Arc::new(Vec::new()),
            file_preview_error: None,
            file_preview_vertical_scroll: UniformListScrollHandle::new(),
            file_preview_horizontal_scroll: ScrollHandle::new(),
            git_branch_project_id: None,
            git_branch_generation: 0,
            git_branches: Vec::new(),
            git_branch_loading: false,
            git_branch_switching: false,
            git_branch_error: None,
            pending_git_branch_load: None,
            pending_git_branch_switch: None,
        }
    }
}
