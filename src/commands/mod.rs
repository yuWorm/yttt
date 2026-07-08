use crate::model::{
    layout::SplitDirection,
    split_tree::{FocusDirection, ResizeDirection},
    workspace::{Workspace, WorkspaceError},
};

const PANE_RESIZE_DELTA: f32 = 0.05;

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

    pub fn contains_str(&self, command_id: &str) -> bool {
        self.commands
            .iter()
            .any(|command| command.id.as_str() == command_id)
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommandOutcome {
    None,
    TabCreated(String),
    PaneSplit(String),
    PaneClosed(String),
    PaneFocused(String),
    PaneResized,
}

#[derive(Debug, thiserror::Error)]
pub enum CommandDispatchError {
    #[error("{0}")]
    Workspace(#[from] WorkspaceError),
    #[error("command is not handled by workspace dispatcher: {0:?}")]
    Unsupported(CommandId),
}

pub fn dispatch_workspace_command(
    workspace: &mut Workspace,
    command_id: CommandId,
) -> Result<CommandOutcome, CommandDispatchError> {
    match command_id {
        CommandId::TabNext => {
            workspace.select_next_tab()?;
            Ok(CommandOutcome::None)
        }
        CommandId::TabPrev => {
            workspace.select_prev_tab()?;
            Ok(CommandOutcome::None)
        }
        CommandId::TabNew => workspace
            .create_shell_tab()
            .map(CommandOutcome::TabCreated)
            .map_err(CommandDispatchError::from),
        CommandId::PaneSplitHorizontal => workspace
            .split_focused_pane(SplitDirection::Horizontal)
            .map(CommandOutcome::PaneSplit)
            .map_err(CommandDispatchError::from),
        CommandId::PaneSplitVertical => workspace
            .split_focused_pane(SplitDirection::Vertical)
            .map(CommandOutcome::PaneSplit)
            .map_err(CommandDispatchError::from),
        CommandId::PaneClose => workspace
            .close_focused_pane()
            .map(CommandOutcome::PaneClosed)
            .map_err(CommandDispatchError::from),
        CommandId::PaneFocusLeft => workspace
            .focus_pane_direction(FocusDirection::Left)
            .map(CommandOutcome::PaneFocused)
            .map_err(CommandDispatchError::from),
        CommandId::PaneFocusRight => workspace
            .focus_pane_direction(FocusDirection::Right)
            .map(CommandOutcome::PaneFocused)
            .map_err(CommandDispatchError::from),
        CommandId::PaneFocusUp => workspace
            .focus_pane_direction(FocusDirection::Up)
            .map(CommandOutcome::PaneFocused)
            .map_err(CommandDispatchError::from),
        CommandId::PaneFocusDown => workspace
            .focus_pane_direction(FocusDirection::Down)
            .map(CommandOutcome::PaneFocused)
            .map_err(CommandDispatchError::from),
        CommandId::PaneResizeLeft => workspace
            .resize_focused_split(ResizeDirection::Left, PANE_RESIZE_DELTA)
            .map(|_| CommandOutcome::PaneResized)
            .map_err(CommandDispatchError::from),
        CommandId::PaneResizeRight => workspace
            .resize_focused_split(ResizeDirection::Right, PANE_RESIZE_DELTA)
            .map(|_| CommandOutcome::PaneResized)
            .map_err(CommandDispatchError::from),
        CommandId::PaneResizeUp => workspace
            .resize_focused_split(ResizeDirection::Up, PANE_RESIZE_DELTA)
            .map(|_| CommandOutcome::PaneResized)
            .map_err(CommandDispatchError::from),
        CommandId::PaneResizeDown => workspace
            .resize_focused_split(ResizeDirection::Down, PANE_RESIZE_DELTA)
            .map(|_| CommandOutcome::PaneResized)
            .map_err(CommandDispatchError::from),
        _ => Err(CommandDispatchError::Unsupported(command_id)),
    }
}
