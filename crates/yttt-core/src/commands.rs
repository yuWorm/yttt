use crate::model::{
    editor::WorkItemId,
    host_workspace::{NewTab, WorkspaceIntent},
    layout::SplitDirection,
    split_tree::{FocusDirection, ResizeDirection},
    workspace::{Workspace, WorkspaceError},
};

const PANE_RESIZE_DELTA: f32 = 0.05;
const DEFAULT_RENAMED_TAB_TITLE: &str = "Renamed Tab";
const DEFAULT_RENAMED_PANE_TITLE: &str = "Renamed Pane";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ActiveSurface {
    None,
    Terminal,
    File,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandContext {
    pub has_selected_project: bool,
    pub active_surface: ActiveSurface,
    pub shared_editing_enabled: bool,
    pub is_remote: bool,
}

impl CommandContext {
    pub const fn local_controller(
        has_selected_project: bool,
        active_surface: ActiveSurface,
    ) -> Self {
        Self {
            has_selected_project,
            active_surface,
            shared_editing_enabled: true,
            is_remote: false,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CommandId {
    ApplicationQuit,
    ProjectCreate,
    ProjectOpen,
    ProjectOpenSsh,
    ConnectExistingHost,
    ProjectOpenRecent,
    ProjectClose,
    ProjectPalette,
    ProjectOpenedPalette,
    ProjectPanelToggle,
    ProjectPanelRefresh,
    GitBranchSwitch,
    GitDiffOpen,
    FileFind,
    FileSave,
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
    LayoutDefaultEdit,
    LayoutDefaultReset,
    LayoutDefaultReload,
    LayoutProjectEdit,
    LayoutSaveCurrent,
    LayoutExportProjectConfig,
    LayoutResetLocalOverride,
    LayoutOpenFile,
    CommandPaletteOpen,
    SettingsOpen,
    SettingsKeybindings,
    SettingsNotifications,
}

impl CommandId {
    pub const ALL: &'static [Self] = &[
        Self::ApplicationQuit,
        Self::ProjectCreate,
        Self::ProjectOpen,
        Self::ProjectOpenSsh,
        Self::ConnectExistingHost,
        Self::ProjectOpenRecent,
        Self::ProjectClose,
        Self::ProjectPalette,
        Self::ProjectOpenedPalette,
        Self::ProjectPanelToggle,
        Self::ProjectPanelRefresh,
        Self::FileFind,
        Self::FileSave,
        Self::GitBranchSwitch,
        Self::GitDiffOpen,
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
        Self::LayoutDefaultEdit,
        Self::LayoutDefaultReset,
        Self::LayoutDefaultReload,
        Self::LayoutProjectEdit,
        Self::LayoutSaveCurrent,
        Self::LayoutExportProjectConfig,
        Self::LayoutResetLocalOverride,
        Self::LayoutOpenFile,
        Self::CommandPaletteOpen,
        Self::SettingsOpen,
        Self::SettingsKeybindings,
        Self::SettingsNotifications,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ApplicationQuit => "application.quit",
            Self::ProjectCreate => "project.create",
            Self::ProjectOpen => "project.open",
            Self::ProjectOpenSsh => "project.open_ssh",
            Self::ConnectExistingHost => "environment.connect_existing_host",
            Self::ProjectOpenRecent => "project.open_recent",
            Self::ProjectClose => "project.close",
            Self::ProjectPalette => "project.palette",
            Self::ProjectOpenedPalette => "project.opened_palette",
            Self::ProjectPanelToggle => "project_panel.toggle",
            Self::ProjectPanelRefresh => "project_panel.refresh",
            Self::GitBranchSwitch => "git.branch.switch",
            Self::GitDiffOpen => "git.diff.open",
            Self::FileFind => "file.find",
            Self::FileSave => "file.save",
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
            Self::LayoutDefaultEdit => "layout.default.edit",
            Self::LayoutDefaultReset => "layout.default.reset",
            Self::LayoutDefaultReload => "layout.default.reload",
            Self::LayoutProjectEdit => "layout.project.edit",
            Self::LayoutSaveCurrent => "layout.save_current",
            Self::LayoutExportProjectConfig => "layout.export_project_config",
            Self::LayoutResetLocalOverride => "layout.reset_local_override",
            Self::LayoutOpenFile => "layout.open_file",
            Self::CommandPaletteOpen => "command_palette.open",
            Self::SettingsOpen => "settings.open",
            Self::SettingsKeybindings => "settings.keybindings",
            Self::SettingsNotifications => "settings.notifications",
        }
    }

    pub fn from_str_id(command_id: &str) -> Option<Self> {
        Self::ALL
            .iter()
            .copied()
            .find(|id| id.as_str() == command_id)
    }

    pub fn presentation(self) -> CommandPresentation {
        match self {
            Self::ApplicationQuit => presentation(
                "Quit yttt",
                "Stop all host resources and quit the application",
            ),
            Self::ProjectCreate => {
                presentation("Create Project", "Create and open a new project directory")
            }
            Self::ProjectOpen => presentation("Open Project", "Choose a project directory"),
            Self::ProjectOpenSsh => {
                presentation("Open SSH Project", "Connect to and open a remote project")
            }
            Self::ConnectExistingHost => presentation(
                "Connect to existing yttt",
                "Connect to a running desktop Host without SSH or deployment",
            ),
            Self::ProjectOpenRecent => {
                presentation("Open Recent Project", "Choose a recent project")
            }
            Self::ProjectClose => presentation("Close Project", "Close the selected project"),
            Self::ProjectPalette => {
                presentation("Open Project Palette", "Switch opened or recent projects")
            }
            Self::ProjectOpenedPalette => presentation(
                "Switch Opened Project",
                "Choose from currently opened projects",
            ),
            Self::ProjectPanelToggle => {
                presentation("Toggle Project Files", "Show or hide the project file tree")
            }
            Self::ProjectPanelRefresh => {
                presentation("Refresh Project Files", "Refresh the project file tree")
            }
            Self::GitBranchSwitch => {
                presentation("Switch Git Branch", "Choose and check out a Git branch")
            }
            Self::GitDiffOpen => presentation(
                "Show Git Changes",
                "Open the selected project's working tree diff",
            ),
            Self::FileFind => presentation("Find Files", "Fuzzy-search files in opened projects"),
            Self::FileSave => presentation("Save File", "Save the active project file"),
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
            Self::PaneFocusLeft => presentation(
                "Focus Region Left",
                "Move focus to the pane, work area, or project tree on the left",
            ),
            Self::PaneFocusRight => presentation(
                "Focus Region Right",
                "Move focus to the pane, work area, or project tree on the right",
            ),
            Self::PaneFocusUp => presentation(
                "Focus Region Up",
                "Move focus to the pane or work area above",
            ),
            Self::PaneFocusDown => presentation(
                "Focus Region Down",
                "Move focus to the pane or work area below",
            ),
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
            Self::LayoutDefaultEdit => {
                presentation("Edit Default Layout", "Edit the global default layout TOML")
            }
            Self::LayoutDefaultReset => presentation(
                "Reset Default Layout",
                "Reset the global default layout to the built-in template",
            ),
            Self::LayoutDefaultReload => presentation(
                "Reload Default Layout",
                "Reload the global default layout from disk",
            ),
            Self::LayoutProjectEdit => presentation(
                "Edit Project Layout",
                "Edit the selected project's effective layout source",
            ),
            Self::LayoutSaveCurrent => presentation(
                "Save Current Layout",
                "Save the current layout as a local override",
            ),
            Self::LayoutExportProjectConfig => presentation(
                "Export Project Layout",
                "Write the current layout to the project config",
            ),
            Self::LayoutResetLocalOverride => presentation(
                "Reset Personal Layout Override",
                "Remove the selected project's personal layout override",
            ),
            Self::LayoutOpenFile => presentation(
                "Open Layout File",
                "Reveal the selected project's layout file path",
            ),
            Self::CommandPaletteOpen => {
                presentation("Open Command Palette", "Search and run commands")
            }
            Self::SettingsOpen => presentation("Open Settings", "Configure YTTT"),
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
        self.availability_for_context(CommandContext::local_controller(
            has_selected_project,
            if has_selected_project {
                ActiveSurface::Terminal
            } else {
                ActiveSurface::None
            },
        ))
    }

    pub fn availability_for_context(self, context: CommandContext) -> CommandAvailability {
        if self == Self::ConnectExistingHost && context.is_remote {
            return disabled("Connect to an existing Host from a local desktop session");
        }
        if self == Self::LayoutOpenFile && context.is_remote {
            return disabled("Reveal Host files from a local desktop session");
        }
        if self.requires_shared_editing(context) && !context.shared_editing_enabled {
            return disabled("Shared editing control is required");
        }

        match self {
            Self::ApplicationQuit
            | Self::CommandPaletteOpen
            | Self::ProjectCreate
            | Self::ProjectOpen
            | Self::ProjectOpenSsh
            | Self::ConnectExistingHost
            | Self::ProjectOpenRecent
            | Self::ProjectPalette
            | Self::SettingsOpen
            | Self::SettingsKeybindings
            | Self::SettingsNotifications
            | Self::LayoutDefaultEdit
            | Self::LayoutDefaultReset
            | Self::LayoutDefaultReload => enabled(),
            Self::ProjectClose
            | Self::ProjectPanelToggle
            | Self::ProjectPanelRefresh
            | Self::GitBranchSwitch
            | Self::GitDiffOpen
            | Self::TabPalette
            | Self::ProjectOpenedPalette
            | Self::LayoutProjectEdit
            | Self::LayoutSaveCurrent
            | Self::LayoutExportProjectConfig
            | Self::LayoutResetLocalOverride
            | Self::LayoutOpenFile
            | Self::FileFind => require_project(context),
            Self::FileSave => {
                if !context.has_selected_project {
                    disabled("Open a project first")
                } else if context.active_surface == ActiveSurface::File {
                    enabled()
                } else {
                    disabled("Focus a project file first")
                }
            }
            Self::TabClose | Self::TabNext | Self::TabPrev => {
                if !context.has_selected_project {
                    disabled("Open a project first")
                } else if matches!(
                    context.active_surface,
                    ActiveSurface::Terminal | ActiveSurface::File
                ) {
                    enabled()
                } else {
                    disabled("Open a terminal or file first")
                }
            }
            Self::TabNew => {
                if !context.has_selected_project {
                    disabled("Open a project first")
                } else if context.active_surface == ActiveSurface::File {
                    disabled("Switch to a terminal tab first")
                } else {
                    enabled()
                }
            }
            Self::PaneFocusLeft
            | Self::PaneFocusRight
            | Self::PaneFocusUp
            | Self::PaneFocusDown => {
                if !context.has_selected_project {
                    disabled("Open a project first")
                } else if matches!(
                    context.active_surface,
                    ActiveSurface::Terminal | ActiveSurface::File
                ) {
                    enabled()
                } else {
                    disabled("Open a terminal or file first")
                }
            }
            Self::TabRename
            | Self::PaneSplitHorizontal
            | Self::PaneSplitVertical
            | Self::PaneClose
            | Self::PaneResizeLeft
            | Self::PaneResizeRight
            | Self::PaneResizeUp
            | Self::PaneResizeDown
            | Self::PaneRename
            | Self::PanePalette => {
                if !context.has_selected_project {
                    disabled("Open a project first")
                } else if context.active_surface == ActiveSurface::Terminal {
                    enabled()
                } else {
                    disabled("Switch to a terminal tab first")
                }
            }
        }
    }

    const fn requires_shared_editing(self, context: CommandContext) -> bool {
        match self {
            Self::ProjectCreate
            | Self::ProjectOpen
            | Self::ProjectOpenSsh
            | Self::ProjectClose
            | Self::GitBranchSwitch
            | Self::FileSave
            | Self::TabNew
            | Self::TabRename
            | Self::PaneSplitHorizontal
            | Self::PaneSplitVertical
            | Self::PaneClose
            | Self::PaneResizeLeft
            | Self::PaneResizeRight
            | Self::PaneResizeUp
            | Self::PaneResizeDown
            | Self::PaneRename
            | Self::LayoutDefaultReset
            | Self::LayoutProjectEdit
            | Self::LayoutSaveCurrent
            | Self::LayoutExportProjectConfig
            | Self::LayoutResetLocalOverride => true,
            Self::TabClose => matches!(context.active_surface, ActiveSurface::Terminal),
            Self::ApplicationQuit
            | Self::ConnectExistingHost
            | Self::ProjectOpenRecent
            | Self::ProjectPalette
            | Self::ProjectOpenedPalette
            | Self::ProjectPanelToggle
            | Self::ProjectPanelRefresh
            | Self::GitDiffOpen
            | Self::FileFind
            | Self::TabNext
            | Self::TabPrev
            | Self::TabPalette
            | Self::PaneFocusLeft
            | Self::PaneFocusRight
            | Self::PaneFocusUp
            | Self::PaneFocusDown
            | Self::PanePalette
            | Self::LayoutDefaultEdit
            | Self::LayoutDefaultReload
            | Self::LayoutOpenFile
            | Self::CommandPaletteOpen
            | Self::SettingsOpen
            | Self::SettingsKeybindings
            | Self::SettingsNotifications => false,
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

fn require_project(context: CommandContext) -> CommandAvailability {
    if context.has_selected_project {
        enabled()
    } else {
        disabled("Open a project first")
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
    bindable_action_ids: Vec<&'static str>,
}

impl CommandRegistry {
    pub fn contains(&self, command_id: CommandId) -> bool {
        self.commands.iter().any(|command| command.id == command_id)
    }

    pub fn contains_str(&self, action_id: &str) -> bool {
        self.commands
            .iter()
            .any(|command| command.id.as_str() == action_id)
            || self.bindable_action_ids.contains(&action_id)
    }

    pub fn register_bindable_action(&mut self, action_id: &'static str) {
        if !self.contains_str(action_id) {
            self.bindable_action_ids.push(action_id);
        }
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
        bindable_action_ids: Vec::new(),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CommandDispatchError {
    #[error("{0}")]
    Workspace(#[from] WorkspaceError),
    #[error("command is not handled by workspace dispatcher: {0:?}")]
    Unsupported(CommandId),
}

/// Translates a workspace command, which acts on the current selection, into
/// the intent that names its targets by stable ID.
pub fn workspace_command_intent(
    workspace: &Workspace,
    command_id: CommandId,
) -> Result<WorkspaceIntent, CommandDispatchError> {
    let project_id = workspace
        .selected_project_id()
        .cloned()
        .ok_or(WorkspaceError::NoSelectedProject)?;
    let project = workspace
        .project(&project_id)
        .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.to_string()))?;
    let tab_id = project.selected_tab_id.clone();
    let focused_pane = || project.focused_or_first_pane_id(&tab_id);
    let relative_tab = |offset: isize| -> Result<WorkspaceIntent, CommandDispatchError> {
        let tabs = &project.layout.tabs;
        if tabs.is_empty() {
            return Err(WorkspaceError::NoTabs.into());
        }
        let current = tabs.iter().position(|tab| tab.id == tab_id).unwrap_or(0);
        let next = (current as isize + offset).rem_euclid(tabs.len() as isize) as usize;
        Ok(WorkspaceIntent::ActivateItem {
            project_id: project_id.clone(),
            item: WorkItemId::Terminal(tabs[next].id.clone()),
        })
    };
    let close_selected_tab = || -> Result<WorkspaceIntent, CommandDispatchError> {
        if project.layout.tabs.len() <= 1 {
            return Err(WorkspaceError::CannotCloseLastTab.into());
        }
        if project.layout.tab(&tab_id).is_none() {
            return Err(WorkspaceError::TabNotFound(tab_id.clone()).into());
        }
        Ok(WorkspaceIntent::CloseTabs {
            project_id: project_id.clone(),
            tab_ids: vec![tab_id.clone()],
        })
    };
    let split = |direction| -> Result<WorkspaceIntent, CommandDispatchError> {
        Ok(WorkspaceIntent::SplitPane {
            project_id: project_id.clone(),
            tab_id: tab_id.clone(),
            pane_id: focused_pane()?,
            direction,
            command: String::new(),
        })
    };
    let focus = |direction| WorkspaceIntent::FocusAdjacentPane {
        project_id: project_id.clone(),
        tab_id: tab_id.clone(),
        direction,
    };
    let resize = |direction| -> Result<WorkspaceIntent, CommandDispatchError> {
        Ok(WorkspaceIntent::ResizePaneSplit {
            project_id: project_id.clone(),
            tab_id: tab_id.clone(),
            pane_id: focused_pane()?,
            direction,
            delta: PANE_RESIZE_DELTA,
        })
    };
    match command_id {
        CommandId::TabNext => relative_tab(1),
        CommandId::TabPrev => relative_tab(-1),
        CommandId::TabNew => Ok(WorkspaceIntent::CreateTab {
            project_id: project_id.clone(),
            tab: NewTab::Shell {
                command: String::new(),
            },
        }),
        CommandId::TabClose => close_selected_tab(),
        CommandId::TabRename => Ok(WorkspaceIntent::RenameTab {
            project_id: project_id.clone(),
            tab_id: tab_id.clone(),
            title: DEFAULT_RENAMED_TAB_TITLE.to_string(),
        }),
        CommandId::PaneSplitHorizontal => split(SplitDirection::Horizontal),
        CommandId::PaneSplitVertical => split(SplitDirection::Vertical),
        CommandId::PaneClose => {
            let tab = project
                .layout
                .tab(&tab_id)
                .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.clone()))?;
            // The last pane closes its tab.
            if tab.layout.pane_id().is_some() {
                close_selected_tab()
            } else {
                Ok(WorkspaceIntent::ClosePane {
                    project_id: project_id.clone(),
                    tab_id: tab_id.clone(),
                    pane_id: focused_pane()?,
                })
            }
        }
        CommandId::PaneRename => Ok(WorkspaceIntent::RenamePane {
            project_id: project_id.clone(),
            tab_id: tab_id.clone(),
            pane_id: focused_pane()?,
            title: DEFAULT_RENAMED_PANE_TITLE.to_string(),
        }),
        CommandId::PaneFocusLeft => Ok(focus(FocusDirection::Left)),
        CommandId::PaneFocusRight => Ok(focus(FocusDirection::Right)),
        CommandId::PaneFocusUp => Ok(focus(FocusDirection::Up)),
        CommandId::PaneFocusDown => Ok(focus(FocusDirection::Down)),
        CommandId::PaneResizeLeft => resize(ResizeDirection::Left),
        CommandId::PaneResizeRight => resize(ResizeDirection::Right),
        CommandId::PaneResizeUp => resize(ResizeDirection::Up),
        CommandId::PaneResizeDown => resize(ResizeDirection::Down),
        _ => Err(CommandDispatchError::Unsupported(command_id)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn observer(surface: ActiveSurface) -> CommandContext {
        CommandContext {
            has_selected_project: true,
            active_surface: surface,
            shared_editing_enabled: false,
            is_remote: true,
        }
    }

    #[test]
    fn observer_cannot_dispatch_shared_mutations() {
        for command in [
            CommandId::FileSave,
            CommandId::TabNew,
            CommandId::TabRename,
            CommandId::PaneSplitHorizontal,
            CommandId::PaneClose,
            CommandId::PaneResizeLeft,
            CommandId::PaneRename,
            CommandId::LayoutSaveCurrent,
            CommandId::LayoutExportProjectConfig,
        ] {
            assert!(
                !command
                    .availability_for_context(observer(ActiveSurface::Terminal))
                    .enabled,
                "{command:?} must require shared editing control",
            );
        }
        assert!(
            !CommandId::FileSave
                .availability_for_context(observer(ActiveSurface::File))
                .enabled
        );
    }

    #[test]
    fn observer_keeps_local_navigation_and_view_commands() {
        for command in [
            CommandId::CommandPaletteOpen,
            CommandId::ProjectPanelToggle,
            CommandId::ProjectPanelRefresh,
            CommandId::GitDiffOpen,
            CommandId::FileFind,
            CommandId::TabNext,
            CommandId::TabPrev,
            CommandId::TabPalette,
            CommandId::PaneFocusLeft,
            CommandId::PanePalette,
            CommandId::SettingsOpen,
            CommandId::SettingsKeybindings,
        ] {
            assert!(
                command
                    .availability_for_context(observer(ActiveSurface::Terminal))
                    .enabled,
                "{command:?} must remain available to observers",
            );
        }
        assert!(
            CommandId::TabClose
                .availability_for_context(observer(ActiveSurface::File))
                .enabled
        );
    }

    #[test]
    fn remote_sessions_cannot_reconnect_to_an_existing_host() {
        let context = CommandContext {
            has_selected_project: false,
            active_surface: ActiveSurface::None,
            shared_editing_enabled: true,
            is_remote: true,
        };

        assert!(
            !CommandId::ConnectExistingHost
                .availability_for_context(context)
                .enabled
        );
        let selected_remote = CommandContext {
            has_selected_project: true,
            ..context
        };
        assert!(
            !CommandId::LayoutOpenFile
                .availability_for_context(selected_remote)
                .enabled
        );
        assert!(
            CommandId::LayoutOpenFile
                .availability_for_context(CommandContext {
                    is_remote: false,
                    ..selected_remote
                })
                .enabled
        );
    }
}
