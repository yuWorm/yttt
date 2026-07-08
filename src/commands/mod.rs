use crate::model::{
    layout::SplitDirection,
    split_tree::{FocusDirection, ResizeDirection},
    workspace::{Workspace, WorkspaceError},
};

const PANE_RESIZE_DELTA: f32 = 0.05;
const DEFAULT_RENAMED_TAB_TITLE: &str = "Renamed Tab";
const DEFAULT_RENAMED_PANE_TITLE: &str = "Renamed Pane";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandId {
    ProjectOpen,
    ProjectOpenRecent,
    ProjectClose,
    ProjectPalette,
    TabNew,
    TabClose,
    TabRename,
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
        Self::TabRename,
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
            Self::TabRename => "tab.rename",
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

    pub fn presentation(self) -> CommandPresentation {
        match self {
            Self::ProjectOpen => presentation("Open Project", "Choose a project directory"),
            Self::ProjectOpenRecent => {
                presentation("Open Recent Project", "Choose a recent project")
            }
            Self::ProjectClose => presentation("Close Project", "Close the selected project"),
            Self::ProjectPalette => {
                presentation("Open Project Palette", "Switch opened or recent projects")
            }
            Self::TabNew => presentation("New Tab", "Create a shell tab in the selected project"),
            Self::TabClose => presentation("Close Tab", "Close the selected tab"),
            Self::TabRename => presentation("Rename Tab", "Rename the selected tab"),
            Self::TabNext => presentation("Next Tab", "Switch to the next project tab"),
            Self::TabPrev => presentation("Previous Tab", "Switch to the previous project tab"),
            Self::TabPalette => {
                presentation("Open Tab Palette", "Switch tabs in the selected project")
            }
            Self::PaneSplitHorizontal => presentation(
                "Split Pane Horizontally",
                "Split the focused pane into top and bottom panes",
            ),
            Self::PaneSplitVertical => presentation(
                "Split Pane Vertically",
                "Split the focused pane into left and right panes",
            ),
            Self::PaneClose => presentation("Close Pane", "Close the focused pane"),
            Self::PaneFocusLeft => {
                presentation("Focus Pane Left", "Move focus to the pane on the left")
            }
            Self::PaneFocusRight => {
                presentation("Focus Pane Right", "Move focus to the pane on the right")
            }
            Self::PaneFocusUp => presentation("Focus Pane Up", "Move focus to the pane above"),
            Self::PaneFocusDown => presentation("Focus Pane Down", "Move focus to the pane below"),
            Self::PaneResizeLeft => presentation(
                "Resize Pane Left",
                "Resize the focused split toward the left",
            ),
            Self::PaneResizeRight => presentation(
                "Resize Pane Right",
                "Resize the focused split toward the right",
            ),
            Self::PaneResizeUp => presentation("Resize Pane Up", "Resize the focused split upward"),
            Self::PaneResizeDown => {
                presentation("Resize Pane Down", "Resize the focused split downward")
            }
            Self::PaneRename => presentation("Rename Pane", "Rename the focused pane"),
            Self::PanePalette => {
                presentation("Open Pane Palette", "Focus panes in the selected tab")
            }
            Self::LayoutSaveCurrent => presentation(
                "Save Current Layout",
                "Save the current layout as a local override",
            ),
            Self::LayoutExportProjectConfig => presentation(
                "Export Project Layout",
                "Write the current layout to the project config",
            ),
            Self::LayoutOpenFile => presentation(
                "Open Layout File",
                "Reveal the selected project's layout file path",
            ),
            Self::CommandPaletteOpen => {
                presentation("Open Command Palette", "Search and run commands")
            }
            Self::SettingsKeybindings => presentation(
                "Open Keybindings File",
                "Open or create the editable keybindings TOML",
            ),
            Self::SettingsNotifications => presentation(
                "Toggle Notifications",
                "Toggle system notifications for agent exits",
            ),
        }
    }

    pub fn availability(self, has_selected_project: bool) -> CommandAvailability {
        match self {
            Self::ProjectOpen | Self::ProjectOpenRecent => {
                disabled("Use the visible project actions")
            }
            Self::CommandPaletteOpen
            | Self::ProjectPalette
            | Self::SettingsKeybindings
            | Self::SettingsNotifications => enabled(),
            Self::ProjectClose
            | Self::TabNew
            | Self::TabClose
            | Self::TabRename
            | Self::TabNext
            | Self::TabPrev
            | Self::TabPalette
            | Self::PaneSplitHorizontal
            | Self::PaneSplitVertical
            | Self::PaneClose
            | Self::PaneFocusLeft
            | Self::PaneFocusRight
            | Self::PaneFocusUp
            | Self::PaneFocusDown
            | Self::PaneResizeLeft
            | Self::PaneResizeRight
            | Self::PaneResizeUp
            | Self::PaneResizeDown
            | Self::PaneRename
            | Self::PanePalette
            | Self::LayoutSaveCurrent
            | Self::LayoutExportProjectConfig
            | Self::LayoutOpenFile => {
                if has_selected_project {
                    enabled()
                } else {
                    disabled("Open a project first")
                }
            }
        }
    }
}

fn presentation(title: &'static str, description: &'static str) -> CommandPresentation {
    CommandPresentation { title, description }
}

fn enabled() -> CommandAvailability {
    CommandAvailability {
        enabled: true,
        disabled_reason: None,
    }
}

fn disabled(reason: &'static str) -> CommandAvailability {
    CommandAvailability {
        enabled: false,
        disabled_reason: Some(reason),
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Command {
    pub id: CommandId,
    pub title: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandPresentation {
    pub title: &'static str,
    pub description: &'static str,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CommandAvailability {
    pub enabled: bool,
    pub disabled_reason: Option<&'static str>,
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
    TabClosed(String),
    TabRenamed,
    PaneSplit(String),
    PaneClosed(String),
    PaneRenamed,
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
        CommandId::TabClose => workspace
            .close_selected_tab()
            .map(CommandOutcome::TabClosed)
            .map_err(CommandDispatchError::from),
        CommandId::TabRename => workspace
            .rename_selected_tab(DEFAULT_RENAMED_TAB_TITLE)
            .map(|_| CommandOutcome::TabRenamed)
            .map_err(CommandDispatchError::from),
        CommandId::PaneSplitHorizontal => workspace
            .split_focused_pane(SplitDirection::Horizontal)
            .map(CommandOutcome::PaneSplit)
            .map_err(CommandDispatchError::from),
        CommandId::PaneSplitVertical => workspace
            .split_focused_pane(SplitDirection::Vertical)
            .map(CommandOutcome::PaneSplit)
            .map_err(CommandDispatchError::from),
        CommandId::PaneClose => match workspace.close_focused_pane() {
            Ok(pane_id) => Ok(CommandOutcome::PaneClosed(pane_id)),
            Err(WorkspaceError::CannotCloseLastPane) => workspace
                .close_selected_tab()
                .map(CommandOutcome::TabClosed)
                .map_err(CommandDispatchError::from),
            Err(error) => Err(CommandDispatchError::from(error)),
        },
        CommandId::PaneRename => workspace
            .rename_focused_pane(DEFAULT_RENAMED_PANE_TITLE)
            .map(|_| CommandOutcome::PaneRenamed)
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
