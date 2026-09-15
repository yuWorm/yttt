use gpui::Keystroke;

use crate::{
    commands::{CommandContext, CommandId},
    ui::interaction::input_owner::InputOwnerKind,
};

#[cfg(test)]
use crate::commands::ActiveSurface;

pub fn workspace_runtime_command_allowed(
    owner: InputOwnerKind,
    command: CommandId,
    context: CommandContext,
) -> bool {
    match owner {
        InputOwnerKind::Workspace | InputOwnerKind::Editor => {
            command.availability_for_context(context).enabled
        }
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
    context: CommandContext,
    keystroke: &Keystroke,
    command_for_keystroke: impl FnOnce(&Keystroke) -> Option<CommandId>,
    terminal_should_receive: impl FnOnce(&Keystroke) -> bool,
) -> Option<CommandId> {
    let command = command_for_keystroke(keystroke)?;
    if !workspace_runtime_command_allowed(owner, command, context) {
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
            CommandContext::local_controller(true, ActiveSurface::Terminal),
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
    #[test]
    fn observer_shortcuts_reject_shared_mutations_but_keep_navigation() {
        let context = CommandContext {
            has_selected_project: true,
            active_surface: ActiveSurface::Terminal,
            shared_editing_enabled: false,
            is_remote: true,
        };
        let keystroke = Keystroke::parse("cmd-shift-t").unwrap();

        assert_eq!(
            workspace_command_for_keystroke(
                InputOwnerKind::Workspace,
                context,
                &keystroke,
                |_| Some(CommandId::TabNew),
                |_| false,
            ),
            None
        );
        assert_eq!(
            workspace_command_for_keystroke(
                InputOwnerKind::Workspace,
                context,
                &keystroke,
                |_| Some(CommandId::TabNext),
                |_| false,
            ),
            Some(CommandId::TabNext)
        );
    }
}
