use std::path::PathBuf;

use gpui::{Entity, Subscription};
use gpui_component::input::InputState;

use crate::{
    model::ids::ProjectId,
    ui::{
        interaction::input_owner::InputOwnerStack, workbench::layout_editor::LayoutEditorSession,
    },
};

use super::super::{GitDiffPanel, PendingKeybindingEdit, PendingTabRename};

#[derive(Default)]
pub(in super::super) struct OverlayControllerState {
    pub(in super::super) pending_close_project_id: Option<ProjectId>,
    pub(in super::super) pending_tab_rename: Option<PendingTabRename>,
    pub(in super::super) pending_keybinding_edit: Option<PendingKeybindingEdit>,
    pub(in super::super) input_owner_stack: InputOwnerStack,
    pub(in super::super) tab_rename_input: Option<Entity<InputState>>,
    pub(in super::super) tab_rename_input_subscription: Option<Subscription>,
    pub(in super::super) tab_rename_input_needs_focus: bool,
    pub(in super::super) keybinding_recorder_needs_focus: bool,
    pub(in super::super) layout_toml_editor: Option<LayoutEditorSession>,
    pub(in super::super) layout_toml_input: Option<Entity<InputState>>,
    pub(in super::super) layout_toml_input_subscription: Option<Subscription>,
    pub(in super::super) layout_toml_input_needs_focus: bool,
    pub(in super::super) git_diff_panel: Option<GitDiffPanel>,
    pub(in super::super) git_diff_generation: u64,
    pub(in super::super) pending_git_diff_load: Option<(ProjectId, PathBuf, u64)>,
}
