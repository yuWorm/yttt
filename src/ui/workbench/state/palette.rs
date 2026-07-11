use std::path::PathBuf;

use gpui::{Entity, ScrollHandle, Subscription};
use gpui_component::input::InputState;

use crate::{
    model::ids::ProjectId,
    palette::{ActivePalette, RecentProject},
    runtime::git_status::GitBranch,
};

pub(in super::super) struct PaletteControllerState {
    pub(in super::super) active_palette: Option<ActivePalette>,
    pub(in super::super) recent_projects: Vec<RecentProject>,
    pub(in super::super) input: Option<Entity<InputState>>,
    pub(in super::super) input_subscription: Option<Subscription>,
    pub(in super::super) input_needs_focus: bool,
    pub(in super::super) scroll_handle: ScrollHandle,
    pub(in super::super) git_branch_project_id: Option<ProjectId>,
    pub(in super::super) git_branch_generation: u64,
    pub(in super::super) git_branches: Vec<GitBranch>,
    pub(in super::super) git_branch_loading: bool,
    pub(in super::super) git_branch_switching: bool,
    pub(in super::super) git_branch_error: Option<String>,
    pub(in super::super) pending_git_branch_load: Option<(ProjectId, PathBuf, u64)>,
    pub(in super::super) pending_git_branch_switch: Option<(ProjectId, PathBuf, GitBranch, u64)>,
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
