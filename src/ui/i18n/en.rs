use super::UiTextKey;

pub(super) fn text(key: UiTextKey) -> &'static str {
    match key {
        UiTextKey::AppName => "yttt",
        UiTextKey::OnboardingTitle => "Welcome to yttt",
        UiTextKey::OnboardingSubtitle => {
            "Choose a language, terminal font, default layout, and coding agent, then optionally import themes from Zed."
        }
        UiTextKey::OnboardingLanguageHeading => "Choose your language",
        UiTextKey::OnboardingLanguageSubtitle => {
            "The detected default is saved now. You can change it here or later in Settings."
        }
        UiTextKey::OnboardingFontHeading => "Choose a terminal font",
        UiTextKey::OnboardingFontSubtitle => {
            "Select an installed font for every terminal. You can change it later in Settings."
        }
        UiTextKey::OnboardingFontRecommendation => {
            "Recommended: use a monospaced Nerd Font. It includes terminal icons and symbols that many system default fonts lack, preventing boxes or garbled glyphs."
        }
        UiTextKey::OnboardingAgentHeading => "Choose a coding agent",
        UiTextKey::OnboardingAgentSubtitle => "This agent will be used in the layout you selected.",
        UiTextKey::OnboardingLayoutHeading => "Choose a default layout",
        UiTextKey::OnboardingSplitLayoutTitle => "Split view",
        UiTextKey::OnboardingSplitLayoutDescription => "One tab with Agent and Shell side by side.",
        UiTextKey::OnboardingTabsLayoutTitle => "Separate tabs",
        UiTextKey::OnboardingTabsLayoutDescription => "Agent and Shell each open in their own tab.",
        UiTextKey::OnboardingAgentPane => "Agent",
        UiTextKey::OnboardingShellPane => "Shell",
        UiTextKey::OnboardingCommandPaletteHint => "Open the Command Palette anytime",
        UiTextKey::OnboardingNext => "Next",
        UiTextKey::OnboardingBack => "Back",
        UiTextKey::OnboardingContinue => "Use this layout",
        UiTextKey::OnboardingZedHeading => "Import themes from Zed",
        UiTextKey::OnboardingZedSubtitle => {
            "Compatible themes were detected in your installed Zed extensions. Review them before importing."
        }
        UiTextKey::OnboardingZedUiThemes => "UI themes",
        UiTextKey::OnboardingZedIconThemes => "Icon themes",
        UiTextKey::OnboardingZedNoThemes => "No compatible Zed themes were detected.",
        UiTextKey::OnboardingZedDetectionWarnings => "Some Zed extensions could not be inspected.",
        UiTextKey::OnboardingZedSkip => "Skip import",
        UiTextKey::OnboardingZedImport => "Import and finish",
        UiTextKey::EmptySubtitle => "Open a directory or choose a recent project.",
        UiTextKey::EmptySidebarNote => "Sidebar shows opened projects only.",
        UiTextKey::CreateProjectDirectoryError => "Failed to create project directory",
        UiTextKey::CreateProjectPickerError => "Failed to choose a project location",
        UiTextKey::OpenDirectory => "Open Directory",
        UiTextKey::OpenRecent => "Open Recent",
        UiTextKey::RestoreLastSession => "Restore Last Session",
        UiTextKey::CommandPalette => "Command Palette",
        UiTextKey::NewTab => "New Tab",
        UiTextKey::NoTerminalTabs => "No terminal tabs",
        UiTextKey::Projects => "Projects",
        UiTextKey::ProjectFiles => "Files",
        UiTextKey::ProjectFilesShow => "Show Files",
        UiTextKey::ProjectFilesHide => "Hide Files",
        UiTextKey::ProjectFilesRefresh => "Refresh",
        UiTextKey::ProjectFilesLoading => "Loading project files…",
        UiTextKey::ProjectFilesEmptyDirectory => "Empty directory",
        UiTextKey::ProjectFilesDirectoryError => "Unable to load directory",
        UiTextKey::ProjectFilesRetry => "Retry",
        UiTextKey::ProjectFilesNewFile => "New File",
        UiTextKey::ProjectFilesNewDirectory => "New Folder",
        UiTextKey::ProjectFilesCreateProjectLayout => "Create Project Layout",
        UiTextKey::ProjectFilesProjectLayoutReady => "Project layout is ready",
        UiTextKey::ProjectFilesRename => "Rename",
        UiTextKey::ProjectFilesDelete => "Delete",
        UiTextKey::ProjectFilesCopy => "Copy",
        UiTextKey::ProjectFilesCut => "Cut",
        UiTextKey::ProjectFilesPaste => "Paste",
        UiTextKey::ProjectFilesShowHidden => "Show Hidden Files",
        UiTextKey::ProjectFilesHideHidden => "Hide Hidden Files",
        UiTextKey::ProjectFilesEntryPlaceholder => "name or path; end with / for a folder",
        UiTextKey::ProjectFilesDeleteConfirmTitle => "Delete project entry?",
        UiTextKey::ProjectFilesDeleteConfirmMessage => {
            "The selected file or folder will be permanently deleted. This cannot be undone."
        }
        UiTextKey::ProjectFileUnsupportedBinary => "Binary files are not supported",
        UiTextKey::ProjectFileInvalidEncoding => "Only UTF-8 files are supported",
        UiTextKey::ProjectFileTooLarge => "File exceeds the 10 MiB editor limit",
        UiTextKey::ProjectFileOutsideProject => "File is outside the project",
        UiTextKey::FileSaveAction => "Save",
        UiTextKey::FileSaving => "Saving…",
        UiTextKey::FileSaved => "Saved",
        UiTextKey::FileSaveFailed => "Save failed",
        UiTextKey::FileChangedOnDisk => "File changed on disk",
        UiTextKey::FileDeletedOnDisk => "File was deleted on disk",
        UiTextKey::FileOverwrite => "Overwrite",
        UiTextKey::FileReload => "Reload",
        UiTextKey::FileRecreate => "Recreate file",
        UiTextKey::MarkdownShowRendered => "Show rendered Markdown",
        UiTextKey::MarkdownShowSource => "Edit Markdown source",
        UiTextKey::Tabs => "Tabs",
        UiTextKey::TabCloseCurrent => "Close Current",
        UiTextKey::TabCloseAll => "Close All",
        UiTextKey::TabCloseBefore => "Close Tabs to the Left",
        UiTextKey::TabCloseAfter => "Close Tabs to the Right",
        UiTextKey::TabCloseAllFiles => "Close All Files",
        UiTextKey::TabCloseAllTerminals => "Close All Terminals",
        UiTextKey::Lazy => "lazy",
        UiTextKey::Started => "started",
        UiTextKey::Active => "active",
        UiTextKey::NoResults => "No results",
        UiTextKey::TypeToFilter => "Type to filter",
        UiTextKey::PalettePlaceholderCommand => "Execute a command...",
        UiTextKey::PalettePlaceholderNewTabCommand => "Choose a command for the new tab...",
        UiTextKey::PalettePlaceholderProject => "Switch project...",
        UiTextKey::PalettePlaceholderOpenedProject => "Switch opened project...",
        UiTextKey::PalettePlaceholderRecentProject => "Open recent project...",
        UiTextKey::PalettePlaceholderTab => "Switch tab...",
        UiTextKey::PalettePlaceholderPane => "Switch pane...",
        UiTextKey::PalettePlaceholderGitBranch => "Switch Git branch...",
        UiTextKey::PaletteRun => "Run",
        UiTextKey::PaletteClose => "Close",
        UiTextKey::PaletteStatusOpen => "open",
        UiTextKey::PaletteStatusRecent => "recent",
        UiTextKey::PaletteStatusPaneSingular => "pane",
        UiTextKey::PaletteStatusPanePlural => "panes",
        UiTextKey::PaletteStatusActive => "active",
        UiTextKey::PaletteStatusLazy => "lazy",
        UiTextKey::PaletteStatusStarted => "started",
        UiTextKey::PaletteStatusIdle => "idle",
        UiTextKey::PaletteStatusRunning => "running",
        UiTextKey::PaletteStatusExited => "exited",
        UiTextKey::PaletteStatusAgent => "agent",
        UiTextKey::PaletteStatusAgentRunning => "agent running",
        UiTextKey::PaletteStatusAgentCompleted => "agent completed",
        UiTextKey::PaletteStatusAgentFailed => "agent failed",
        UiTextKey::CommandDisabledVisibleProjectActions => "Use the visible project actions",
        UiTextKey::CommandDisabledOpenProjectFirst => "Open a project first",
        UiTextKey::CommandDisabledFocusProjectFileFirst => "Focus a project file first",
        UiTextKey::CommandDisabledOpenWorkItemFirst => "Open a terminal or file first",
        UiTextKey::CommandDisabledSwitchTerminalFirst => "Switch to a terminal tab first",
        UiTextKey::CommandUnavailable => "Command is unavailable",
        UiTextKey::CommandProjectCreateTitle => "Create Project",
        UiTextKey::CommandProjectCreateDescription => "Create and open a new project directory",
        UiTextKey::CommandProjectOpenTitle => "Open Project",
        UiTextKey::CommandProjectOpenDescription => "Choose a project directory",
        UiTextKey::CommandProjectOpenRecentTitle => "Open Recent Project",
        UiTextKey::CommandProjectOpenRecentDescription => "Choose a recent project",
        UiTextKey::CommandProjectCloseTitle => "Close Project",
        UiTextKey::CommandProjectCloseDescription => "Close the selected project",
        UiTextKey::CommandProjectPaletteTitle => "Open Project Palette",
        UiTextKey::CommandProjectPaletteDescription => "Switch opened or recent projects",
        UiTextKey::CommandProjectOpenedPaletteTitle => "Switch Opened Project",
        UiTextKey::CommandProjectOpenedPaletteDescription => {
            "Choose from currently opened projects"
        }
        UiTextKey::CommandProjectPanelToggleTitle => "Toggle Project Files",
        UiTextKey::CommandProjectPanelToggleDescription => "Show or hide the project file tree",
        UiTextKey::CommandProjectPanelRefreshTitle => "Refresh Project Files",
        UiTextKey::CommandProjectPanelRefreshDescription => "Refresh the project file tree",
        UiTextKey::CommandGitBranchSwitchTitle => "Switch Git Branch",
        UiTextKey::CommandGitBranchSwitchDescription => "Choose and check out a Git branch",
        UiTextKey::CommandGitDiffOpenTitle => "Show Git Changes",
        UiTextKey::CommandGitDiffOpenDescription => "Open the selected project's working tree diff",
        UiTextKey::CommandFileSaveTitle => "Save File",
        UiTextKey::CommandFileSaveDescription => "Save the active project file",
        UiTextKey::CommandTabNewTitle => "New Tab",
        UiTextKey::CommandTabNewDescription => "Create a shell tab in the selected project",
        UiTextKey::CommandTabCloseTitle => "Close Tab",
        UiTextKey::CommandTabCloseDescription => "Close the selected tab",
        UiTextKey::CommandTabRenameTitle => "Rename Tab",
        UiTextKey::CommandTabRenameDescription => "Rename the selected tab",
        UiTextKey::CommandTabNextTitle => "Next Tab",
        UiTextKey::CommandTabNextDescription => "Switch to the next project tab",
        UiTextKey::CommandTabPrevTitle => "Previous Tab",
        UiTextKey::CommandTabPrevDescription => "Switch to the previous project tab",
        UiTextKey::CommandTabPaletteTitle => "Open Tab Palette",
        UiTextKey::CommandTabPaletteDescription => "Switch tabs in the selected project",
        UiTextKey::CommandPaneSplitHorizontalTitle => "Split Pane Horizontally",
        UiTextKey::CommandPaneSplitHorizontalDescription => {
            "Split the focused pane into top and bottom panes"
        }
        UiTextKey::CommandPaneSplitVerticalTitle => "Split Pane Vertically",
        UiTextKey::CommandPaneSplitVerticalDescription => {
            "Split the focused pane into left and right panes"
        }
        UiTextKey::CommandPaneCloseTitle => "Close Pane",
        UiTextKey::CommandPaneCloseDescription => "Close the focused pane",
        UiTextKey::CommandPaneFocusLeftTitle => "Focus Pane Left",
        UiTextKey::CommandPaneFocusLeftDescription => "Move focus to the pane on the left",
        UiTextKey::CommandPaneFocusRightTitle => "Focus Pane Right",
        UiTextKey::CommandPaneFocusRightDescription => "Move focus to the pane on the right",
        UiTextKey::CommandPaneFocusUpTitle => "Focus Pane Up",
        UiTextKey::CommandPaneFocusUpDescription => "Move focus to the pane above",
        UiTextKey::CommandPaneFocusDownTitle => "Focus Pane Down",
        UiTextKey::CommandPaneFocusDownDescription => "Move focus to the pane below",
        UiTextKey::CommandPaneResizeLeftTitle => "Resize Pane Left",
        UiTextKey::CommandPaneResizeLeftDescription => "Resize the focused split toward the left",
        UiTextKey::CommandPaneResizeRightTitle => "Resize Pane Right",
        UiTextKey::CommandPaneResizeRightDescription => "Resize the focused split toward the right",
        UiTextKey::CommandPaneResizeUpTitle => "Resize Pane Up",
        UiTextKey::CommandPaneResizeUpDescription => "Resize the focused split upward",
        UiTextKey::CommandPaneResizeDownTitle => "Resize Pane Down",
        UiTextKey::CommandPaneResizeDownDescription => "Resize the focused split downward",
        UiTextKey::CommandPaneRenameTitle => "Rename Pane",
        UiTextKey::CommandPaneRenameDescription => "Rename the focused pane",
        UiTextKey::CommandPanePaletteTitle => "Open Pane Palette",
        UiTextKey::CommandPanePaletteDescription => "Focus panes in the selected tab",
        UiTextKey::CommandLayoutDefaultEditTitle => "Edit Default Layout",
        UiTextKey::CommandLayoutDefaultEditDescription => "Edit the global default layout TOML",
        UiTextKey::CommandLayoutDefaultResetTitle => "Reset Default Layout",
        UiTextKey::CommandLayoutDefaultResetDescription => {
            "Reset the global default layout to the built-in template"
        }
        UiTextKey::CommandLayoutDefaultReloadTitle => "Reload Default Layout",
        UiTextKey::CommandLayoutDefaultReloadDescription => {
            "Reload the global default layout from disk"
        }
        UiTextKey::CommandLayoutProjectEditTitle => "Edit Project Layout",
        UiTextKey::CommandLayoutProjectEditDescription => {
            "Edit the selected project's effective layout source"
        }
        UiTextKey::CommandLayoutSaveCurrentTitle => "Save Current Layout",
        UiTextKey::CommandLayoutSaveCurrentDescription => {
            "Save the current layout as a local override"
        }
        UiTextKey::CommandLayoutExportProjectConfigTitle => "Export Project Layout",
        UiTextKey::CommandLayoutExportProjectConfigDescription => {
            "Write the current layout to the project config"
        }
        UiTextKey::CommandLayoutResetLocalOverrideTitle => "Reset Personal Layout Override",
        UiTextKey::CommandLayoutResetLocalOverrideDescription => {
            "Remove the selected project's personal layout override"
        }
        UiTextKey::CommandLayoutOpenFileTitle => "Open Layout File",
        UiTextKey::CommandLayoutOpenFileDescription => {
            "Reveal the selected project's layout file path"
        }
        UiTextKey::LayoutEditorPlaceholder => "Edit layout TOML...",
        UiTextKey::LayoutEditorReadFailed => "Failed to read layout TOML",
        UiTextKey::LayoutEditorParseFailed => "Failed to parse layout TOML",
        UiTextKey::LayoutEditorValidationFailed => "Invalid layout TOML",
        UiTextKey::LayoutEditorPersonalInvalid => "Invalid personal layout",
        UiTextKey::LayoutEditorRequiresPatchMode => {
            "Personal layout editor requires mode = \"patch\""
        }
        UiTextKey::LayoutEditorRequiresReplaceMode => {
            "Personal layout editor requires mode = \"replace\""
        }
        UiTextKey::LayoutEditorSaveFailed => "Failed to save layout TOML",
        UiTextKey::CommandPaletteOpenTitle => "Open Command Palette",
        UiTextKey::CommandPaletteOpenDescription => "Search and run commands",
        UiTextKey::CommandSettingsOpenTitle => "Open Settings",
        UiTextKey::CommandSettingsOpenDescription => "Configure YTTT",
        UiTextKey::CommandSettingsKeybindingsTitle => "Open Keybindings File",
        UiTextKey::CommandSettingsKeybindingsDescription => {
            "Open or create the editable keybindings TOML"
        }
        UiTextKey::CommandSettingsNotificationsTitle => "Toggle Notifications",
        UiTextKey::CommandSettingsNotificationsDescription => {
            "Toggle system notifications for agent exits"
        }
        UiTextKey::GitBranchesLoading => "Loading Git branches…",
        UiTextKey::GitBranchSwitchFailed => "Could not switch Git branch",
        UiTextKey::GitBranchLocal => "Local branch",
        UiTextKey::GitBranchRemote => "Remote branch",
        UiTextKey::GitBranchAlreadyActive => "Already active",
        UiTextKey::GitDiffTitle => "Git Changes",
        UiTextKey::GitDiffLoading => "Loading working tree diff…",
        UiTextKey::GitDiffClean => "Working tree is clean",
        UiTextKey::GitDiffFile => "file",
        UiTextKey::GitDiffFiles => "files",
        UiTextKey::GitDiffFilesHeading => "Changed Files",
        UiTextKey::GitDiffWhitespace => "Ignore whitespace",
        UiTextKey::GitDiffUnified => "Unified",
        UiTextKey::GitDiffSplit => "Split",
        UiTextKey::GitDiffUnstaged => "Unstaged",
        UiTextKey::GitDiffStaged => "Staged",
        UiTextKey::GitDiffCloseHint => "close",
        UiTextKey::GitDiffStageHint => "staged / unstaged",
        UiTextKey::GitDiffSplitHint => "split / unified",
        UiTextKey::GitDiffNavigateHint => "navigate files",
        UiTextKey::GitDiffCopyHint => "copy",
        UiTextKey::GitDiffBinaryUnavailable => "Binary file — diff unavailable",
        UiTextKey::GitDiffSourceHead => "HEAD",
        UiTextKey::GitDiffSourceIndex => "Index",
        UiTextKey::GitDiffSourceWorkingTree => "Working Tree",
        UiTextKey::CloseProjectTitle => "Close project?",
        UiTextKey::CloseProjectBody => "Running terminal processes will be stopped.",
        UiTextKey::UnsavedChangesTitle => "Unsaved changes",
        UiTextKey::CloseWindowTitle => "Close YTTT?",
        UiTextKey::UnsavedFileSingular => "unsaved file",
        UiTextKey::UnsavedFilePlural => "unsaved files",
        UiTextKey::RunningProcessSingular => "running process",
        UiTextKey::RunningProcessPlural => "running processes",
        UiTextKey::SaveAllAndContinue => "Save All and Continue",
        UiTextKey::Discard => "Discard",
        UiTextKey::DiscardAndContinue => "Discard and Continue",
        UiTextKey::CloseSaveFailureGuidance => {
            "Fix the save error or discard the changes to continue."
        }
        UiTextKey::Cancel => "Cancel",
        UiTextKey::CloseProjectAction => "Close Project",
        UiTextKey::RenameTabTitle => "Rename tab",
        UiTextKey::RenameTabAction => "Rename",
        UiTextKey::RenameTabHint => "Enter to rename, Escape to cancel",
        UiTextKey::OpenNotificationTarget => "Open",
        UiTextKey::SettingsSearchPlaceholder => "Search settings...",
        UiTextKey::SettingsGroupGeneral => "General",
        UiTextKey::SettingsGroupGeneralDescription => "Application behavior and notifications",
        UiTextKey::SettingsGroupAppearance => "Appearance",
        UiTextKey::SettingsGroupAppearanceDescription => "UI font, UI and terminal themes",
        UiTextKey::SettingsGroupLanguages => "Languages",
        UiTextKey::SettingsGroupLanguagesDescription => {
            "Code language detection and language server defaults"
        }
        UiTextKey::SettingsGroupEditor => "Editor",
        UiTextKey::SettingsGroupEditorDescription => "Text editing and project file tree behavior",
        UiTextKey::SettingsGroupTerminal => "Terminal",
        UiTextKey::SettingsGroupTerminalDescription => "Shell, font, and terminal runtime defaults",
        UiTextKey::SettingsGroupProjectLayout => "Project Layout",
        UiTextKey::SettingsGroupProjectLayoutDescription => "Project layout files and TOML editing",
        UiTextKey::SettingsGroupDefaultLayout => "Default Layout",
        UiTextKey::SettingsGroupDefaultLayoutDescription => {
            "Global layout inherited by projects without project config"
        }
        UiTextKey::SettingsGroupKeybindings => "Keybindings",
        UiTextKey::SettingsGroupKeybindingsDescription => {
            "Keyboard shortcuts and conflict diagnostics"
        }
        UiTextKey::SettingsLanguage => "Language",
        UiTextKey::SettingsLanguageDescription => "Application display language.",
        UiTextKey::SettingsSelectLanguage => "Select language",
        UiTextKey::SettingsSystemNotifications => "System notifications",
        UiTextKey::SettingsSystemNotificationsDescription => {
            "Notify when agent terminal tasks complete or fail."
        }
        UiTextKey::SettingsRestoreLastSession => "Restore projects on startup",
        UiTextKey::SettingsRestoreLastSessionDescription => {
            "Open every project that was still open when YTTT last exited."
        }
        UiTextKey::SettingsPerformanceMetrics => "Application performance metrics",
        UiTextKey::SettingsPerformanceMetricsDescription => {
            "Show project, terminal, tab, editor, and application resource metrics in the title bar."
        }
        UiTextKey::SettingsSystemPerformanceMetrics => "System-wide CPU and memory",
        UiTextKey::SettingsSystemPerformanceMetricsDescription => {
            "Show total system CPU and memory usage in the title bar."
        }
        UiTextKey::PerformanceProjects => "Projects",
        UiTextKey::PerformanceTerminals => "Terminals",
        UiTextKey::PerformanceTabs => "Tabs",
        UiTextKey::PerformanceEditors => "Editors",
        UiTextKey::PerformanceCpu => "Application CPU",
        UiTextKey::PerformanceMemory => "Application memory",
        UiTextKey::PerformanceSystemCpu => "System CPU",
        UiTextKey::PerformanceSystemMemory => "System memory",
        UiTextKey::SettingsNewTabCommandPicker => "New tab command picker",
        UiTextKey::SettingsNewTabCommandPickerDescription => {
            "Show a command picker when clicking the new tab button."
        }
        UiTextKey::SettingsNewTabCommands => "New tab commands",
        UiTextKey::SettingsNewTabCommandsDescription => {
            "Commands available in the new tab picker. Each command runs in the selected project directory."
        }
        UiTextKey::SettingsNewTabCommandPlaceholder => "e.g. lazygit, nvim ., codex",
        UiTextKey::SettingsAddCommand => "Add command",
        UiTextKey::SettingsWindowEffect => "Window background effect",
        UiTextKey::SettingsWindowEffectDescription => {
            "Choose an opaque window, plain transparency, or frosted glass."
        }
        UiTextKey::SettingsWindowEffectNone => "None",
        UiTextKey::SettingsWindowEffectTransparent => "Transparent",
        UiTextKey::SettingsWindowEffectBlurred => "Frosted glass",
        UiTextKey::SettingsWindowOpacity => "Main window opacity",
        UiTextKey::SettingsWindowOpacityDescription => {
            "Shared opacity for the title bar, sidebars, editors, and terminals. 0.00 is fully transparent; 1.00 is opaque. Used by Transparent and Frosted glass."
        }
        UiTextKey::SettingsUiFontFamily => "UI font",
        UiTextKey::SettingsUiFontFamilyDescription => {
            "Font used across YTTT chrome, panels, and controls."
        }
        UiTextKey::SettingsUiFontSize => "UI font size",
        UiTextKey::SettingsUiFontSizeDescription => {
            "Base UI font size in pixels; controls scale proportionally."
        }
        UiTextKey::SettingsUiLineHeight => "UI line height",
        UiTextKey::SettingsUiLineHeightDescription => "Line-height multiplier for interface text.",
        UiTextKey::SettingsUiTheme => "UI theme",
        UiTextKey::SettingsUiThemeDescription => {
            "Theme used for YTTT chrome, panels, and controls."
        }
        UiTextKey::SettingsUiStyle => "UI style",
        UiTextKey::SettingsUiStyleDescription => {
            "Global shape, density, borders, hover treatment, and elevation."
        }
        UiTextKey::SettingsIconTheme => "Icon theme",
        UiTextKey::SettingsIconThemeDescription => {
            "File, folder, and editor icons from installed Zed-compatible themes."
        }
        UiTextKey::SettingsTerminalTheme => "Terminal theme",
        UiTextKey::SettingsTerminalThemeDescription => "Optional terminal colors override.",
        UiTextKey::SettingsSearchTheme => "Search theme...",
        UiTextKey::SettingsEditSettingsToml => "Edit settings TOML",
        UiTextKey::SettingsEditSettingsTomlDescription => {
            "Open the app settings file for advanced edits."
        }
        UiTextKey::SettingsShowPath => "Show Path",
        UiTextKey::SettingsThemesDirectory => "Themes directory",
        UiTextKey::SettingsThemesDirectoryDescription => {
            "Open the folder containing user theme TOML files."
        }
        UiTextKey::SettingsImportZedThemes => "Import Zed themes",
        UiTextKey::SettingsImportZedThemesDescription => {
            "Detect and import compatible UI and icon themes from installed Zed extensions."
        }
        UiTextKey::SettingsImportZedThemesAction => "Import",
        UiTextKey::SettingsImportZedThemesComplete => "Zed themes imported",
        UiTextKey::SettingsImportZedThemesNone => "No compatible Zed themes were detected.",
        UiTextKey::SettingsImportZedThemesImported => "Already imported",
        UiTextKey::SettingsImportZedThemesConflictHint => "For already imported themes:",
        UiTextKey::SettingsImportZedThemesSkipExisting => "Skip existing",
        UiTextKey::SettingsImportZedThemesOverwriteExisting => "Overwrite existing",
        UiTextKey::SettingsLanguageDetection => "Language detection",
        UiTextKey::SettingsLanguageDetectionDescription => {
            "Detect code editor language from filename, extension, and first line."
        }
        UiTextKey::SettingsDefaultCodeLanguage => "Default code language",
        UiTextKey::SettingsDefaultCodeLanguageDescription => {
            "Fallback language used when automatic detection is disabled or unknown."
        }
        UiTextKey::SettingsSupportedLanguages => "Supported languages",
        UiTextKey::SettingsSupportedLanguagesDescription => {
            "Built-in languages available to the code editor."
        }
        UiTextKey::SettingsLanguageServer => "Language server",
        UiTextKey::SettingsLanguageServerDescription => {
            "Reserve an LSP launch point for future diagnostics and completion."
        }
        UiTextKey::SettingsLanguageServerCommand => "Language server command",
        UiTextKey::SettingsLanguageServerCommandDescription => {
            "Command reserved for the default language server integration."
        }
        UiTextKey::SettingsSearchCodeLanguage => "Search language...",
        UiTextKey::SettingsEditorFontFamily => "Font family",
        UiTextKey::SettingsEditorFontFamilyDescription => "Font used by project file editors.",
        UiTextKey::SettingsEditorFontSize => "Font size",
        UiTextKey::SettingsEditorFontSizeDescription => "Editor font size in pixels.",
        UiTextKey::SettingsEditorLineHeight => "Line height",
        UiTextKey::SettingsEditorLineHeightDescription => "Editor line height multiplier.",
        UiTextKey::SettingsEditorTabSize => "Tab size",
        UiTextKey::SettingsEditorTabSizeDescription => {
            "Number of spaces per tab. Reopen already-open files to apply this change."
        }
        UiTextKey::SettingsEditorSoftWrap => "Soft wrap",
        UiTextKey::SettingsEditorSoftWrapDescription => {
            "Wrap long editor lines to the available width."
        }
        UiTextKey::SettingsEditorLineNumbers => "Line numbers",
        UiTextKey::SettingsEditorLineNumbersDescription => "Show line numbers in project files.",
        UiTextKey::SettingsEditorVimMode => "Vim mode",
        UiTextKey::SettingsEditorVimModeDescription => {
            "Use modal Vim keybindings in project code editors."
        }
        UiTextKey::SettingsEditorAutosave => "Autosave",
        UiTextKey::SettingsEditorAutosaveDescription => "Choose when edited files are saved.",
        UiTextKey::SettingsEditorAutosaveOff => "Off",
        UiTextKey::SettingsEditorAutosaveOnFocusChange => "On focus change",
        UiTextKey::SettingsEditorAutosaveAfterDelay => "After delay",
        UiTextKey::SettingsEditorAutosaveDelay => "Autosave delay",
        UiTextKey::SettingsEditorAutosaveDelayDescription => {
            "Delay in milliseconds used by delayed autosave."
        }
        UiTextKey::SettingsProjectPanelDefaultOpen => "Open file tree by default",
        UiTextKey::SettingsProjectPanelDefaultOpenDescription => {
            "Show the file tree when a project is opened for the first time."
        }
        UiTextKey::SettingsProjectPanelShowHidden => "Show hidden files",
        UiTextKey::SettingsProjectPanelShowHiddenDescription => {
            "Include hidden files and directories in project trees."
        }
        UiTextKey::SettingsProjectPanelWidth => "File tree width",
        UiTextKey::SettingsProjectPanelWidthDescription => {
            "Default width of the right project file tree."
        }
        UiTextKey::SettingsProjectSidebarWidth => "Project sidebar width",
        UiTextKey::SettingsProjectSidebarWidthDescription => {
            "Width of the left opened-project sidebar."
        }
        UiTextKey::SettingsDefaultShell => "Default shell",
        UiTextKey::SettingsDefaultShellDescription => {
            "Shell used by layout panes in shell execution mode and new terminal tabs."
        }
        UiTextKey::SettingsSelectShell => "Select shell",
        UiTextKey::SettingsCustomShell => "Custom shell",
        UiTextKey::SettingsCustomShellDescription => {
            "Add an executable path or command name to the saved shell list."
        }
        UiTextKey::SettingsCustomShellPlaceholder => "Shell path or command",
        UiTextKey::SettingsAddShell => "Add",
        UiTextKey::SettingsFontFamily => "Font family",
        UiTextKey::SettingsFontFamilyDescription => "Terminal font family.",
        UiTextKey::SettingsSearchFont => "Search font...",
        UiTextKey::SettingsFontSize => "Font size",
        UiTextKey::SettingsFontSizeDescription => "Terminal font size in pixels.",
        UiTextKey::SettingsLineHeight => "Line height",
        UiTextKey::SettingsLineHeightDescription => "Terminal line height multiplier.",
        UiTextKey::SettingsPadding => "Padding",
        UiTextKey::SettingsPaddingDescription => "Terminal pane inner padding.",
        UiTextKey::SettingsScrollback => "Scrollback",
        UiTextKey::SettingsScrollbackDescription => "Number of terminal lines kept in memory.",
        UiTextKey::SettingsScrollbar => "Scrollbar",
        UiTextKey::SettingsScrollbarDescription => {
            "Show a thin scrollback indicator in terminal panes."
        }
        UiTextKey::SettingsTerminalCursorShape => "Cursor shape",
        UiTextKey::SettingsTerminalCursorShapeDescription => "Shape of the terminal cursor.",
        UiTextKey::SettingsTerminalCursorBlinking => "Blinking cursor",
        UiTextKey::SettingsTerminalCursorBlinkingDescription => {
            "Blink the terminal cursor while the terminal is focused."
        }
        UiTextKey::SettingsTerminalHideMouseWhenTyping => "Hide mouse while typing",
        UiTextKey::SettingsTerminalHideMouseWhenTypingDescription => {
            "Hide the pointer while keyboard input is sent to the terminal."
        }
        UiTextKey::SettingsTerminalCopyOnSelect => "Copy on select",
        UiTextKey::SettingsTerminalCopyOnSelectDescription => {
            "Copy completed terminal selections without an explicit Copy action."
        }
        UiTextKey::SettingsTerminalOsc52Policy => "OSC 52 clipboard access",
        UiTextKey::SettingsTerminalOsc52PolicyDescription => {
            "Control terminal application access to the system clipboard."
        }
        UiTextKey::SettingsTerminalKittyKeyboard => "Kitty keyboard protocol",
        UiTextKey::SettingsTerminalKittyKeyboardDescription => {
            "Allow terminal applications to negotiate the Kitty keyboard protocol."
        }
        UiTextKey::SettingsLayoutSource => "Layout source",
        UiTextKey::SettingsLayoutSourceDescription => "Current project layout source.",
        UiTextKey::SettingsOpenProjectFirst => "Open a project first",
        UiTextKey::SettingsSaveCurrentLayout => "Save current layout",
        UiTextKey::SettingsSaveCurrentLayoutDescription => {
            "Save current layout as an app-local override."
        }
        UiTextKey::SettingsExportProjectLayout => "Export project layout",
        UiTextKey::SettingsExportProjectLayoutDescription => {
            "Write current layout into the project config."
        }
        UiTextKey::SettingsEditLayoutToml => "Edit layout TOML",
        UiTextKey::SettingsEditLayoutTomlDescription => "Edit the selected project layout file.",
        UiTextKey::SettingsDefaultLayoutPath => "Default layout file",
        UiTextKey::SettingsDefaultLayoutPathDescription => {
            "Global default layout TOML used by projects without project config."
        }
        UiTextKey::SettingsEditDefaultLayout => "Edit default layout TOML",
        UiTextKey::SettingsEditDefaultLayoutDescription => "Edit the global default layout file.",
        UiTextKey::SettingsReloadDefaultLayout => "Reload default layout",
        UiTextKey::SettingsReloadDefaultLayoutDescription => {
            "Reload the global default layout from disk."
        }
        UiTextKey::SettingsResetDefaultLayout => "Reset default layout",
        UiTextKey::SettingsResetDefaultLayoutDescription => {
            "Replace the global default layout with the built-in template."
        }
        UiTextKey::SettingsEditKeybindingsToml => "Edit keybindings TOML",
        UiTextKey::SettingsEditKeybindingsTomlDescription => "Open the user keybindings file.",
        UiTextKey::SettingsKeybindingDiagnostics => "Keybinding diagnostics",
        UiTextKey::SettingsKeybindingDiagnosticsDescription => {
            "Show invalid commands and shortcut conflicts."
        }
        UiTextKey::SettingsNoKeybindingConflicts => "No keybinding conflicts",
        UiTextKey::SettingsUnbound => "Unbound",
        UiTextKey::SettingsConflict => "conflict",
        UiTextKey::SettingsKeybindingDialogTitle => "Edit keybinding",
        UiTextKey::SettingsKeybindingRecorderPrompt => "Press a shortcut",
        UiTextKey::SettingsKeybindingRecorderHint => {
            "The first recorded shortcut replaces the current bindings. Keep pressing to add alternatives; modifier keys alone are ignored."
        }
        UiTextKey::SettingsClearKeybindings => "Clear",
        UiTextKey::SettingsConflictingKeybinding => "Conflicting keybinding",
        UiTextKey::SettingsInvalidCommandId => "Invalid command id",
        UiTextKey::SettingsSave => "Save",
        UiTextKey::SettingsExport => "Export",
        UiTextKey::SettingsOpen => "Open",
        UiTextKey::SettingsEdit => "Edit",
        UiTextKey::SettingsReset => "Reset",
        UiTextKey::SettingsDelete => "Delete",
        UiTextKey::StatusSystemNotificationsEnabled => "System notifications: enabled",
        UiTextKey::StatusSystemNotificationsDisabled => "System notifications: disabled",
        UiTextKey::StatusErrorContext => "Error",
        UiTextKey::StatusWarningContext => "Warning",
        UiTextKey::StatusKeybindingsFile => "Keybindings file",
        UiTextKey::StatusLayoutFile => "Layout file",
        UiTextKey::StatusSettingsFile => "Settings file",
        UiTextKey::StatusThemesDirectory => "Themes directory",
    }
}
