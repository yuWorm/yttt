use std::{
    borrow::Cow,
    collections::{HashMap, HashSet},
};

use gpui::{
    Action, KeyBinding, KeyBindingContextPredicate, KeybindingKeystroke, Keystroke, NoAction,
    SharedString, Unbind, actions,
};
use yttt_terminal::{
    CancelSearch as TerminalCancelSearch, Copy as TerminalCopy, Paste as TerminalPaste,
    SearchHistoryNext as TerminalSearchHistoryNext,
    SearchHistoryPrevious as TerminalSearchHistoryPrevious, SearchNext as TerminalSearchNext,
    SearchPrevious as TerminalSearchPrevious, SendBacktab as TerminalSendBacktab,
    SendTab as TerminalSendTab, StartHintMode as TerminalStartHintMode,
    StartSearch as TerminalStartSearch, TERMINAL_HINT_KEY_CONTEXT, TERMINAL_KEY_CONTEXT,
    TERMINAL_SEARCH_KEY_CONTEXT, TERMINAL_VI_KEY_CONTEXT, TerminalViCopySelection, TerminalViExit,
    TerminalViMotion, TerminalViMotionAction, TerminalViToggleSelection,
    ToggleViMode as TerminalToggleViMode,
};

use crate::{
    commands::{CommandId, CommandRegistry},
    config::{
        keybindings::{
            DEFAULT_KEYBINDING_CONTEXT, Keybinding, KeybindingsConfig, default_keybindings,
            load_keybindings, resolve_keybinding_sequence,
        },
        paths::AppConfigPaths,
    },
    ui::{
        editor::{
            EditorVimActionId, default_bindable_keybindings as default_editor_vim_keybindings,
        },
        vim::{
            VIM_CONTROL_CONTEXT, VIM_ESCAPE_CONTEXT, VIM_NORMAL_CONTEXT,
            VIM_PALETTE_NORMAL_CONTEXT, VIM_PROJECT_TREE_NORMAL_CONTEXT,
            VIM_PROJECTS_NORMAL_CONTEXT, VIM_SETTINGS_NORMAL_CONTEXT, VIM_TERMINAL_CONTEXT,
            VIM_TERMINAL_NORMAL_CONTEXT,
        },
    },
};

pub const WORKSPACE_CONTEXT: &str = "Workspace";
pub const PALETTE_CONTEXT: &str = "YtttPalette";
pub const PROJECT_TREE_CONTEXT: &str = "Tree";
pub const GIT_DIFF_CONTEXT: &str = "YtttGitDiff";
pub const WORKSPACE_VIM_CONTEXT: &str = "WorkspaceVim";

actions!(
    yttt,
    [
        OpenCommandPalette,
        OpenFileFinder,
        CreateProject,
        OpenProject,
        OpenSshProject,
        ProjectClose,
        OpenRecentProjectPalette,
        OpenProjectPalette,
        OpenOpenedProjectPalette,
        ProjectPanelToggle,
        ProjectPanelRefresh,
        GitBranchSwitch,
        GitDiffOpen,
        GitDiffClose,
        GitDiffToggleStageMode,
        GitDiffToggleViewMode,
        GitDiffToggleWhitespace,
        GitDiffSelectPreviousFile,
        GitDiffSelectNextFile,
        GitDiffCopySelected,
        FileSave,
        OpenTabPalette,
        OpenPanePalette,
        PaletteSelectNext,
        PaletteSelectPrev,
        PaletteConfirm,
        PaletteCancel,
        TabNew,
        TabClose,
        TabCloseAll,
        TabCloseBefore,
        TabCloseAfter,
        TabCloseAllFiles,
        TabCloseAllTerminals,
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
        SettingsVimPreviousGroup,
        SettingsVimNextGroup,
        SettingsVimFirstGroup,
        SettingsVimLastGroup,
        VimEnterNormal,
        VimEnterInsert,
        VimEnterTerminal,
        FocusProjects,
        ProjectsSelectPrevious,
        ProjectsSelectNext,
        ProjectsSelectFirst,
        ProjectsSelectLast,
        ProjectTreeSelectPrevious,
        ProjectTreeSelectNext,
        ProjectTreeCollapse,
        ProjectTreeExpand,
        ProjectTreeOpen,
        ProjectTreeToggle,
        ProjectTreeSelectFirst,
        ProjectTreeSelectLast,
        ProjectTreeCollapseAll,
        ProjectTreeToggleHidden,
        ProjectTreeNewFile,
        ProjectTreeNewDirectory,
        ProjectTreeRename,
        ProjectTreeDelete,
        ProjectTreeCopy,
        ProjectTreeCut,
        ProjectTreePaste,
    ]
);

macro_rules! define_bindable_actions {
    (
        commands {
            $( $command:path => $command_action:ident, )+
        }
        ui {
            $(
                $variant:ident => {
                    id: $id:literal,
                    action: $ui_action:expr,
                    title: $title:literal,
                    description: $description:literal,
                },
            )+
        }
    ) => {
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum BindableActionId {
            Command(CommandId),
            EditorVim(EditorVimActionId),
            $( $variant, )+
        }

        impl BindableActionId {
            pub fn all() -> impl Iterator<Item = Self> {
                CommandId::ALL
                    .iter()
                    .copied()
                    .map(Self::Command)
                    .chain([$(Self::$variant),+])
                    .chain(
                        EditorVimActionId::ALL
                            .iter()
                            .copied()
                            .map(Self::EditorVim),
                    )
            }

            pub fn from_str_id(id: &str) -> Option<Self> {
                if let Some(command) = CommandId::from_str_id(id) {
                    return Some(Self::Command(command));
                }
                match id {
                    $( $id => Some(Self::$variant), )+
                    _ => EditorVimActionId::from_str_id(id).map(Self::EditorVim),
                }
            }

            pub const fn as_str(self) -> &'static str {
                match self {
                    Self::Command(command) => command.as_str(),
                    Self::EditorVim(action) => action.as_str(),
                    $( Self::$variant => $id, )+
                }
            }

            pub const fn command(self) -> Option<CommandId> {
                match self {
                    Self::Command(command) => Some(command),
                    Self::EditorVim(_) => None,
                    $( Self::$variant => None, )+
                }
            }

            pub const fn title(self) -> Option<&'static str> {
                match self {
                    Self::Command(_) => None,
                    Self::EditorVim(action) => Some(action.title()),
                    $( Self::$variant => Some($title), )+
                }
            }

            pub const fn description(self) -> Option<&'static str> {
                match self {
                    Self::Command(_) => None,
                    Self::EditorVim(action) => Some(action.description()),
                    $( Self::$variant => Some($description), )+
                }
            }

            pub fn build(self) -> Box<dyn Action> {
                match self {
                    Self::Command(command) => match command {
                        $( $command => Box::new($command_action), )+
                    },
                    Self::EditorVim(action) => action.build(),
                    $( Self::$variant => Box::new($ui_action), )+
                }
            }

            pub fn key_binding(
                self,
                keys: &str,
                context: Option<&str>,
            ) -> KeyBinding {
                match self {
                    Self::Command(command) => match command {
                        $( $command => KeyBinding::new(keys, $command_action, context), )+
                    },
                    Self::EditorVim(action) => action.key_binding(keys, context),
                    $( Self::$variant => KeyBinding::new(keys, $ui_action, context), )+
                }
            }
        }

        impl PartialEq<CommandId> for BindableActionId {
            fn eq(&self, other: &CommandId) -> bool {
                self.command() == Some(*other)
            }
        }
    };
}

define_bindable_actions! {
    commands {
        CommandId::ProjectCreate => CreateProject,
        CommandId::ProjectOpen => OpenProject,
        CommandId::ProjectOpenSsh => OpenSshProject,
        CommandId::ProjectOpenRecent => OpenRecentProjectPalette,
        CommandId::ProjectClose => ProjectClose,
        CommandId::ProjectPalette => OpenProjectPalette,
        CommandId::ProjectOpenedPalette => OpenOpenedProjectPalette,
        CommandId::ProjectPanelToggle => ProjectPanelToggle,
        CommandId::ProjectPanelRefresh => ProjectPanelRefresh,
        CommandId::FileFind => OpenFileFinder,
        CommandId::FileSave => FileSave,
        CommandId::GitBranchSwitch => GitBranchSwitch,
        CommandId::GitDiffOpen => GitDiffOpen,
        CommandId::TabNew => TabNew,
        CommandId::TabClose => TabClose,
        CommandId::TabRename => TabRename,
        CommandId::TabNext => TabNext,
        CommandId::TabPrev => TabPrev,
        CommandId::TabPalette => OpenTabPalette,
        CommandId::PaneSplitHorizontal => PaneSplitHorizontal,
        CommandId::PaneSplitVertical => PaneSplitVertical,
        CommandId::PaneClose => PaneClose,
        CommandId::PaneFocusLeft => PaneFocusLeft,
        CommandId::PaneFocusRight => PaneFocusRight,
        CommandId::PaneFocusUp => PaneFocusUp,
        CommandId::PaneFocusDown => PaneFocusDown,
        CommandId::PaneResizeLeft => PaneResizeLeft,
        CommandId::PaneResizeRight => PaneResizeRight,
        CommandId::PaneResizeUp => PaneResizeUp,
        CommandId::PaneResizeDown => PaneResizeDown,
        CommandId::PaneRename => PaneRename,
        CommandId::PanePalette => OpenPanePalette,
        CommandId::LayoutDefaultEdit => LayoutDefaultEdit,
        CommandId::LayoutDefaultReset => LayoutDefaultReset,
        CommandId::LayoutDefaultReload => LayoutDefaultReload,
        CommandId::LayoutProjectEdit => LayoutProjectEdit,
        CommandId::LayoutSaveCurrent => LayoutSaveCurrent,
        CommandId::LayoutExportProjectConfig => LayoutExportProjectConfig,
        CommandId::LayoutResetLocalOverride => LayoutResetLocalOverride,
        CommandId::LayoutOpenFile => LayoutOpenFile,
        CommandId::CommandPaletteOpen => OpenCommandPalette,
        CommandId::SettingsOpen => SettingsOpen,
        CommandId::SettingsKeybindings => SettingsKeybindings,
        CommandId::SettingsNotifications => SettingsNotifications,
    }
    ui {
        PaletteNext => {
            id: "palette.select_next",
            action: PaletteSelectNext,
            title: "Select Next Palette Item",
            description: "Move selection to the next item in an open palette.",
        },
        PalettePrevious => {
            id: "palette.select_previous",
            action: PaletteSelectPrev,
            title: "Select Previous Palette Item",
            description: "Move selection to the previous item in an open palette.",
        },
        PaletteAccept => {
            id: "palette.confirm",
            action: PaletteConfirm,
            title: "Confirm Palette Selection",
            description: "Open the selected item in an active palette.",
        },
        PaletteDismiss => {
            id: "palette.cancel",
            action: PaletteCancel,
            title: "Close Palette",
            description: "Close the active palette without selecting an item.",
        },
        SettingsVimPrevious => {
            id: "settings.vim.previous_group",
            action: SettingsVimPreviousGroup,
            title: "Settings Vim: Previous Group",
            description: "Select the previous visible settings group.",
        },
        SettingsVimNext => {
            id: "settings.vim.next_group",
            action: SettingsVimNextGroup,
            title: "Settings Vim: Next Group",
            description: "Select the next visible settings group.",
        },
        SettingsVimFirst => {
            id: "settings.vim.first_group",
            action: SettingsVimFirstGroup,
            title: "Settings Vim: First Group",
            description: "Select the first visible settings group.",
        },
        SettingsVimLast => {
            id: "settings.vim.last_group",
            action: SettingsVimLastGroup,
            title: "Settings Vim: Last Group",
            description: "Select the last visible settings group.",
        },
        VimNormal => {
            id: "vim.mode.normal",
            action: VimEnterNormal,
            title: "Enter Vim Normal Mode",
            description: "Return the active yttt surface to Vim Normal mode.",
        },
        VimInsert => {
            id: "vim.mode.insert",
            action: VimEnterInsert,
            title: "Enter Vim Insert Mode",
            description: "Allow direct text input on the active yttt surface.",
        },
        VimTerminal => {
            id: "vim.mode.terminal",
            action: VimEnterTerminal,
            title: "Enter Vim Terminal Mode",
            description: "Return the active terminal to direct process input.",
        },
        ProjectsFocusList => {
            id: "projects.focus",
            action: FocusProjects,
            title: "Focus Projects List",
            description: "Move keyboard focus to the opened-projects list.",
        },
        ProjectsVimPrevious => {
            id: "projects.vim.previous",
            action: ProjectsSelectPrevious,
            title: "Projects Vim: Previous Project",
            description: "Select the previous opened project.",
        },
        ProjectsVimNext => {
            id: "projects.vim.next",
            action: ProjectsSelectNext,
            title: "Projects Vim: Next Project",
            description: "Select the next opened project.",
        },
        ProjectsVimFirst => {
            id: "projects.vim.first",
            action: ProjectsSelectFirst,
            title: "Projects Vim: First Project",
            description: "Select the first opened project.",
        },
        ProjectsVimLast => {
            id: "projects.vim.last",
            action: ProjectsSelectLast,
            title: "Projects Vim: Last Project",
            description: "Select the last opened project.",
        },
        ProjectTreeVimUp => {
            id: "project_tree.vim.up",
            action: ProjectTreeSelectPrevious,
            title: "Project Tree Vim: Previous Entry",
            description: "Move to the previous visible project-tree entry.",
        },
        ProjectTreeVimDown => {
            id: "project_tree.vim.down",
            action: ProjectTreeSelectNext,
            title: "Project Tree Vim: Next Entry",
            description: "Move to the next visible project-tree entry.",
        },
        ProjectTreeVimLeft => {
            id: "project_tree.vim.left",
            action: ProjectTreeCollapse,
            title: "Project Tree Vim: Collapse or Parent",
            description: "Collapse the current directory or select its parent.",
        },
        ProjectTreeVimRight => {
            id: "project_tree.vim.right",
            action: ProjectTreeExpand,
            title: "Project Tree Vim: Expand or Open",
            description: "Expand a directory, select its first child, or open a file.",
        },
        ProjectTreeVimOpen => {
            id: "project_tree.vim.open",
            action: ProjectTreeOpen,
            title: "Project Tree Vim: Open",
            description: "Open a file or toggle the selected directory.",
        },
        ProjectTreeVimToggle => {
            id: "project_tree.vim.toggle",
            action: ProjectTreeToggle,
            title: "Project Tree Vim: Toggle Directory",
            description: "Expand or collapse the selected project-tree directory.",
        },
        ProjectTreeVimFirst => {
            id: "project_tree.vim.first",
            action: ProjectTreeSelectFirst,
            title: "Project Tree Vim: First Entry",
            description: "Select the first visible project-tree entry.",
        },
        ProjectTreeVimLast => {
            id: "project_tree.vim.last",
            action: ProjectTreeSelectLast,
            title: "Project Tree Vim: Last Entry",
            description: "Select the last visible project-tree entry.",
        },
        ProjectTreeCollapseAllNodes => {
            id: "project_tree.collapse_all",
            action: ProjectTreeCollapseAll,
            title: "Collapse All Project Directories",
            description: "Collapse every expanded directory in the project tree.",
        },
        ProjectTreeToggleHiddenFiles => {
            id: "project_tree.toggle_hidden",
            action: ProjectTreeToggleHidden,
            title: "Toggle Hidden Project Files",
            description: "Show or hide dotfiles in the project tree.",
        },
        ProjectTreeCreateFile => {
            id: "project_tree.new_file",
            action: ProjectTreeNewFile,
            title: "New Project File",
            description: "Create a file in the selected project directory.",
        },
        ProjectTreeCreateDirectory => {
            id: "project_tree.new_directory",
            action: ProjectTreeNewDirectory,
            title: "New Project Directory",
            description: "Create a directory in the selected project directory.",
        },
        ProjectTreeRenameEntry => {
            id: "project_tree.rename",
            action: ProjectTreeRename,
            title: "Rename Project Entry",
            description: "Rename the selected project file or directory.",
        },
        ProjectTreeDeleteEntry => {
            id: "project_tree.delete",
            action: ProjectTreeDelete,
            title: "Delete Project Entry",
            description: "Delete the selected project file or directory.",
        },
        ProjectTreeCopyEntry => {
            id: "project_tree.copy",
            action: ProjectTreeCopy,
            title: "Copy Project Entry",
            description: "Copy the selected project file or directory.",
        },
        ProjectTreeCutEntry => {
            id: "project_tree.cut",
            action: ProjectTreeCut,
            title: "Cut Project Entry",
            description: "Cut the selected project file or directory.",
        },
        ProjectTreePasteEntry => {
            id: "project_tree.paste",
            action: ProjectTreePaste,
            title: "Paste Project Entry",
            description: "Paste a copied project entry into the selected directory.",
        },
        GitDiffDismiss => {
            id: "git.diff.close",
            action: GitDiffClose,
            title: "Close Git Diff",
            description: "Close the active Git diff panel.",
        },
        GitDiffStageMode => {
            id: "git.diff.toggle_stage_mode",
            action: GitDiffToggleStageMode,
            title: "Toggle Git Diff Stage",
            description: "Switch between staged and unstaged changes.",
        },
        GitDiffViewMode => {
            id: "git.diff.toggle_view_mode",
            action: GitDiffToggleViewMode,
            title: "Toggle Git Diff View",
            description: "Switch between unified and split diff views.",
        },
        GitDiffWhitespace => {
            id: "git.diff.toggle_whitespace",
            action: GitDiffToggleWhitespace,
            title: "Toggle Git Diff Whitespace",
            description: "Include or ignore whitespace changes.",
        },
        GitDiffPreviousFile => {
            id: "git.diff.select_previous_file",
            action: GitDiffSelectPreviousFile,
            title: "Previous Git Diff File",
            description: "Select the previous changed file.",
        },
        GitDiffNextFile => {
            id: "git.diff.select_next_file",
            action: GitDiffSelectNextFile,
            title: "Next Git Diff File",
            description: "Select the next changed file.",
        },
        GitDiffCopy => {
            id: "git.diff.copy_selected",
            action: GitDiffCopySelected,
            title: "Copy Git Diff",
            description: "Copy the selected file diff.",
        },
        TerminalTab => {
            id: "terminal.send_tab",
            action: TerminalSendTab,
            title: "Send Tab to Terminal",
            description: "Send a Tab key to the focused terminal.",
        },
        TerminalBacktab => {
            id: "terminal.send_backtab",
            action: TerminalSendBacktab,
            title: "Send Backtab to Terminal",
            description: "Send a reverse Tab key to the focused terminal.",
        },
        TerminalClipboardCopy => {
            id: "terminal.copy",
            action: TerminalCopy,
            title: "Copy from Terminal",
            description: "Copy the current terminal selection.",
        },
        TerminalClipboardPaste => {
            id: "terminal.paste",
            action: TerminalPaste,
            title: "Paste into Terminal",
            description: "Paste clipboard text into the focused terminal.",
        },
        TerminalSearchOpen => {
            id: "terminal.search.open",
            action: TerminalStartSearch,
            title: "Open Terminal Search",
            description: "Open search in the focused terminal.",
        },
        TerminalSearchForward => {
            id: "terminal.search.next",
            action: TerminalSearchNext,
            title: "Next Terminal Search Match",
            description: "Select the next terminal search match.",
        },
        TerminalSearchBackward => {
            id: "terminal.search.previous",
            action: TerminalSearchPrevious,
            title: "Previous Terminal Search Match",
            description: "Select the previous terminal search match.",
        },
        TerminalSearchHistoryUp => {
            id: "terminal.search.history_previous",
            action: TerminalSearchHistoryPrevious,
            title: "Previous Terminal Search",
            description: "Recall the previous terminal search query.",
        },
        TerminalSearchHistoryDown => {
            id: "terminal.search.history_next",
            action: TerminalSearchHistoryNext,
            title: "Next Terminal Search",
            description: "Recall the next terminal search query.",
        },
        TerminalSearchDismiss => {
            id: "terminal.search.cancel",
            action: TerminalCancelSearch,
            title: "Close Terminal Search or Hint",
            description: "Close terminal search or keyboard hint mode.",
        },
        TerminalViToggle => {
            id: "terminal.vi.toggle",
            action: TerminalToggleViMode,
            title: "Toggle Terminal Vi Mode",
            description: "Enter or leave terminal Vi mode.",
        },
        TerminalHintOpen => {
            id: "terminal.hint.open",
            action: TerminalStartHintMode,
            title: "Open Terminal Keyboard Hints",
            description: "Open keyboard-selectable links and text hints.",
        },
        TerminalViLeave => {
            id: "terminal.vi.exit",
            action: TerminalViExit,
            title: "Exit Terminal Vi Mode",
            description: "Return the terminal to normal input mode.",
        },
        TerminalViSelection => {
            id: "terminal.vi.toggle_selection",
            action: TerminalViToggleSelection,
            title: "Toggle Terminal Vi Selection",
            description: "Start or clear a terminal Vi selection.",
        },
        TerminalViYank => {
            id: "terminal.vi.copy_selection",
            action: TerminalViCopySelection,
            title: "Copy Terminal Vi Selection",
            description: "Copy and clear the terminal Vi selection.",
        },
        TerminalViMoveLeft => {
            id: "terminal.vi.motion.left",
            action: TerminalViMotionAction { motion: TerminalViMotion::Left },
            title: "Terminal Vi: Move Left",
            description: "Move the terminal Vi cursor left.",
        },
        TerminalViMoveDown => {
            id: "terminal.vi.motion.down",
            action: TerminalViMotionAction { motion: TerminalViMotion::Down },
            title: "Terminal Vi: Move Down",
            description: "Move the terminal Vi cursor down.",
        },
        TerminalViMoveUp => {
            id: "terminal.vi.motion.up",
            action: TerminalViMotionAction { motion: TerminalViMotion::Up },
            title: "Terminal Vi: Move Up",
            description: "Move the terminal Vi cursor up.",
        },
        TerminalViMoveRight => {
            id: "terminal.vi.motion.right",
            action: TerminalViMotionAction { motion: TerminalViMotion::Right },
            title: "Terminal Vi: Move Right",
            description: "Move the terminal Vi cursor right.",
        },
        TerminalViMoveFirst => {
            id: "terminal.vi.motion.first",
            action: TerminalViMotionAction { motion: TerminalViMotion::First },
            title: "Terminal Vi: First Column",
            description: "Move to the first terminal column.",
        },
        TerminalViMoveLast => {
            id: "terminal.vi.motion.last",
            action: TerminalViMotionAction { motion: TerminalViMotion::Last },
            title: "Terminal Vi: Last Column",
            description: "Move to the last terminal column.",
        },
        TerminalViMoveFirstOccupied => {
            id: "terminal.vi.motion.first_occupied",
            action: TerminalViMotionAction { motion: TerminalViMotion::FirstOccupied },
            title: "Terminal Vi: First Occupied Column",
            description: "Move to the first occupied terminal column.",
        },
        TerminalViMoveHigh => {
            id: "terminal.vi.motion.high",
            action: TerminalViMotionAction { motion: TerminalViMotion::High },
            title: "Terminal Vi: Viewport Top",
            description: "Move to the top of the terminal viewport.",
        },
        TerminalViMoveMiddle => {
            id: "terminal.vi.motion.middle",
            action: TerminalViMotionAction { motion: TerminalViMotion::Middle },
            title: "Terminal Vi: Viewport Middle",
            description: "Move to the middle of the terminal viewport.",
        },
        TerminalViMoveLow => {
            id: "terminal.vi.motion.low",
            action: TerminalViMotionAction { motion: TerminalViMotion::Low },
            title: "Terminal Vi: Viewport Bottom",
            description: "Move to the bottom of the terminal viewport.",
        },
        TerminalViMoveWordLeft => {
            id: "terminal.vi.motion.semantic_left",
            action: TerminalViMotionAction { motion: TerminalViMotion::SemanticLeft },
            title: "Terminal Vi: Previous Word",
            description: "Move to the previous terminal word.",
        },
        TerminalViMoveWordRight => {
            id: "terminal.vi.motion.semantic_right",
            action: TerminalViMotionAction { motion: TerminalViMotion::SemanticRight },
            title: "Terminal Vi: Next Word",
            description: "Move to the next terminal word.",
        },
        TerminalViMoveWordEnd => {
            id: "terminal.vi.motion.semantic_right_end",
            action: TerminalViMotionAction { motion: TerminalViMotion::SemanticRightEnd },
            title: "Terminal Vi: Next Word End",
            description: "Move to the end of the next terminal word.",
        },
        TerminalViMoveBracket => {
            id: "terminal.vi.motion.bracket",
            action: TerminalViMotionAction { motion: TerminalViMotion::Bracket },
            title: "Terminal Vi: Matching Bracket",
            description: "Move to the matching terminal bracket.",
        },
        TerminalViMoveParagraphUp => {
            id: "terminal.vi.motion.paragraph_up",
            action: TerminalViMotionAction { motion: TerminalViMotion::ParagraphUp },
            title: "Terminal Vi: Previous Paragraph",
            description: "Move to the previous terminal paragraph.",
        },
        TerminalViMoveParagraphDown => {
            id: "terminal.vi.motion.paragraph_down",
            action: TerminalViMotionAction { motion: TerminalViMotion::ParagraphDown },
            title: "Terminal Vi: Next Paragraph",
            description: "Move to the next terminal paragraph.",
        },
        CloseAllTabs => {
            id: "tab.close_all",
            action: TabCloseAll,
            title: "Close All Tabs",
            description: "Close every tab in the active pane.",
        },
        CloseTabsBefore => {
            id: "tab.close_before",
            action: TabCloseBefore,
            title: "Close Tabs Before",
            description: "Close tabs before the active tab.",
        },
        CloseTabsAfter => {
            id: "tab.close_after",
            action: TabCloseAfter,
            title: "Close Tabs After",
            description: "Close tabs after the active tab.",
        },
        CloseAllFileTabs => {
            id: "tab.close_all_files",
            action: TabCloseAllFiles,
            title: "Close All File Tabs",
            description: "Close every file tab.",
        },
        CloseAllTerminalTabs => {
            id: "tab.close_all_terminals",
            action: TabCloseAllTerminals,
            title: "Close All Terminal Tabs",
            description: "Close every terminal tab.",
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UiKeybindingSpec {
    pub keys: Cow<'static, str>,
    pub command: BindableActionId,
    pub context: Option<Cow<'static, str>>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CompiledKeybindingAction {
    Bind(BindableActionId),
    Unbind(Option<BindableActionId>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompiledKeybindingSpec {
    pub keys: String,
    pub context: Option<String>,
    pub action: CompiledKeybindingAction,
}

impl CompiledKeybindingSpec {
    pub fn to_gpui_keybinding(&self) -> KeyBinding {
        match self.action {
            CompiledKeybindingAction::Bind(action) => {
                action.key_binding(&self.keys, self.context.as_deref())
            }
            CompiledKeybindingAction::Unbind(Some(action)) => {
                let action_name = SharedString::from(action.build().name().to_string());
                KeyBinding::new(&self.keys, Unbind(action_name), self.context.as_deref())
            }
            CompiledKeybindingAction::Unbind(None) => {
                KeyBinding::new(&self.keys, NoAction, self.context.as_deref())
            }
        }
    }
}

pub fn bindable_registry() -> CommandRegistry {
    let mut registry = crate::commands::default_registry();
    for action in BindableActionId::all().filter(|action| action.command().is_none()) {
        registry.register_bindable_action(action.as_str());
    }
    registry
}

pub fn default_ui_keybinding_specs() -> Vec<UiKeybindingSpec> {
    layered_ui_keybinding_specs(&KeybindingsConfig::default(), &bindable_registry())
}

pub fn default_bindings_for_action(action: BindableActionId) -> Vec<UiKeybindingSpec> {
    let config = KeybindingsConfig::default();
    default_bindings_for_action_with_leader(action, &config.leader)
}

pub fn default_bindings_for_action_with_leader(
    action: BindableActionId,
    leader: &str,
) -> Vec<UiKeybindingSpec> {
    let config = KeybindingsConfig {
        leader: leader.to_string(),
        ..KeybindingsConfig::default()
    };
    layered_ui_keybinding_specs(&config, &bindable_registry())
        .into_iter()
        .filter(|binding| binding.command == action)
        .collect()
}

pub fn preferred_context_for_action(action: BindableActionId) -> Option<String> {
    default_bindings_for_action(action)
        .into_iter()
        .find_map(|binding| binding.context.map(Cow::into_owned))
        .or_else(|| Some(DEFAULT_KEYBINDING_CONTEXT.to_string()))
}

pub fn app_startup_keybindings() -> Vec<KeyBinding> {
    compiled_app_keybindings(&KeybindingsConfig::default(), &bindable_registry())
}

pub fn load_app_keybindings(paths: &AppConfigPaths, registry: &CommandRegistry) -> Vec<KeyBinding> {
    let config = load_keybindings(paths, registry)
        .ok()
        .filter(|loaded| loaded.warnings.is_empty())
        .map(|loaded| loaded.config)
        .unwrap_or_default();
    compiled_app_keybindings(&config, registry)
}

pub fn compiled_app_keybindings(
    config: &KeybindingsConfig,
    registry: &CommandRegistry,
) -> Vec<KeyBinding> {
    compile_layered_keybinding_specs(config, registry)
        .into_iter()
        .map(|binding| binding.to_gpui_keybinding())
        .collect()
}

pub fn compile_layered_keybinding_specs(
    config: &KeybindingsConfig,
    registry: &CommandRegistry,
) -> Vec<CompiledKeybindingSpec> {
    let defaults = default_keybindings();
    let supplemental = default_contextual_keybindings();
    compile_keybinding_layers(
        defaults
            .bindings
            .iter()
            .chain(supplemental.iter())
            .chain(config.bindings.iter()),
        registry,
        &config.leader,
    )
}

fn compile_keybinding_spec(
    binding: &Keybinding,
    leader: &str,
    registry: &CommandRegistry,
) -> Option<CompiledKeybindingSpec> {
    let keys = resolve_keybinding_sequence(&binding.keys, leader);
    if keys.is_empty()
        || keys
            .split_whitespace()
            .any(|keystroke| Keystroke::parse(keystroke).is_err())
    {
        return None;
    }
    let context = normalize_context(binding.context.as_deref());
    if context
        .as_deref()
        .is_some_and(|context| KeyBindingContextPredicate::parse(context).is_err())
    {
        return None;
    }

    let command = binding.command.trim();
    let action = if command.is_empty() {
        None
    } else {
        let action = BindableActionId::from_str_id(command)?;
        registry.contains_str(action.as_str()).then_some(action)
    };
    let action = if binding.unbind {
        CompiledKeybindingAction::Unbind(action)
    } else {
        CompiledKeybindingAction::Bind(action?)
    };
    Some(CompiledKeybindingSpec {
        keys,
        context,
        action,
    })
}

fn compile_keybinding_layers<'a>(
    bindings: impl IntoIterator<Item = &'a Keybinding>,
    registry: &CommandRegistry,
    leader: &str,
) -> Vec<CompiledKeybindingSpec> {
    let mut effective = Vec::<Option<CompiledKeybindingSpec>>::new();
    let mut by_key =
        HashMap::<(Option<String>, String), HashMap<Option<BindableActionId>, usize>>::new();

    for binding in bindings {
        let Some(binding) = compile_keybinding_spec(binding, leader, registry) else {
            continue;
        };
        let assignments = by_key
            .entry((binding.context.clone(), binding.keys.clone()))
            .or_default();
        match binding.action {
            CompiledKeybindingAction::Bind(action) => {
                assignments.retain(|_, previous| {
                    let keep = !matches!(
                        effective[*previous].as_ref().map(|binding| binding.action),
                        Some(CompiledKeybindingAction::Bind(_))
                    );
                    if !keep {
                        effective[*previous] = None;
                    }
                    keep
                });
                if let Some(previous) = assignments.remove(&Some(action)) {
                    effective[previous] = None;
                }
                assignments.insert(Some(action), effective.len());
            }
            CompiledKeybindingAction::Unbind(Some(action)) => {
                if let Some(previous) = assignments.insert(Some(action), effective.len()) {
                    effective[previous] = None;
                }
            }
            CompiledKeybindingAction::Unbind(None) => {
                for (_, previous) in assignments.drain() {
                    effective[previous] = None;
                }
                assignments.insert(None, effective.len());
            }
        }
        effective.push(Some(binding));
    }

    effective.into_iter().flatten().collect()
}

pub fn layered_ui_keybinding_specs(
    config: &KeybindingsConfig,
    registry: &CommandRegistry,
) -> Vec<UiKeybindingSpec> {
    compile_layered_keybinding_specs(config, registry)
        .into_iter()
        .filter_map(|binding| {
            let CompiledKeybindingAction::Bind(action) = binding.action else {
                return None;
            };
            Some(UiKeybindingSpec {
                keys: Cow::Owned(binding.keys),
                command: action,
                context: binding.context.map(Cow::Owned),
            })
        })
        .collect()
}

fn compile_assigned_keybinding_specs<'a>(
    bindings: impl IntoIterator<Item = &'a Keybinding>,
    registry: &CommandRegistry,
    leader: &str,
) -> Vec<UiKeybindingSpec> {
    let mut effective = Vec::<Option<UiKeybindingSpec>>::new();
    let mut by_key = HashMap::<(Option<String>, String), HashMap<BindableActionId, usize>>::new();

    for binding in bindings {
        let Some(binding) = compile_keybinding_spec(binding, leader, registry) else {
            continue;
        };
        let CompiledKeybindingSpec {
            keys,
            context,
            action,
        } = binding;
        let assignments = by_key.entry((context.clone(), keys.clone())).or_default();
        match action {
            CompiledKeybindingAction::Bind(action) => {
                if let Some(previous) = assignments.insert(action, effective.len()) {
                    effective[previous] = None;
                }
                effective.push(Some(UiKeybindingSpec {
                    keys: Cow::Owned(keys),
                    command: action,
                    context: context.map(Cow::Owned),
                }));
            }
            CompiledKeybindingAction::Unbind(Some(action)) => {
                if let Some(previous) = assignments.remove(&action) {
                    effective[previous] = None;
                }
            }
            CompiledKeybindingAction::Unbind(None) => {
                for (_, previous) in assignments.drain() {
                    effective[previous] = None;
                }
            }
        }
    }

    effective.into_iter().flatten().collect()
}

pub fn assigned_ui_keybinding_specs(
    config: &KeybindingsConfig,
    registry: &CommandRegistry,
) -> Vec<UiKeybindingSpec> {
    let defaults = default_keybindings();
    let supplemental = default_contextual_keybindings();
    compile_assigned_keybinding_specs(
        defaults
            .bindings
            .iter()
            .chain(supplemental.iter())
            .chain(config.bindings.iter()),
        registry,
        &config.leader,
    )
}

pub fn ui_keybinding_specs_from_config(
    config: &KeybindingsConfig,
    registry: &CommandRegistry,
) -> Vec<UiKeybindingSpec> {
    let conflicting_keys: HashSet<_> = config
        .conflicts()
        .into_iter()
        .map(|conflict| (conflict.context, conflict.keys))
        .collect();

    config
        .bindings
        .iter()
        .filter(|binding| !binding.unbind)
        .filter(|binding| {
            !conflicting_keys.contains(&(
                normalize_context(binding.context.as_deref()),
                normalize_keys(&binding.keys),
            ))
        })
        .filter_map(|binding| {
            let action = BindableActionId::from_str_id(&binding.command)?;
            if !registry.contains_str(binding.command.as_str()) {
                return None;
            }
            Some(UiKeybindingSpec {
                keys: Cow::Owned(normalize_keys(&binding.keys)),
                command: action,
                context: normalize_context(binding.context.as_deref()).map(Cow::Owned),
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
        .rev()
        .filter(|spec| {
            matches!(
                spec.context.as_deref(),
                None | Some(DEFAULT_KEYBINDING_CONTEXT)
            )
        })
        .find(|spec| {
            Keystroke::parse(spec.keys.as_ref())
                .map(KeybindingKeystroke::from_keystroke)
                .map(|target| keystroke.should_match(&target))
                .unwrap_or(false)
        })
        .and_then(|spec| spec.command.command())
}

pub fn ui_action_for_command(command: CommandId) -> Option<Box<dyn Action>> {
    Some(BindableActionId::Command(command).build())
}

pub fn ui_action_for_id(id: &str) -> Option<Box<dyn Action>> {
    Some(BindableActionId::from_str_id(id)?.build())
}

fn default_contextual_keybindings() -> Vec<Keybinding> {
    let mut bindings = vec![
        contextual_binding("down", BindableActionId::PaletteNext, PALETTE_CONTEXT),
        contextual_binding("up", BindableActionId::PalettePrevious, PALETTE_CONTEXT),
        contextual_binding("enter", BindableActionId::PaletteAccept, PALETTE_CONTEXT),
        contextual_binding("escape", BindableActionId::PaletteDismiss, PALETTE_CONTEXT),
        contextual_binding(
            "j",
            BindableActionId::PaletteNext,
            VIM_PALETTE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "k",
            BindableActionId::PalettePrevious,
            VIM_PALETTE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "cmd-n",
            BindableActionId::ProjectTreeCreateFile,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "cmd-shift-n",
            BindableActionId::ProjectTreeCreateDirectory,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "cmd-backspace",
            BindableActionId::ProjectTreeDeleteEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "delete",
            BindableActionId::ProjectTreeDeleteEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "cmd-c",
            BindableActionId::ProjectTreeCopyEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "cmd-x",
            BindableActionId::ProjectTreeCutEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "cmd-v",
            BindableActionId::ProjectTreePasteEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "ctrl-n",
            BindableActionId::ProjectTreeCreateFile,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "ctrl-shift-n",
            BindableActionId::ProjectTreeCreateDirectory,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "f2",
            BindableActionId::ProjectTreeRenameEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "ctrl-c",
            BindableActionId::ProjectTreeCopyEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "ctrl-x",
            BindableActionId::ProjectTreeCutEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding(
            "ctrl-v",
            BindableActionId::ProjectTreePasteEntry,
            PROJECT_TREE_CONTEXT,
        ),
        contextual_binding("escape", BindableActionId::GitDiffDismiss, GIT_DIFF_CONTEXT),
        contextual_binding("tab", BindableActionId::GitDiffStageMode, GIT_DIFF_CONTEXT),
        contextual_binding("s", BindableActionId::GitDiffViewMode, GIT_DIFF_CONTEXT),
        contextual_binding("w", BindableActionId::GitDiffWhitespace, GIT_DIFF_CONTEXT),
        contextual_binding(
            "up",
            BindableActionId::GitDiffPreviousFile,
            GIT_DIFF_CONTEXT,
        ),
        contextual_binding("down", BindableActionId::GitDiffNextFile, GIT_DIFF_CONTEXT),
        contextual_binding("cmd-c", BindableActionId::GitDiffCopy, GIT_DIFF_CONTEXT),
        contextual_binding("ctrl-c", BindableActionId::GitDiffCopy, GIT_DIFF_CONTEXT),
        contextual_binding("tab", BindableActionId::TerminalTab, TERMINAL_KEY_CONTEXT),
        contextual_binding(
            "shift-tab",
            BindableActionId::TerminalBacktab,
            TERMINAL_KEY_CONTEXT,
        ),
        contextual_binding(
            "enter",
            BindableActionId::TerminalSearchForward,
            TERMINAL_SEARCH_KEY_CONTEXT,
        ),
        contextual_binding(
            "shift-enter",
            BindableActionId::TerminalSearchBackward,
            TERMINAL_SEARCH_KEY_CONTEXT,
        ),
        contextual_binding(
            "up",
            BindableActionId::TerminalSearchHistoryUp,
            TERMINAL_SEARCH_KEY_CONTEXT,
        ),
        contextual_binding(
            "down",
            BindableActionId::TerminalSearchHistoryDown,
            TERMINAL_SEARCH_KEY_CONTEXT,
        ),
        contextual_binding(
            "escape",
            BindableActionId::TerminalSearchDismiss,
            TERMINAL_SEARCH_KEY_CONTEXT,
        ),
        contextual_binding(
            "escape",
            BindableActionId::TerminalSearchDismiss,
            TERMINAL_HINT_KEY_CONTEXT,
        ),
        contextual_binding(
            "v",
            BindableActionId::TerminalViSelection,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "y",
            BindableActionId::TerminalViYank,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "h",
            BindableActionId::TerminalViMoveLeft,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "left",
            BindableActionId::TerminalViMoveLeft,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "j",
            BindableActionId::TerminalViMoveDown,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "down",
            BindableActionId::TerminalViMoveDown,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "k",
            BindableActionId::TerminalViMoveUp,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "up",
            BindableActionId::TerminalViMoveUp,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "l",
            BindableActionId::TerminalViMoveRight,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "right",
            BindableActionId::TerminalViMoveRight,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "0",
            BindableActionId::TerminalViMoveFirst,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "$",
            BindableActionId::TerminalViMoveLast,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "^",
            BindableActionId::TerminalViMoveFirstOccupied,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "shift-h",
            BindableActionId::TerminalViMoveHigh,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "shift-m",
            BindableActionId::TerminalViMoveMiddle,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "shift-l",
            BindableActionId::TerminalViMoveLow,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "b",
            BindableActionId::TerminalViMoveWordLeft,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "w",
            BindableActionId::TerminalViMoveWordRight,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "e",
            BindableActionId::TerminalViMoveWordEnd,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "%",
            BindableActionId::TerminalViMoveBracket,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "{",
            BindableActionId::TerminalViMoveParagraphUp,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding(
            "}",
            BindableActionId::TerminalViMoveParagraphDown,
            TERMINAL_VI_KEY_CONTEXT,
        ),
        contextual_binding("escape", BindableActionId::VimNormal, VIM_ESCAPE_CONTEXT),
        contextual_binding("ctrl-[", BindableActionId::VimNormal, VIM_ESCAPE_CONTEXT),
        contextual_binding(
            "ctrl-w h",
            BindableActionId::Command(CommandId::PaneFocusLeft),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "ctrl-w j",
            BindableActionId::Command(CommandId::PaneFocusDown),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "ctrl-w k",
            BindableActionId::Command(CommandId::PaneFocusUp),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "ctrl-w l",
            BindableActionId::Command(CommandId::PaneFocusRight),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "ctrl-w ctrl-h",
            BindableActionId::Command(CommandId::PaneFocusLeft),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "ctrl-w ctrl-j",
            BindableActionId::Command(CommandId::PaneFocusDown),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "ctrl-w ctrl-k",
            BindableActionId::Command(CommandId::PaneFocusUp),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "ctrl-w ctrl-l",
            BindableActionId::Command(CommandId::PaneFocusRight),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "g t",
            BindableActionId::Command(CommandId::TabNext),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "g shift-t",
            BindableActionId::Command(CommandId::TabPrev),
            VIM_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "<leader> f f",
            BindableActionId::Command(CommandId::FileFind),
            VIM_CONTROL_CONTEXT,
        ),
        contextual_binding(
            "<leader> p",
            BindableActionId::Command(CommandId::CommandPaletteOpen),
            VIM_CONTROL_CONTEXT,
        ),
        contextual_binding(
            "<leader> b",
            BindableActionId::Command(CommandId::TabPalette),
            VIM_CONTROL_CONTEXT,
        ),
        contextual_binding(
            "ctrl-\\ ctrl-n",
            BindableActionId::VimNormal,
            VIM_TERMINAL_CONTEXT,
        ),
        contextual_binding(
            "i",
            BindableActionId::VimTerminal,
            VIM_TERMINAL_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "a",
            BindableActionId::VimTerminal,
            VIM_TERMINAL_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "shift-i",
            BindableActionId::VimTerminal,
            VIM_TERMINAL_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "shift-a",
            BindableActionId::VimTerminal,
            VIM_TERMINAL_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "k",
            BindableActionId::ProjectsVimPrevious,
            VIM_PROJECTS_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "j",
            BindableActionId::ProjectsVimNext,
            VIM_PROJECTS_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "g g",
            BindableActionId::ProjectsVimFirst,
            VIM_PROJECTS_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "shift-g",
            BindableActionId::ProjectsVimLast,
            VIM_PROJECTS_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "k",
            BindableActionId::ProjectTreeVimUp,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "j",
            BindableActionId::ProjectTreeVimDown,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "h",
            BindableActionId::ProjectTreeVimLeft,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "l",
            BindableActionId::ProjectTreeVimRight,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "enter",
            BindableActionId::ProjectTreeVimOpen,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "o",
            BindableActionId::ProjectTreeVimOpen,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "g g",
            BindableActionId::ProjectTreeVimFirst,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "shift-g",
            BindableActionId::ProjectTreeVimLast,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "z",
            BindableActionId::ProjectTreeCollapseAllNodes,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "a",
            BindableActionId::ProjectTreeCreateFile,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "shift-a",
            BindableActionId::ProjectTreeCreateDirectory,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "r",
            BindableActionId::ProjectTreeRenameEntry,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "d",
            BindableActionId::ProjectTreeDeleteEntry,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "y",
            BindableActionId::ProjectTreeCopyEntry,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "x",
            BindableActionId::ProjectTreeCutEntry,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "p",
            BindableActionId::ProjectTreePasteEntry,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "shift-r",
            BindableActionId::Command(CommandId::ProjectPanelRefresh),
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "shift-h",
            BindableActionId::ProjectTreeToggleHiddenFiles,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "q",
            BindableActionId::Command(CommandId::ProjectPanelToggle),
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "/",
            BindableActionId::Command(CommandId::FileFind),
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "i",
            BindableActionId::VimInsert,
            VIM_PROJECT_TREE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "k",
            BindableActionId::SettingsVimPrevious,
            VIM_SETTINGS_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "j",
            BindableActionId::SettingsVimNext,
            VIM_SETTINGS_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "g g",
            BindableActionId::SettingsVimFirst,
            VIM_SETTINGS_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "shift-g",
            BindableActionId::SettingsVimLast,
            VIM_SETTINGS_NORMAL_CONTEXT,
        ),
        contextual_binding("i", BindableActionId::VimInsert, VIM_PALETTE_NORMAL_CONTEXT),
        contextual_binding(
            "escape",
            BindableActionId::PaletteDismiss,
            VIM_PALETTE_NORMAL_CONTEXT,
        ),
        contextual_binding(
            "i",
            BindableActionId::VimInsert,
            VIM_SETTINGS_NORMAL_CONTEXT,
        ),
    ];
    #[cfg(target_os = "macos")]
    bindings.extend([
        contextual_binding(
            "cmd-c",
            BindableActionId::TerminalClipboardCopy,
            TERMINAL_KEY_CONTEXT,
        ),
        contextual_binding(
            "cmd-v",
            BindableActionId::TerminalClipboardPaste,
            TERMINAL_KEY_CONTEXT,
        ),
        contextual_binding(
            "cmd-f",
            BindableActionId::TerminalSearchOpen,
            TERMINAL_KEY_CONTEXT,
        ),
        contextual_binding(
            "cmd-shift-o",
            BindableActionId::TerminalHintOpen,
            TERMINAL_KEY_CONTEXT,
        ),
    ]);
    #[cfg(not(target_os = "macos"))]
    bindings.extend([
        contextual_binding(
            "ctrl-shift-c",
            BindableActionId::TerminalClipboardCopy,
            TERMINAL_KEY_CONTEXT,
        ),
        contextual_binding(
            "ctrl-shift-v",
            BindableActionId::TerminalClipboardPaste,
            TERMINAL_KEY_CONTEXT,
        ),
        contextual_binding(
            "ctrl-shift-f",
            BindableActionId::TerminalSearchOpen,
            TERMINAL_KEY_CONTEXT,
        ),
        contextual_binding(
            "ctrl-shift-o",
            BindableActionId::TerminalHintOpen,
            TERMINAL_KEY_CONTEXT,
        ),
    ]);
    bindings.extend(default_editor_vim_keybindings().into_iter().map(|binding| {
        contextual_binding(
            binding.keys,
            BindableActionId::EditorVim(binding.action),
            binding.context,
        )
    }));
    bindings
}

fn contextual_binding(keys: &str, action: BindableActionId, context: &str) -> Keybinding {
    Keybinding {
        keys: keys.to_string(),
        command: action.as_str().to_string(),
        context: Some(context.to_string()),
        unbind: false,
    }
}

fn normalize_keys(keys: &str) -> String {
    keys.trim().to_ascii_lowercase()
}

fn normalize_context(context: Option<&str>) -> Option<String> {
    context
        .map(str::trim)
        .filter(|context| !context.is_empty())
        .map(str::to_string)
}
