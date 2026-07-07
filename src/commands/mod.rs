#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandId {
    ProjectOpen,
    ProjectOpenRecent,
    ProjectClose,
    ProjectPalette,
    TabNew,
    TabClose,
    TabNext,
    TabPrev,
    TabPalette,
    PaneSplitHorizontal,
    PaneSplitVertical,
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
    PanePalette,
    LayoutSaveCurrent,
    LayoutExportProjectConfig,
    LayoutOpenFile,
    CommandPaletteOpen,
    SettingsKeybindings,
    SettingsNotifications,
}

impl CommandId {
    pub const ALL: &'static [Self] = &[
        Self::ProjectOpen,
        Self::ProjectOpenRecent,
        Self::ProjectClose,
        Self::ProjectPalette,
        Self::TabNew,
        Self::TabClose,
        Self::TabNext,
        Self::TabPrev,
        Self::TabPalette,
        Self::PaneSplitHorizontal,
        Self::PaneSplitVertical,
        Self::PaneClose,
        Self::PaneFocusLeft,
        Self::PaneFocusRight,
        Self::PaneFocusUp,
        Self::PaneFocusDown,
        Self::PaneResizeLeft,
        Self::PaneResizeRight,
        Self::PaneResizeUp,
        Self::PaneResizeDown,
        Self::PaneRename,
        Self::PanePalette,
        Self::LayoutSaveCurrent,
        Self::LayoutExportProjectConfig,
        Self::LayoutOpenFile,
        Self::CommandPaletteOpen,
        Self::SettingsKeybindings,
        Self::SettingsNotifications,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::ProjectOpen => "project.open",
            Self::ProjectOpenRecent => "project.open_recent",
            Self::ProjectClose => "project.close",
            Self::ProjectPalette => "project.palette",
            Self::TabNew => "tab.new",
            Self::TabClose => "tab.close",
            Self::TabNext => "tab.next",
            Self::TabPrev => "tab.prev",
            Self::TabPalette => "tab.palette",
            Self::PaneSplitHorizontal => "pane.split_horizontal",
            Self::PaneSplitVertical => "pane.split_vertical",
            Self::PaneClose => "pane.close",
            Self::PaneFocusLeft => "pane.focus_left",
            Self::PaneFocusRight => "pane.focus_right",
            Self::PaneFocusUp => "pane.focus_up",
            Self::PaneFocusDown => "pane.focus_down",
            Self::PaneResizeLeft => "pane.resize_left",
            Self::PaneResizeRight => "pane.resize_right",
            Self::PaneResizeUp => "pane.resize_up",
            Self::PaneResizeDown => "pane.resize_down",
            Self::PaneRename => "pane.rename",
            Self::PanePalette => "pane.palette",
            Self::LayoutSaveCurrent => "layout.save_current",
            Self::LayoutExportProjectConfig => "layout.export_project_config",
            Self::LayoutOpenFile => "layout.open_file",
            Self::CommandPaletteOpen => "command_palette.open",
            Self::SettingsKeybindings => "settings.keybindings",
            Self::SettingsNotifications => "settings.notifications",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: CommandId,
    pub title: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandRegistry {
    commands: Vec<Command>,
}

impl CommandRegistry {
    pub fn contains(&self, command_id: CommandId) -> bool {
        self.commands.iter().any(|command| command.id == command_id)
    }

    pub fn commands(&self) -> &[Command] {
        &self.commands
    }
}

pub fn default_registry() -> CommandRegistry {
    CommandRegistry {
        commands: CommandId::ALL
            .iter()
            .copied()
            .map(|id| Command {
                id,
                title: id.as_str(),
            })
            .collect(),
    }
}
