use gpui::Keystroke;

use crate::{commands::CommandId, ui::interaction::input_owner::InputOwnerKind};

pub fn workspace_runtime_command_allowed(owner: InputOwnerKind, command: CommandId) -> bool {
    match owner {
        InputOwnerKind::Workspace => true,
        InputOwnerKind::Editor => editor_runtime_command_allowed(command),
        InputOwnerKind::Palette
        | InputOwnerKind::Settings
        | InputOwnerKind::Dialog
        | InputOwnerKind::KeybindingRecorder
        | InputOwnerKind::ContextMenu
        | InputOwnerKind::Popover => false,
    }
}

pub fn workspace_command_for_keystroke(
    owner: InputOwnerKind,
    keystroke: &Keystroke,
    command_for_keystroke: impl FnOnce(&Keystroke) -> Option<CommandId>,
    terminal_should_receive: impl FnOnce(&Keystroke) -> bool,
) -> Option<CommandId> {
    let command = command_for_keystroke(keystroke)?;
    if !workspace_runtime_command_allowed(owner, command) {
        return None;
    }

    if owner == InputOwnerKind::Workspace && terminal_should_receive(keystroke) {
        return None;
    }

    Some(command)
}

fn editor_runtime_command_allowed(command: CommandId) -> bool {
    matches!(
        command,
        CommandId::FileSave
            | CommandId::TabClose
            | CommandId::TabNext
            | CommandId::TabPrev
            | CommandId::ProjectPanelToggle
            | CommandId::ProjectPanelRefresh
            | CommandId::CommandPaletteOpen
            | CommandId::ProjectOpenRecent
            | CommandId::ProjectPalette
            | CommandId::TabPalette
    )
}
