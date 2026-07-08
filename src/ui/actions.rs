use gpui::{KeyBinding, actions};

use crate::commands::CommandId;

pub const WORKSPACE_CONTEXT: &str = "Workspace";

actions!(
    yttt,
    [
        OpenCommandPalette,
        OpenProject,
        OpenProjectPalette,
        OpenTabPalette,
        OpenPanePalette,
        PaletteSelectNext,
        PaletteSelectPrev,
        PaletteConfirm,
        PaletteCancel,
        TabNew,
        TabNext,
        TabPrev,
        PaneSplitVertical,
        PaneSplitHorizontal,
        PaneClose,
        PaneFocusLeft,
        PaneFocusRight,
        PaneFocusUp,
        PaneFocusDown,
        PaneResizeLeft,
        PaneResizeRight,
        PaneResizeUp,
        PaneResizeDown,
    ]
);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UiKeybindingSpec {
    pub keys: &'static str,
    pub command: CommandId,
}

const DEFAULT_UI_KEYBINDING_SPECS: &[UiKeybindingSpec] = &[
    UiKeybindingSpec {
        keys: "cmd-o",
        command: CommandId::ProjectOpen,
    },
    UiKeybindingSpec {
        keys: "ctrl-o",
        command: CommandId::ProjectOpen,
    },
    UiKeybindingSpec {
        keys: "cmd-p",
        command: CommandId::CommandPaletteOpen,
    },
    UiKeybindingSpec {
        keys: "ctrl-p",
        command: CommandId::CommandPaletteOpen,
    },
    UiKeybindingSpec {
        keys: "cmd-shift-o",
        command: CommandId::ProjectPalette,
    },
    UiKeybindingSpec {
        keys: "ctrl-shift-o",
        command: CommandId::ProjectPalette,
    },
    UiKeybindingSpec {
        keys: "cmd-j",
        command: CommandId::TabPalette,
    },
    UiKeybindingSpec {
        keys: "ctrl-j",
        command: CommandId::TabPalette,
    },
    UiKeybindingSpec {
        keys: "cmd-k",
        command: CommandId::PanePalette,
    },
    UiKeybindingSpec {
        keys: "ctrl-k",
        command: CommandId::PanePalette,
    },
    UiKeybindingSpec {
        keys: "cmd-t",
        command: CommandId::TabNew,
    },
    UiKeybindingSpec {
        keys: "ctrl-t",
        command: CommandId::TabNew,
    },
    UiKeybindingSpec {
        keys: "cmd-]",
        command: CommandId::TabNext,
    },
    UiKeybindingSpec {
        keys: "ctrl-tab",
        command: CommandId::TabNext,
    },
    UiKeybindingSpec {
        keys: "cmd-[",
        command: CommandId::TabPrev,
    },
    UiKeybindingSpec {
        keys: "ctrl-shift-tab",
        command: CommandId::TabPrev,
    },
    UiKeybindingSpec {
        keys: "cmd-d",
        command: CommandId::PaneSplitVertical,
    },
    UiKeybindingSpec {
        keys: "ctrl-d",
        command: CommandId::PaneSplitVertical,
    },
    UiKeybindingSpec {
        keys: "cmd-shift-d",
        command: CommandId::PaneSplitHorizontal,
    },
    UiKeybindingSpec {
        keys: "ctrl-shift-d",
        command: CommandId::PaneSplitHorizontal,
    },
    UiKeybindingSpec {
        keys: "cmd-w",
        command: CommandId::PaneClose,
    },
    UiKeybindingSpec {
        keys: "ctrl-w",
        command: CommandId::PaneClose,
    },
    UiKeybindingSpec {
        keys: "cmd-alt-left",
        command: CommandId::PaneFocusLeft,
    },
    UiKeybindingSpec {
        keys: "cmd-alt-right",
        command: CommandId::PaneFocusRight,
    },
    UiKeybindingSpec {
        keys: "cmd-alt-up",
        command: CommandId::PaneFocusUp,
    },
    UiKeybindingSpec {
        keys: "cmd-alt-down",
        command: CommandId::PaneFocusDown,
    },
    UiKeybindingSpec {
        keys: "ctrl-alt-left",
        command: CommandId::PaneFocusLeft,
    },
    UiKeybindingSpec {
        keys: "ctrl-alt-right",
        command: CommandId::PaneFocusRight,
    },
    UiKeybindingSpec {
        keys: "ctrl-alt-up",
        command: CommandId::PaneFocusUp,
    },
    UiKeybindingSpec {
        keys: "ctrl-alt-down",
        command: CommandId::PaneFocusDown,
    },
    UiKeybindingSpec {
        keys: "cmd-alt-shift-left",
        command: CommandId::PaneResizeLeft,
    },
    UiKeybindingSpec {
        keys: "cmd-alt-shift-right",
        command: CommandId::PaneResizeRight,
    },
    UiKeybindingSpec {
        keys: "cmd-alt-shift-up",
        command: CommandId::PaneResizeUp,
    },
    UiKeybindingSpec {
        keys: "cmd-alt-shift-down",
        command: CommandId::PaneResizeDown,
    },
    UiKeybindingSpec {
        keys: "ctrl-alt-shift-left",
        command: CommandId::PaneResizeLeft,
    },
    UiKeybindingSpec {
        keys: "ctrl-alt-shift-right",
        command: CommandId::PaneResizeRight,
    },
    UiKeybindingSpec {
        keys: "ctrl-alt-shift-up",
        command: CommandId::PaneResizeUp,
    },
    UiKeybindingSpec {
        keys: "ctrl-alt-shift-down",
        command: CommandId::PaneResizeDown,
    },
];

pub fn default_ui_keybinding_specs() -> &'static [UiKeybindingSpec] {
    DEFAULT_UI_KEYBINDING_SPECS
}

pub fn default_ui_keybindings() -> Vec<KeyBinding> {
    let mut bindings: Vec<_> = default_ui_keybinding_specs()
        .iter()
        .map(command_keybinding)
        .collect();
    bindings.extend([
        KeyBinding::new("down", PaletteSelectNext, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("up", PaletteSelectPrev, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("enter", PaletteConfirm, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("escape", PaletteCancel, Some(WORKSPACE_CONTEXT)),
    ]);
    bindings
}

fn command_keybinding(spec: &UiKeybindingSpec) -> KeyBinding {
    match spec.command {
        CommandId::ProjectOpen => KeyBinding::new(spec.keys, OpenProject, Some(WORKSPACE_CONTEXT)),
        CommandId::CommandPaletteOpen => {
            KeyBinding::new(spec.keys, OpenCommandPalette, Some(WORKSPACE_CONTEXT))
        }
        CommandId::ProjectPalette => {
            KeyBinding::new(spec.keys, OpenProjectPalette, Some(WORKSPACE_CONTEXT))
        }
        CommandId::TabPalette => {
            KeyBinding::new(spec.keys, OpenTabPalette, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PanePalette => {
            KeyBinding::new(spec.keys, OpenPanePalette, Some(WORKSPACE_CONTEXT))
        }
        CommandId::TabNew => KeyBinding::new(spec.keys, TabNew, Some(WORKSPACE_CONTEXT)),
        CommandId::TabNext => KeyBinding::new(spec.keys, TabNext, Some(WORKSPACE_CONTEXT)),
        CommandId::TabPrev => KeyBinding::new(spec.keys, TabPrev, Some(WORKSPACE_CONTEXT)),
        CommandId::PaneSplitVertical => {
            KeyBinding::new(spec.keys, PaneSplitVertical, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PaneSplitHorizontal => {
            KeyBinding::new(spec.keys, PaneSplitHorizontal, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PaneClose => KeyBinding::new(spec.keys, PaneClose, Some(WORKSPACE_CONTEXT)),
        CommandId::PaneFocusLeft => {
            KeyBinding::new(spec.keys, PaneFocusLeft, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PaneFocusRight => {
            KeyBinding::new(spec.keys, PaneFocusRight, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PaneFocusUp => KeyBinding::new(spec.keys, PaneFocusUp, Some(WORKSPACE_CONTEXT)),
        CommandId::PaneFocusDown => {
            KeyBinding::new(spec.keys, PaneFocusDown, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PaneResizeLeft => {
            KeyBinding::new(spec.keys, PaneResizeLeft, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PaneResizeRight => {
            KeyBinding::new(spec.keys, PaneResizeRight, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PaneResizeUp => {
            KeyBinding::new(spec.keys, PaneResizeUp, Some(WORKSPACE_CONTEXT))
        }
        CommandId::PaneResizeDown => {
            KeyBinding::new(spec.keys, PaneResizeDown, Some(WORKSPACE_CONTEXT))
        }
        _ => unreachable!(
            "unsupported default UI keybinding command: {:?}",
            spec.command
        ),
    }
}
