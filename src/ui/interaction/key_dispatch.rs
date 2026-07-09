use gpui::Keystroke;

use crate::{commands::CommandId, ui::interaction::input_owner::InputOwnerKind};

pub fn workspace_runtime_command_allowed(owner: InputOwnerKind) -> bool {
    owner == InputOwnerKind::Workspace
}

pub fn workspace_command_for_keystroke(
    owner: InputOwnerKind,
    keystroke: &Keystroke,
    command_for_keystroke: impl FnOnce(&Keystroke) -> Option<CommandId>,
    terminal_should_receive: impl FnOnce(&Keystroke) -> bool,
) -> Option<CommandId> {
    if !workspace_runtime_command_allowed(owner) || terminal_should_receive(keystroke) {
        return None;
    }

    command_for_keystroke(keystroke)
}
