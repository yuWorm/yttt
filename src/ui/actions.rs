use gpui::{KeyBinding, actions};

pub const WORKSPACE_CONTEXT: &str = "Workspace";

actions!(
    yttt,
    [
        OpenCommandPalette,
        OpenProjectPalette,
        OpenTabPalette,
        OpenPanePalette,
        PaletteSelectNext,
        PaletteSelectPrev,
        PaletteConfirm,
        PaletteCancel,
        TabNext,
        TabPrev,
        PaneSplitVertical,
        PaneSplitHorizontal,
        PaneClose,
    ]
);

pub fn default_ui_keybindings() -> Vec<KeyBinding> {
    vec![
        KeyBinding::new("cmd-p", OpenCommandPalette, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("ctrl-p", OpenCommandPalette, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("cmd-shift-o", OpenProjectPalette, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("ctrl-shift-o", OpenProjectPalette, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("cmd-j", OpenTabPalette, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("ctrl-j", OpenTabPalette, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("cmd-k", OpenPanePalette, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("ctrl-k", OpenPanePalette, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("down", PaletteSelectNext, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("up", PaletteSelectPrev, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("enter", PaletteConfirm, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("escape", PaletteCancel, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("cmd-]", TabNext, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("ctrl-tab", TabNext, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("cmd-[", TabPrev, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("ctrl-shift-tab", TabPrev, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("cmd-d", PaneSplitVertical, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("ctrl-d", PaneSplitVertical, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("cmd-shift-d", PaneSplitHorizontal, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("ctrl-shift-d", PaneSplitHorizontal, Some(WORKSPACE_CONTEXT)),
        KeyBinding::new("cmd-w", PaneClose, Some(WORKSPACE_CONTEXT)),
    ]
}
