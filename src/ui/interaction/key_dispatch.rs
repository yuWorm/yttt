use gpui::Keystroke;

use crate::{
    commands::{ActiveSurface, CommandContext, CommandId},
    ui::interaction::input_owner::InputOwnerKind,
};

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

    if owner == InputOwnerKind::Workspace
        && terminal_should_receive(keystroke)
        && !uses_workspace_shortcut_modifier(keystroke)
    {
        return None;
    }

    Some(command)
}

fn uses_workspace_shortcut_modifier(keystroke: &Keystroke) -> bool {
    uses_workspace_shortcut_modifier_for_platform(keystroke, cfg!(target_os = "macos"))
}

fn uses_workspace_shortcut_modifier_for_platform(keystroke: &Keystroke, macos: bool) -> bool {
    if macos {
        keystroke.modifiers.platform
    } else {
        keystroke.modifiers.control
    }
}

fn editor_runtime_command_allowed(command: CommandId) -> bool {
    command
        .availability_for_context(CommandContext {
            has_selected_project: true,
            active_surface: ActiveSurface::File,
        })
        .enabled
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_primary_shortcut_wins_over_focused_terminal_input() {
        let keys = if cfg!(target_os = "macos") {
            "cmd-p"
        } else {
            "ctrl-p"
        };
        let keystroke = Keystroke::parse(keys).unwrap();

        let command = workspace_command_for_keystroke(
            InputOwnerKind::Workspace,
            &keystroke,
            |_| Some(CommandId::CommandPaletteOpen),
            |_| true,
        );

        assert_eq!(command, Some(CommandId::CommandPaletteOpen));
    }

    #[test]
    fn windows_control_shortcuts_use_the_workspace_modifier() {
        let keystroke = Keystroke::parse("ctrl-p").unwrap();

        assert!(uses_workspace_shortcut_modifier_for_platform(
            &keystroke, false
        ));
    }
}
