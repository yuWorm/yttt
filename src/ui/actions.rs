use std::{borrow::Cow, collections::HashSet};

use gpui::{KeyBinding, KeybindingKeystroke, Keystroke, actions};

use crate::{
    commands::{CommandId, CommandRegistry},
    config::{
        keybindings::{KeybindingsConfig, load_keybindings},
        paths::AppConfigPaths,
    },
};

pub const WORKSPACE_CONTEXT: &str = "Workspace";

actions!(
    yttt,
    [
        OpenCommandPalette,
        OpenProject,
        ProjectClose,
        OpenProjectPalette,
        OpenTabPalette,
        OpenPanePalette,
        PaletteSelectNext,
        PaletteSelectPrev,
        PaletteConfirm,
        PaletteCancel,
        TabNew,
        TabClose,
        TabRename,
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
        PaneRename,
        LayoutDefaultEdit,
        LayoutDefaultReset,
        LayoutDefaultReload,
        LayoutProjectEdit,
        LayoutSaveCurrent,
        LayoutExportProjectConfig,
        LayoutResetLocalOverride,
        LayoutOpenFile,
        SettingsOpen,
        SettingsKeybindings,
        SettingsNotifications,
    ]
);

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiKeybindingSpec {
    pub keys: Cow<'static, str>,
    pub command: CommandId,
}

const DEFAULT_UI_KEYBINDING_SPECS: &[UiKeybindingSpec] = &[
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-o"),
        command: CommandId::ProjectOpen,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-o"),
        command: CommandId::ProjectOpen,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-p"),
        command: CommandId::CommandPaletteOpen,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-p"),
        command: CommandId::CommandPaletteOpen,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-,"),
        command: CommandId::SettingsOpen,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-,"),
        command: CommandId::SettingsOpen,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-shift-o"),
        command: CommandId::ProjectPalette,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-shift-o"),
        command: CommandId::ProjectPalette,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-j"),
        command: CommandId::TabPalette,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-j"),
        command: CommandId::TabPalette,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-k"),
        command: CommandId::PanePalette,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-k"),
        command: CommandId::PanePalette,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-t"),
        command: CommandId::TabNew,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-t"),
        command: CommandId::TabNew,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-]"),
        command: CommandId::TabNext,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-tab"),
        command: CommandId::TabNext,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-["),
        command: CommandId::TabPrev,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-shift-tab"),
        command: CommandId::TabPrev,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-d"),
        command: CommandId::PaneSplitVertical,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-d"),
        command: CommandId::PaneSplitVertical,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-shift-d"),
        command: CommandId::PaneSplitHorizontal,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-shift-d"),
        command: CommandId::PaneSplitHorizontal,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-w"),
        command: CommandId::PaneClose,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-w"),
        command: CommandId::PaneClose,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-alt-left"),
        command: CommandId::PaneFocusLeft,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-alt-right"),
        command: CommandId::PaneFocusRight,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-alt-up"),
        command: CommandId::PaneFocusUp,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-alt-down"),
        command: CommandId::PaneFocusDown,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-alt-left"),
        command: CommandId::PaneFocusLeft,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-alt-right"),
        command: CommandId::PaneFocusRight,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-alt-up"),
        command: CommandId::PaneFocusUp,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-alt-down"),
        command: CommandId::PaneFocusDown,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-alt-shift-left"),
        command: CommandId::PaneResizeLeft,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-alt-shift-right"),
        command: CommandId::PaneResizeRight,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-alt-shift-up"),
        command: CommandId::PaneResizeUp,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("cmd-alt-shift-down"),
        command: CommandId::PaneResizeDown,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-alt-shift-left"),
        command: CommandId::PaneResizeLeft,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-alt-shift-right"),
        command: CommandId::PaneResizeRight,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-alt-shift-up"),
        command: CommandId::PaneResizeUp,
    },
    UiKeybindingSpec {
        keys: Cow::Borrowed("ctrl-alt-shift-down"),
        command: CommandId::PaneResizeDown,
    },
];

pub fn default_ui_keybinding_specs() -> &'static [UiKeybindingSpec] {
    DEFAULT_UI_KEYBINDING_SPECS
}

pub fn app_startup_keybindings() -> Vec<KeyBinding> {
    built_in_ui_keybindings()
}

pub fn load_app_keybindings(paths: &AppConfigPaths, registry: &CommandRegistry) -> Vec<KeyBinding> {
    let _ = load_keybindings(paths, registry);
    app_startup_keybindings()
}

pub fn ui_keybinding_specs_from_config(
    config: &KeybindingsConfig,
    registry: &CommandRegistry,
) -> Vec<UiKeybindingSpec> {
    let conflicting_keys: HashSet<_> = config
        .conflicts()
        .into_iter()
        .map(|conflict| conflict.keys)
        .collect();

    config
        .bindings
        .iter()
        .filter(|binding| !conflicting_keys.contains(&normalize_keys(&binding.keys)))
        .filter_map(|binding| {
            let command = CommandId::from_str_id(&binding.command)?;
            if !registry.contains(command) || !command_has_ui_action(command) {
                return None;
            }

            Some(UiKeybindingSpec {
                keys: Cow::Owned(normalize_keys(&binding.keys)),
                command,
            })
        })
        .collect()
}

pub fn runtime_command_for_keystroke(
    specs: &[UiKeybindingSpec],
    keystroke: &Keystroke,
) -> Option<CommandId> {
    specs
        .iter()
        .find(|spec| {
            Keystroke::parse(spec.keys.as_ref())
                .map(KeybindingKeystroke::from_keystroke)
                .map(|target| keystroke.should_match(&target))
                .unwrap_or(false)
        })
        .map(|spec| spec.command)
}

fn built_in_ui_keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("down", PaletteSelectNext, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("up", PaletteSelectPrev, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("enter", PaletteConfirm, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("escape", PaletteCancel, Some(WORKSPACE_CONTEXT)),
    ]
}

fn command_has_ui_action(command: CommandId) -> bool {
    !matches!(command, CommandId::ProjectOpenRecent)
}

fn normalize_keys(keys: &str) -> String {
    keys.trim().to_ascii_lowercase()
}
