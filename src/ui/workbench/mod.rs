use gpui::{
    AnyElement, App, ClickEvent, ClipboardItem, Context, Div, DragMoveEvent, Entity, FocusHandle,
    Focusable as _, FontWeight, HighlightStyle, InteractiveElement as _, IntoElement, KeyDownEvent,
    Keystroke, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, ParentElement as _,
    PathPromptOptions, Pixels, Point, Render, Rgba, ScrollHandle, SharedString, Stateful,
    Subscription, Task, UniformListScrollHandle, Window, div, prelude::*, px, relative, rems, rgba,
};
use gpui_component::{
    ActiveTheme as _, Disableable as _, IconName, IndexPath, Root as ComponentRoot, Sizable as _,
    Theme as ComponentTheme, WindowExt as _,
    button::{Button, ButtonVariants as _},
    dialog::DialogFooter,
    highlighter::SyntaxHighlighter,
    input::{Input, InputEvent, InputState, NumberInput, NumberInputEvent, Rope, StepAction},
    radio::Radio,
    scroll::ScrollableElement as _,
    searchable_list::{SearchableListDelegate, SearchableListItem},
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use yttt_terminal::input::{KeyState, TerminalKeyEvent};
use yttt_terminal::{TerminalCursorShape, TerminalOsc52Policy};

mod action_handlers;
mod dialogs;
mod document_lifecycle;
mod file_finder;
mod git;
mod helpers;
pub mod layout_editor;
mod layout_editor_controller;
#[cfg(test)]
mod non_destructive_tests;
mod onboarding;
mod palette;
mod performance;
mod project_files;
mod render;
mod resize;
mod settings;
pub mod shell;
mod ssh_connections;
mod ssh_project_picker;
mod state;
mod surface;
mod work_area;
use dialogs::*;
use git::*;
use helpers::*;
use onboarding::*;
use render::{push_component_notification, split_child};
use settings::{settings_button, settings_overlay};
use ssh_connections::{ssh_connections_overlay, ssh_host_key_overlay};
use ssh_project_picker::ssh_project_picker_overlay;
use state::{
    documents::DocumentLifecycleState,
    overlays::OverlayControllerState,
    palette::PaletteControllerState,
    project::{ProjectControllerState, ProjectTreeClipboard},
    settings::{SettingsControllerState, ZedThemeImportDialogState},
    ssh::{
        SshConnectionForm, SshConnectionFormInputs, SshConnectionFormMode, SshConnectionListAction,
        SshConnectionListDelegate, SshConnectionListEntry, SshConnectionListSection,
        SshConnectionListTone, SshControllerState, SshProjectConnectContinuation,
        SshProjectDirectory, SshProjectPickerView,
    },
    terminal::{TerminalControllerState, TerminalPaneTarget},
};

use std::{
    collections::{BTreeMap, BTreeSet, HashMap, HashSet},
    fs,
    ops::Range,
    path::{Path, PathBuf},
    rc::Rc,
    sync::Arc,
    time::Duration,
};

type SettingsStringSelectState = SelectState<SearchableVec<String>>;
type SettingsFontFamilySelectState = SelectState<FontFamilyOptions>;

const TERMINAL_THEME_FOLLOW_UI: &str = "Follow UI theme";
const ICON_THEME_BUILTIN: &str = "Built-in";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SettingsNumberField {
    WindowOpacity,
    UiFontSize,
    UiLineHeight,
    FontSize,
    LineHeight,
    Padding,
    Scrollback,
    EditorFontSize,
    EditorLineHeight,
    EditorTabSize,
    EditorAutosaveDelay,
    ProjectPanelWidth,
    ProjectSidebarWidth,
}

use crate::{
    commands::{
        ActiveSurface, CommandContext, CommandDispatchError, CommandId, CommandRegistry,
        default_registry, dispatch_workspace_command,
    },
    config::{
        default_layout::{
            BuiltinAgent, DefaultLayoutKind, DefaultLayoutState, DefaultLayoutTemplate,
            LayoutLoadWarning,
        },
        keybindings::{
            KeybindingLoadWarning, KeybindingsLoadError, ensure_keybindings_file, load_keybindings,
        },
        layout_loader::{
            LayoutSource, PersonalLayout, ProjectOpenError, ProjectReferenceConfig,
            RecentProjectsConfig, create_project_layout_scaffold, export_project_layout,
            load_recent_projects, open_project_config, open_ssh_project_config,
            parse_personal_layout, remove_recent_projects_for_ssh_connection, reset_local_override,
            save_last_opened_projects, save_local_layout,
        },
        paths::AppConfigPaths,
        settings::{
            AppSettings, DEFAULT_UI_FONT_SIZE, DEFAULT_UI_LINE_HEIGHT, DEFAULT_WINDOW_OPACITY,
            EditorAutosave, LanguageSetting, MAX_UI_FONT_SIZE, MAX_UI_LINE_HEIGHT,
            MAX_WINDOW_OPACITY, MIN_UI_FONT_SIZE, MIN_UI_LINE_HEIGHT, MIN_WINDOW_OPACITY,
            SettingsLoadWarning, SettingsSaveError, WindowBackgroundEffect,
            detect_shell_candidates, detect_system_language_setting, load_or_create_settings,
            resolve_default_shell, save_settings,
        },
        theme::{ThemeLoadWarning, ThemeStore, load_theme_store},
    },
    model::{
        ids::ProjectId,
        layout::{LayoutNode, PaneConfig, ProcessExitBehavior, ProjectLayout, SplitDirection},
        project::{ProjectDescriptor, ProjectLocation},
        workspace::{
            AgentStatus, CloseProjectDecision, CloseProjectError, PaneExitCloseOutcome, Workspace,
            WorkspaceError,
        },
    },
    palette::{
        ActivePalette, CommandPaletteContext, PaletteItem, PaletteKind, RecentProject,
        TabPaletteSnapshot, command_palette_items_with_text, command_title_with_text,
        decode_tab_palette_item_id, new_tab_command_palette_items,
        opened_project_palette_items_with_text, pane_palette_items_with_text,
        project_palette_items_with_text, recent_project_palette_items_with_text,
        tab_palette_items_with_text, unified_tab_palette_items,
    },
    runtime::{
        file_search::{
            FileSearchCandidate, FileSearchCollection, FileSearchProject,
            collect_file_search_candidates, match_file_search_candidates,
        },
        git_status::{
            GitDiffLine, GitDiffLineKind, GitDiffMode, GitDiffResult, GitFileChangeKind,
            GitFileDiff, read_project_git_branches_with, read_project_git_diff_result_with,
            read_project_git_status, read_project_git_status_with, switch_project_git_branch_with,
        },
        notification::{
            NoopSystemNotifier, NotificationEvent, NotificationKind, maybe_notify_system,
        },
        project::ProjectServices,
    },
    ui::{
        app::{platform, startup::startup_project_paths},
        components::{
            ActionEmphasis, workbench_action_button, workbench_agent_notification,
            workbench_error_notification, workbench_icon_button, workbench_inline_notification,
            workbench_keybinding_badge, workbench_settings_row, workbench_status_notification,
            workbench_switch,
        },
        editor::{
            CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, CurrentDiskState,
            EditorAppearance, EditorDiagnostic, EditorDiagnosticSeverity, EditorLanguageCatalog,
            EditorLanguageId, LoadedProjectFile, MarkdownDocumentConfig, ProjectEditorDocument,
            ProjectEditorDocumentEvent, ProjectEditorModel, ProjectEditorRuntime,
            ProjectEditorSaveState, ProjectFileIoError, ProjectFileLoadRequest, ReadonlyCodeRow,
            ReadonlyCodeView, SaveProjectFileOutcome, SaveRequest, TabGroup, TabGroupId,
            WorkAreaDropEdge, WorkAreaDropPlacement, WorkAreaNode, WorkAreaSplitAxis,
            WorkAreaSplitId, WorkItemId, code_editor_input_state, styled_code_editor_input,
        },
        i18n::{Locale, UiText, UiTextKey},
        interaction::actions::{
            CreateProject, FileSave, GitBranchSwitch, GitDiffOpen, LayoutDefaultEdit,
            LayoutDefaultReload, LayoutDefaultReset, LayoutExportProjectConfig, LayoutOpenFile,
            LayoutProjectEdit, LayoutResetLocalOverride, LayoutSaveCurrent, OpenCommandPalette,
            OpenFileFinder, OpenOpenedProjectPalette, OpenPanePalette, OpenProject,
            OpenProjectPalette, OpenSshProject, OpenTabPalette, PaletteCancel, PaletteConfirm,
            PaletteSelectNext, PaletteSelectPrev, PaneClose, PaneFocusDown, PaneFocusLeft,
            PaneFocusRight, PaneFocusUp, PaneRename, PaneResizeDown, PaneResizeLeft,
            PaneResizeRight, PaneResizeUp, PaneSplitHorizontal, PaneSplitVertical, ProjectClose,
            ProjectPanelRefresh, ProjectPanelToggle, SettingsKeybindings, SettingsNotifications,
            SettingsOpen, TabClose, TabCloseAfter, TabCloseAll, TabCloseAllFiles,
            TabCloseAllTerminals, TabCloseBefore, TabNew, TabNext, TabPrev, TabRename,
            UiKeybindingSpec, WORKSPACE_CONTEXT, runtime_command_for_keystroke,
            ui_keybinding_specs_from_config,
        },
        interaction::input_owner::{
            InputOwnerKind, InputOwnerRegistration, InputScopeId, TerminalInputGate,
        },
        interaction::key_dispatch::workspace_command_for_keystroke,
        interaction::overlay::capture_overlay_input,
        notifications::{ToastItem, ToastQueue, ToastTone, toast_item_for_event},
        palette::surface::palette_input_placeholder,
        palette::{file_finder_palette_overlay, palette_overlay},
        primitives::{
            button::{YtttButtonVariant, yttt_button},
            dialog::yttt_dialog_style,
            icon_button::YtttIconButtonKind,
            input::{YtttInputKind, yttt_input_style},
            panel::{YtttPanelKind, yttt_panel_style},
            select::yttt_select_style,
            sidebar::{
                PROJECT_FILE_PANEL_MAX_WIDTH, PROJECT_FILE_PANEL_MIN_WIDTH,
                PROJECT_SIDEBAR_MAX_WIDTH, PROJECT_SIDEBAR_MIN_WIDTH,
                SIDEBAR_RESIZE_HIT_AREA_WIDTH, SidebarSide, resize_sidebar_width,
            },
        },
        project_tree::{
            DirectoryLoadRequest, DirectorySnapshot, ProjectEntryFsError, ProjectEntryPasteMode,
            ProjectTreeFsError, ProjectTreeInteractionText, ProjectTreeLoadState,
            ProjectTreeRenderSnapshot, ProjectTreeRenderText, ProjectTreeView,
            ProjectTreeViewEvent,
        },
        settings::font_options::{
            FontFamilyOptions, detect_installed_monospace_nerd_font,
            font_family_option_for_setting, font_family_options_from_system,
            font_family_setting_from_option, terminal_font_family_option_for_setting,
            terminal_font_family_options_from_system, terminal_font_family_setting_from_option,
        },
        settings::keybinding_display::{
            primary_display_keybinding_for_current_platform, recorded_keybinding,
        },
        settings::keybindings::{KeybindingEditError, KeybindingRow, KeybindingsEditorState},
        settings::{SettingsGroupId, SettingsPanelStyle, settings_panel_style},
        terminal::pane::{
            SshTerminalContext, TerminalPaneContext, TerminalPaneEvent, TerminalPaneExitedEvent,
            TerminalPaneStartedEvent, TerminalPaneView,
        },
        theme::{
            AppearanceState, EditorTheme, ThemeRuntime, UiStyle, UiStyleId, WorkbenchTheme,
            current_ui_style,
            icons::{
                IconTheme, available_icon_theme_names as load_icon_theme_names, load_icon_theme,
            },
            zed::{
                DetectedZedExtension, ZedThemeDetection, ZedThemeImportConflictPolicy,
                detect_installed_zed_themes, import_detected_zed_themes_with_policy,
                zed_icon_theme_output_path, zed_ui_theme_output_path,
            },
        },
        workbench::layout_editor::{
            LayoutEditorSession, LayoutEditorTarget, ProjectLayoutEditorFormat,
            write_layout_file_atomic,
        },
        workbench::shell::sidebar::project_sidebar,
        workbench::shell::split_view::{pointer_resize_for_drag_delta, split_child_basis},
        workbench::shell::tabs::{
            DraggedWorkbenchTab, FileTabSnapshot, ProjectTabsToolbar, WorkbenchTabCloseScope,
            WorkbenchTabItem, project_tabs, tab_close_targets, visible_tab_items,
            visible_work_item_tabs as merge_work_item_tabs,
        },
        workbench::shell::titlebar::{TitlebarInfo, compact_path_for_titlebar, workbench_titlebar},
    },
};

pub use crate::ui::interaction::overlay::overlay_input_capture_policy;

pub struct WorkbenchView {
    workspace: Workspace,
    config_paths: AppConfigPaths,
    default_layout_state: DefaultLayoutState,
    onboarding: Option<OnboardingState>,
    palette: PaletteControllerState,
    recent_projects_config: RecentProjectsConfig,
    command_registry: CommandRegistry,
    load_error: Option<String>,
    presented_error_notification: Option<String>,
    project: ProjectControllerState,
    ssh: SshControllerState,
    settings: SettingsControllerState,
    performance: performance::PerformanceMonitorState,
    last_opened_layout_file: Option<PathBuf>,
    last_opened_keybindings_file: Option<PathBuf>,
    overlays: OverlayControllerState,
    documents: DocumentLifecycleState,
    pending_create_project_request: bool,
    pending_open_project_request: bool,
    pending_status_notifications: Vec<ToastItem>,
    focus_handle: Option<FocusHandle>,
    window_activation_subscription: Option<Subscription>,
    terminal: TerminalControllerState,
    sidebar_collapsed: bool,
    active_sidebar_resize_drag: Option<ActiveSidebarResizeDrag>,
    active_split_resize_drag: Option<ActiveSplitResizeDrag>,
    active_work_area_resize_drag: Option<ActiveWorkAreaResizeDrag>,
    work_area_drop_target: Option<WorkAreaDropTarget>,
    toast_queue: ToastQueue,
    system_notifier: NoopSystemNotifier,
    system_notifications_enabled: bool,
    ui_text: UiText,
    app_settings: AppSettings,
    appearance: AppearanceState,
    icon_theme: IconTheme,
    active_project_file_watcher: Option<ActiveProjectFileWatcher>,
    project_file_watching_enabled: bool,
}

struct WorkbenchErrorNotification;

struct ActiveProjectFileWatcher {
    project_id: ProjectId,
    project_path: PathBuf,
    _task: Task<()>,
}

const EMPTY_WORKSPACE_ACTIONS: [UiTextKey; 4] = [
    UiTextKey::OpenDirectory,
    UiTextKey::SshOpenRemoteProject,
    UiTextKey::OpenRecent,
    UiTextKey::CommandPalette,
];

fn palette_input_scope_id(kind: PaletteKind) -> &'static str {
    match kind {
        PaletteKind::Command => "palette.command",
        PaletteKind::NewTabCommand => "palette.new_tab_command",
        PaletteKind::File => "palette.file",
        PaletteKind::Project => "palette.project",
        PaletteKind::OpenedProject => "palette.opened_project",
        PaletteKind::RecentProject => "palette.recent_project",
        PaletteKind::Tab => "palette.tab",
        PaletteKind::Pane => "palette.pane",
        PaletteKind::GitBranch => "palette.git_branch",
    }
}

struct RenderTerminalPaneInput<'a> {
    group_id: TabGroupId,
    project_id: &'a str,
    project_path: &'a Path,
    project_title: &'a str,
    pane: &'a PaneConfig,
    tab_id: &'a str,
    tab_title: &'a str,
    is_focused: bool,
}

struct RenderTerminalTreeInput<'a> {
    group_id: TabGroupId,
    group_active: bool,
    project_id: &'a str,
    project_path: &'a Path,
    project_title: &'a str,
    tab_id: &'a str,
    tab_title: &'a str,
    focused_pane_id: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
struct ActiveSplitResizeDrag {
    direction: SplitDirection,
    last_position: Point<Pixels>,
}

#[derive(Clone, Copy, Debug)]
struct ActiveWorkAreaResizeDrag {
    split_id: WorkAreaSplitId,
    axis: WorkAreaSplitAxis,
    last_position: Point<Pixels>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct WorkAreaDropTarget {
    project_id: ProjectId,
    group_id: TabGroupId,
    edge: Option<WorkAreaDropEdge>,
}

#[derive(Clone, Copy, Debug)]
struct ActiveSidebarResizeDrag {
    side: SidebarSide,
    last_position: Point<Pixels>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum SaveContinuation {
    None,
    CompletePendingClose,
}

#[derive(Clone, Debug)]
struct PendingFileConflict {
    document_id: crate::ui::editor::DocumentId,
    request: SaveRequest,
    current_disk: CurrentDiskState,
    continuation: SaveContinuation,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum DirtyCloseIntent {
    File(crate::ui::editor::DocumentId),
    WorkItems {
        terminal_ids: Vec<String>,
        file_ids: Vec<crate::ui::editor::DocumentId>,
    },
    Project(ProjectId),
    Window,
}

#[derive(Clone, Debug)]
struct PendingDirtyClose {
    intent: DirtyCloseIntent,
    dirty_documents: Vec<crate::ui::editor::DocumentId>,
    running_pane_count: usize,
    saving_documents: HashSet<crate::ui::editor::DocumentId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingTabRename {
    tab_id: String,
    value: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct PendingKeybindingEdit {
    command: CommandId,
    keys: Vec<String>,
    has_recorded: bool,
    error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SplitHandleStyle {
    pub visible_line_width: Pixels,
    pub hit_area_width: Pixels,
}

impl WorkbenchView {
    pub fn new() -> Self {
        Self::with_config_paths(AppConfigPaths::for_app())
    }

    pub fn with_config_paths(config_paths: AppConfigPaths) -> Self {
        Self::with_config_paths_and_force_onboarding(config_paths, false)
    }

    pub fn with_config_paths_and_force_onboarding(
        config_paths: AppConfigPaths,
        force_onboarding: bool,
    ) -> Self {
        let mut root =
            Self::with_workspace_and_config_paths(Workspace::new(), config_paths, force_onboarding);
        if root.restore_last_session_enabled() {
            root.restore_projects_open_at_last_exit();
        }
        root
    }

    pub fn from_startup(force_onboarding: bool) -> Self {
        let mut root = Self::with_config_paths_and_force_onboarding(
            AppConfigPaths::for_app(),
            force_onboarding,
        );
        for project_path in startup_project_paths() {
            let _ = root.open_project_path(project_path);
        }
        root
    }
    pub fn has_last_opened_projects(&self) -> bool {
        !self.recent_projects_config.last_opened_projects.is_empty()
            || !self
                .recent_projects_config
                .last_restorable_projects
                .is_empty()
            || !self.recent_projects_config.projects.is_empty()
    }

    pub fn restore_last_opened_projects(&mut self) -> usize {
        let projects = if !self.recent_projects_config.last_opened_projects.is_empty() {
            self.recent_projects_config.last_opened_projects.clone()
        } else if !self
            .recent_projects_config
            .last_restorable_projects
            .is_empty()
        {
            self.recent_projects_config.last_restorable_projects.clone()
        } else {
            self.recent_projects_config
                .projects
                .first()
                .map(|project| {
                    vec![ProjectReferenceConfig::new(
                        project.id.clone(),
                        project.location.clone(),
                    )]
                })
                .unwrap_or_default()
        };
        self.restore_projects(projects)
    }

    fn restore_projects_open_at_last_exit(&mut self) -> usize {
        self.restore_projects(self.recent_projects_config.last_opened_projects.clone())
    }

    fn restore_projects(&mut self, projects: Vec<ProjectReferenceConfig>) -> usize {
        let mut restored = 0;
        let mut messages = self.load_error.take().into_iter().collect::<Vec<_>>();
        for project in projects {
            let result = match project.location {
                ProjectLocation::Local { path } => self.open_project_path(path),
                ProjectLocation::Ssh {
                    connection_id,
                    root,
                } => self.open_ssh_project_location(connection_id, root, false),
            };
            if result.is_ok() {
                restored += 1;
            }
            if let Some(message) = self.load_error.take() {
                push_unique_string(&mut messages, message);
            }
        }
        self.load_error = (!messages.is_empty()).then(|| messages.join("; "));
        restored
    }

    fn persist_opened_project_paths(&mut self) -> Option<String> {
        let projects = self
            .workspace
            .opened_projects()
            .iter()
            .map(|project| {
                ProjectReferenceConfig::new(project.id.clone(), project.location.clone())
            })
            .collect();
        save_last_opened_projects(
            &self.config_paths,
            &mut self.recent_projects_config,
            projects,
        )
        .err()
        .map(|error| error.to_string())
    }

    fn with_workspace_and_config_paths(
        workspace: Workspace,
        config_paths: AppConfigPaths,
        force_onboarding: bool,
    ) -> Self {
        let default_layout_state = DefaultLayoutState::load_or_create(&config_paths);
        let command_registry = default_registry();
        let recent_projects_config = load_recent_projects(&config_paths).unwrap_or_default();
        let (ssh, ssh_load_error) = SshControllerState::new(&config_paths);
        let recent_projects = recent_projects_for_palette(&recent_projects_config);
        let keybindings_editor = load_keybindings_editor_state(&config_paths, &command_registry);
        let (mut app_settings, settings_warning_lines) = load_app_settings_messages(&config_paths);
        let language_detection_error = (!app_settings.general.onboarding_completed
            && workspace.opened_projects().is_empty()
            && app_settings.general.language == LanguageSetting::System)
            .then(|| {
                app_settings.general.language = detect_system_language_setting();
                save_settings(&config_paths, &app_settings)
                    .err()
                    .map(|error| error.to_string())
            })
            .flatten();
        let ui_text = ui_text_for_language(app_settings.general.language);
        let (load_error, keybinding_warning_lines) =
            load_keybindings_messages(&config_paths, &command_registry, &ui_text);
        let (theme_runtime, theme_warning_lines) =
            load_theme_runtime_messages(&config_paths, &app_settings);
        let (icon_theme, icon_theme_error) =
            match load_icon_theme(&config_paths, app_settings.theme.icon_theme.as_deref()) {
                Ok(icon_theme) => (icon_theme, None),
                Err(error) => (IconTheme::default(), Some(error.to_string())),
            };
        let load_error = combine_load_messages(
            load_error,
            settings_warning_lines
                .iter()
                .chain(theme_warning_lines.iter())
                .next()
                .map(|_| {
                    settings_warning_lines
                        .iter()
                        .chain(theme_warning_lines.iter())
                        .cloned()
                        .collect::<Vec<_>>()
                        .join("; ")
                }),
        );
        let load_error = combine_load_messages(load_error, icon_theme_error);
        let load_error = combine_load_messages(load_error, language_detection_error);
        let load_error = combine_load_messages(
            load_error,
            layout_load_warning_message(default_layout_state.warnings()),
        );
        let load_error = combine_load_messages(load_error, ssh_load_error);
        let system_notifications_enabled = app_settings.notifications.system;
        let onboarding = ((force_onboarding || !app_settings.general.onboarding_completed)
            && workspace.opened_projects().is_empty())
        .then(|| {
            OnboardingState::new(detect_installed_zed_themes(), app_settings.general.language)
        });
        let mut project_services = HashMap::new();
        let mut project_editor_runtime = ProjectEditorRuntime::default();
        for project in workspace.opened_projects() {
            let selected_terminal_id = project
                .layout
                .tab(&project.selected_tab_id)
                .map(|_| project.selected_tab_id.clone());
            if let Some(project_path) = project.location.local_path() {
                project_editor_runtime.open_project(
                    project.id.clone(),
                    project_path.clone(),
                    selected_terminal_id,
                    app_settings.project_panel.default_open,
                    app_settings.project_panel.width,
                );
                project_services.insert(
                    project.id.clone(),
                    ProjectServices::local(project_path.clone()),
                );
            }
        }

        Self {
            workspace,
            config_paths,
            default_layout_state,
            onboarding,
            palette: PaletteControllerState::new(recent_projects),
            recent_projects_config,
            command_registry,
            load_error,
            presented_error_notification: None,
            project: ProjectControllerState {
                services: project_services,
                project_editor_runtime,
                ..Default::default()
            },
            ssh,
            active_project_file_watcher: None,
            project_file_watching_enabled: true,
            settings: SettingsControllerState::new(keybinding_warning_lines, keybindings_editor),
            performance: performance::PerformanceMonitorState::default(),
            last_opened_layout_file: None,
            last_opened_keybindings_file: None,
            overlays: OverlayControllerState::default(),
            documents: DocumentLifecycleState::default(),
            pending_create_project_request: false,
            pending_open_project_request: false,
            pending_status_notifications: Vec::new(),
            focus_handle: None,
            window_activation_subscription: None,
            terminal: TerminalControllerState::default(),
            sidebar_collapsed: false,
            active_sidebar_resize_drag: None,
            active_split_resize_drag: None,
            active_work_area_resize_drag: None,
            work_area_drop_target: None,
            toast_queue: ToastQueue::default(),
            system_notifier: NoopSystemNotifier,
            system_notifications_enabled,
            ui_text,
            app_settings,
            appearance: AppearanceState::new(theme_runtime),
            icon_theme,
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    pub fn onboarding_language(&self) -> Option<LanguageSetting> {
        self.onboarding
            .as_ref()
            .map(|state| state.selected_language)
    }

    pub fn onboarding_agent(&self) -> Option<BuiltinAgent> {
        self.onboarding.as_ref().map(|state| state.selected_agent)
    }

    pub fn onboarding_layout_kind(&self) -> Option<DefaultLayoutKind> {
        self.onboarding.as_ref().map(|state| state.selected_layout)
    }

    pub fn select_onboarding_language(
        &mut self,
        language: LanguageSetting,
    ) -> Result<(), WorkbenchError> {
        if !self
            .onboarding
            .as_ref()
            .is_some_and(|state| state.step == OnboardingStep::Language)
        {
            return Ok(());
        }
        let language = match language {
            LanguageSetting::System => LanguageSetting::English,
            language => language,
        };
        self.set_language(language)?;
        if let Some(state) = &mut self.onboarding {
            state.selected_language = language;
        }
        Ok(())
    }

    pub fn select_onboarding_layout(&mut self, layout_kind: DefaultLayoutKind) {
        if let Some(state) = &mut self.onboarding
            && state.step == OnboardingStep::Layout
        {
            state.selected_layout = layout_kind;
        }
    }

    pub fn advance_onboarding(&mut self) {
        if let Some(state) = &mut self.onboarding {
            state.step = match state.step {
                OnboardingStep::Language => OnboardingStep::Font,
                OnboardingStep::Font => OnboardingStep::Layout,
                OnboardingStep::Layout => OnboardingStep::Agent,
                OnboardingStep::Agent => OnboardingStep::ZedImport,
                OnboardingStep::ZedImport => OnboardingStep::ZedImport,
            };
        }
    }

    pub fn return_to_onboarding_language(&mut self) {
        if let Some(state) = &mut self.onboarding {
            state.step = OnboardingStep::Language;
        }
    }

    pub fn return_to_onboarding_terminal_font(&mut self) {
        if let Some(state) = &mut self.onboarding {
            state.step = OnboardingStep::Font;
        }
    }

    pub fn return_to_onboarding_layout(&mut self) {
        if let Some(state) = &mut self.onboarding {
            state.step = OnboardingStep::Layout;
        }
    }

    pub fn return_to_onboarding_agent(&mut self) {
        if let Some(state) = &mut self.onboarding {
            state.step = OnboardingStep::Agent;
        }
    }

    pub fn select_onboarding_agent(&mut self, agent: BuiltinAgent) {
        if let Some(state) = &mut self.onboarding
            && state.step == OnboardingStep::Agent
        {
            state.selected_agent = agent;
        }
    }

    pub fn complete_onboarding(&mut self, import_zed_themes: bool) -> Result<(), String> {
        let Some(state) = self.onboarding.clone() else {
            return Ok(());
        };
        if state.step != OnboardingStep::ZedImport {
            return Err(
                "confirm whether to import Zed themes before completing onboarding".to_string(),
            );
        }

        if import_zed_themes && !state.zed_import_completed {
            import_detected_zed_themes_with_policy(
                &state.zed_detection,
                &self.config_paths,
                ZedThemeImportConflictPolicy::SkipExisting,
            )
            .map_err(|error| error.to_string())?;
            if let Some(current_state) = &mut self.onboarding {
                current_state.zed_import_completed = true;
            }
        }

        self.default_layout_state
            .save(DefaultLayoutTemplate::for_onboarding(
                state.selected_layout,
                state.selected_agent,
            ))
            .map_err(|error| error.to_string())?;

        let mut settings = self.app_settings.clone();
        settings.general.onboarding_completed = true;
        save_settings(&self.config_paths, &settings).map_err(|error| error.to_string())?;

        self.app_settings = settings;
        self.onboarding = None;
        Ok(())
    }

    pub fn select_project(&mut self, project_id: &ProjectId) -> Result<(), WorkbenchError> {
        if self.workspace.selected_project_id() != Some(project_id)
            && let Some(WorkItemId::File(document_id)) = self.active_work_item()
        {
            self.queue_focus_change_autosave(document_id);
        }
        self.workspace.select_project(project_id)?;
        if let Some(active) = self.active_work_item() {
            self.apply_active_work_item(&active)?;
        }
        Ok(())
    }

    pub fn project_editor_runtime(&self) -> &ProjectEditorRuntime {
        &self.project.project_editor_runtime
    }

    pub fn project_editor_runtime_mut(&mut self) -> &mut ProjectEditorRuntime {
        &mut self.project.project_editor_runtime
    }

    pub fn active_work_item(&self) -> Option<WorkItemId> {
        let project_id = self.workspace.selected_project_id()?;
        self.project
            .project_editor_runtime
            .workspace()
            .session(project_id)?
            .active_work_item()
            .cloned()
    }

    pub fn select_work_item(&mut self, item: WorkItemId) -> Result<bool, WorkbenchError> {
        if self.active_work_item().as_ref() != Some(&item)
            && let Some(WorkItemId::File(document_id)) = self.active_work_item()
        {
            self.queue_focus_change_autosave(document_id);
        }
        let Some((project_id, terminal_ids)) = self.selected_project_work_item_ids() else {
            return Ok(false);
        };
        let selected = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
            .is_some_and(|session| session.select_work_item(item.clone(), &terminal_ids));
        if !selected {
            return Ok(false);
        }

        self.apply_active_work_item(&item)?;
        Ok(true)
    }

    pub fn select_next_work_item(&mut self) -> Result<Option<WorkItemId>, WorkbenchError> {
        self.select_relative_work_item(true)
    }

    pub fn select_previous_work_item(&mut self) -> Result<Option<WorkItemId>, WorkbenchError> {
        self.select_relative_work_item(false)
    }

    pub fn sidebar_is_collapsed(&self) -> bool {
        self.sidebar_collapsed
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
    }

    pub fn project_sidebar_width(&self) -> f32 {
        self.app_settings.project_panel.project_sidebar_width
    }

    pub fn set_project_sidebar_width(&mut self, width: f32) -> Result<(), WorkbenchError> {
        self.app_settings.project_panel.project_sidebar_width =
            width.clamp(PROJECT_SIDEBAR_MIN_WIDTH, PROJECT_SIDEBAR_MAX_WIDTH);
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn selected_project_panel_width(&self) -> Option<f32> {
        let project_id = self.workspace.selected_project_id()?;
        self.project
            .project_editor_runtime
            .workspace()
            .session(project_id)
            .map(|session| session.project_panel_width())
    }

    pub fn set_project_panel_width(&mut self, width: f32) -> Result<(), WorkbenchError> {
        let width = width.clamp(PROJECT_FILE_PANEL_MIN_WIDTH, PROJECT_FILE_PANEL_MAX_WIDTH);
        self.app_settings.project_panel.width = width;
        if let Some(project_id) = self.workspace.selected_project_id().cloned()
            && let Some(session) = self
                .project
                .project_editor_runtime
                .workspace_mut()
                .session_mut(&project_id)
        {
            session.set_project_panel_width(width);
        }
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn resize_sidebar_from_pointer_delta(
        &mut self,
        side: SidebarSide,
        pointer_delta_x: f32,
    ) -> Option<f32> {
        match side {
            SidebarSide::Left => {
                let width = resize_sidebar_width(
                    side,
                    self.app_settings.project_panel.project_sidebar_width,
                    pointer_delta_x,
                    PROJECT_SIDEBAR_MIN_WIDTH,
                    PROJECT_SIDEBAR_MAX_WIDTH,
                );
                self.app_settings.project_panel.project_sidebar_width = width;
                Some(width)
            }
            SidebarSide::Right => {
                let project_id = self.workspace.selected_project_id()?.clone();
                let session = self
                    .project
                    .project_editor_runtime
                    .workspace_mut()
                    .session_mut(&project_id)?;
                let width = resize_sidebar_width(
                    side,
                    session.project_panel_width(),
                    pointer_delta_x,
                    PROJECT_FILE_PANEL_MIN_WIDTH,
                    PROJECT_FILE_PANEL_MAX_WIDTH,
                );
                session.set_project_panel_width(width);
                Some(width)
            }
        }
    }

    pub fn persist_sidebar_width(&mut self, side: SidebarSide) -> Result<(), WorkbenchError> {
        if side == SidebarSide::Right {
            let Some(width) = self.selected_project_panel_width() else {
                return Ok(());
            };
            self.app_settings.project_panel.width = width;
        }
        save_settings(&self.config_paths, &self.app_settings)?;
        Ok(())
    }

    pub fn theme_runtime(&self) -> Rc<ThemeRuntime> {
        self.appearance.runtime()
    }

    pub fn with_appearance_state(mut self, appearance: AppearanceState) -> Self {
        self.appearance = appearance;
        self
    }

    fn markdown_document_config(&self) -> MarkdownDocumentConfig {
        let runtime = self.appearance.runtime();
        MarkdownDocumentConfig::new(
            Arc::new(runtime.to_markdown_editor_theme(
                self.app_settings.editor.font_size,
                self.app_settings.editor.line_height,
            )),
            markdown_editor_strings_for_language(self.app_settings.general.language),
            self.ui_text,
        )
    }

    pub fn visible_tab_rename_dialog_title(&self) -> Option<String> {
        self.overlays
            .pending_tab_rename
            .as_ref()
            .map(|_| self.ui_text.get(UiTextKey::RenameTabTitle).to_string())
    }

    pub fn pending_tab_rename_value(&self) -> Option<String> {
        self.overlays
            .pending_tab_rename
            .as_ref()
            .map(|rename| rename.value.clone())
    }

    pub fn pending_keybinding_edit_keys(&self) -> Option<Vec<String>> {
        self.overlays
            .pending_keybinding_edit
            .as_ref()
            .map(|edit| edit.keys.clone())
    }

    pub fn confirm_tab_rename_dialog(&mut self, title: &str) -> Result<(), WorkbenchError> {
        let Some(rename) = self.overlays.pending_tab_rename.clone() else {
            return Ok(());
        };

        self.workspace.select_tab(&rename.tab_id)?;
        match self.workspace.rename_selected_tab(title) {
            Ok(()) => {
                self.clear_tab_rename_dialog();
                self.queue_selected_terminal_focus();
                self.load_error = None;
                self.sync_input_owner_state();
                Ok(())
            }
            Err(error) => self.fail_workspace_error(error),
        }
    }

    pub fn cancel_tab_rename_dialog(&mut self) {
        self.clear_tab_rename_dialog();
        self.queue_selected_terminal_focus();
        self.sync_input_owner_state();
    }

    pub fn open_keybinding_edit_dialog(
        &mut self,
        command: CommandId,
    ) -> Result<(), WorkbenchError> {
        let keys = self.settings.keybindings_editor.command_keys(command);
        self.overlays.pending_keybinding_edit = Some(PendingKeybindingEdit {
            command,
            keys,
            has_recorded: false,
            error: None,
        });
        self.overlays.keybinding_recorder_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn confirm_keybinding_edit_dialog(&mut self) -> Result<(), WorkbenchError> {
        let Some(edit) = self.overlays.pending_keybinding_edit.clone() else {
            return Ok(());
        };
        if let Err(error) = self.set_keybinding_command_keys(edit.command, edit.keys) {
            let message = match &error {
                WorkbenchError::KeybindingEdit(error) => {
                    self.localized_keybinding_edit_error(error)
                }
                _ => error.to_string(),
            };
            if let Some(edit) = &mut self.overlays.pending_keybinding_edit {
                edit.error = Some(message);
            }
            return Err(error);
        }
        self.clear_keybinding_edit_dialog();
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn record_keybinding_edit_keystroke(&mut self, keystroke: &Keystroke) -> bool {
        let Some(keybinding) = recorded_keybinding(keystroke) else {
            return false;
        };
        let Some(edit) = &mut self.overlays.pending_keybinding_edit else {
            return false;
        };

        if !edit.has_recorded {
            edit.keys.clear();
            edit.has_recorded = true;
        }
        if !edit.keys.contains(&keybinding) {
            edit.keys.push(keybinding);
        }
        edit.error = None;
        true
    }

    pub fn clear_keybinding_edit_keys(&mut self) {
        if let Some(edit) = &mut self.overlays.pending_keybinding_edit {
            edit.keys.clear();
            edit.has_recorded = true;
            edit.error = None;
        }
    }

    fn localized_keybinding_edit_error(&self, error: &KeybindingEditError) -> String {
        match error {
            KeybindingEditError::ConflictingBindings(keys) => format!(
                "{}: {}",
                self.ui_text.get(UiTextKey::SettingsConflictingKeybinding),
                keys.join(", ")
            ),
            KeybindingEditError::InvalidCommands(commands) => format!(
                "{}: {}",
                self.ui_text.get(UiTextKey::SettingsInvalidCommandId),
                commands.join(", ")
            ),
            KeybindingEditError::Save(message) => message.clone(),
        }
    }

    pub fn cancel_keybinding_edit_dialog(&mut self) {
        self.clear_keybinding_edit_dialog();
        self.sync_input_owner_state();
    }

    pub fn handle_terminal_notification(&mut self, event: NotificationEvent) {
        let project_id = ProjectId::new(event.project_id.clone());
        let agent_status = match event.kind {
            NotificationKind::AgentCompleted => AgentStatus::Completed,
            NotificationKind::AgentFailed => AgentStatus::Failed,
        };
        if let Err(error) = self.workspace.record_agent_status(
            &project_id,
            &event.tab_id,
            &event.pane_id,
            agent_status,
        ) {
            self.load_error = Some(error.to_string());
        }

        let _ = maybe_notify_system(
            &self.system_notifier,
            self.system_notifications_enabled,
            &event,
        );
        self.toast_queue.push(event);
    }

    pub fn focus_notification_target(
        &mut self,
        event: &NotificationEvent,
    ) -> Result<(), WorkbenchError> {
        let project_id = ProjectId::new(event.project_id.clone());
        if self.workspace.project(&project_id).is_none() {
            return self.fail_workspace_error(WorkspaceError::ProjectNotFound(
                project_id.as_str().to_string(),
            ));
        }

        self.select_project(&project_id)?;
        if !self.select_work_item(WorkItemId::Terminal(event.tab_id.clone()))? {
            return self.fail_workspace_error(WorkspaceError::TabNotFound(event.tab_id.clone()));
        }
        if let Err(error) = self.workspace.focus_pane(&event.pane_id) {
            return self.fail_workspace_error(error);
        }

        self.queue_selected_terminal_focus();
        self.load_error = None;
        Ok(())
    }

    pub fn handle_project_tab_click(
        &mut self,
        tab_id: &str,
        click_count: usize,
    ) -> Result<(), WorkbenchError> {
        self.handle_work_item_tab_click(WorkItemId::Terminal(tab_id.to_string()), click_count)
    }

    pub fn handle_work_item_tab_click(
        &mut self,
        work_item: WorkItemId,
        click_count: usize,
    ) -> Result<(), WorkbenchError> {
        if !self.select_work_item(work_item.clone())? {
            return Ok(());
        }
        if click_count >= 2 && matches!(work_item, WorkItemId::Terminal(_)) {
            self.run_command(CommandId::TabRename)?;
        }
        self.load_error = None;
        Ok(())
    }

    pub fn close_project_tab(&mut self, tab_id: &str) -> Result<(), WorkbenchError> {
        self.close_work_item_tab(WorkItemId::Terminal(tab_id.to_string()))
    }

    pub fn close_work_item_tab(&mut self, work_item: WorkItemId) -> Result<(), WorkbenchError> {
        if self.select_work_item(work_item)? {
            self.run_command(CommandId::TabClose)?;
        }
        Ok(())
    }

    pub fn move_work_item_tab(
        &mut self,
        work_item: &WorkItemId,
        to_index: usize,
    ) -> Result<bool, WorkbenchError> {
        let Some((project_id, terminal_ids)) = self.selected_project_work_item_ids() else {
            return Ok(false);
        };
        Ok(self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
            .is_some_and(|session| session.move_work_item(work_item, to_index, &terminal_ids)))
    }

    pub fn close_work_item_tabs(
        &mut self,
        anchor: &WorkItemId,
        scope: WorkbenchTabCloseScope,
        cx: &Context<Self>,
    ) -> Result<(), WorkbenchError> {
        if self.documents.pending_dirty_close.is_some() {
            return Ok(());
        }
        let ordered = self
            .workspace
            .selected_project_id()
            .and_then(|project_id| {
                self.project
                    .project_editor_runtime
                    .workspace()
                    .session(project_id)
            })
            .and_then(|session| session.group_items_containing(anchor))
            .map(ToOwned::to_owned)
            .unwrap_or_default();
        let targets = tab_close_targets(&ordered, anchor, scope);
        let terminal_ids = targets
            .iter()
            .filter_map(|item| match item {
                WorkItemId::Terminal(tab_id) => Some(tab_id.clone()),
                WorkItemId::File(_) => None,
            })
            .collect::<Vec<_>>();
        let file_ids = targets
            .into_iter()
            .filter_map(|item| match item {
                WorkItemId::Terminal(_) => None,
                WorkItemId::File(document_id) => Some(document_id),
            })
            .collect::<Vec<_>>();
        let dirty_file_ids = file_ids
            .iter()
            .filter(|document_id| {
                self.project
                    .project_editor_runtime
                    .document(document_id)
                    .is_some_and(|document| document.read(cx).model().is_dirty())
            })
            .cloned()
            .collect::<Vec<_>>();

        if dirty_file_ids.is_empty() {
            self.close_work_items_immediately(&terminal_ids, &file_ids)?;
        } else {
            self.documents.pending_dirty_close = Some(PendingDirtyClose {
                intent: DirtyCloseIntent::WorkItems {
                    terminal_ids,
                    file_ids,
                },
                dirty_documents: dirty_file_ids,
                running_pane_count: 0,
                saving_documents: HashSet::new(),
            });
        }
        self.sync_input_owner_state();
        Ok(())
    }

    fn close_work_items_immediately(
        &mut self,
        terminal_ids: &[String],
        file_ids: &[crate::ui::editor::DocumentId],
    ) -> Result<(), WorkbenchError> {
        if !terminal_ids.is_empty() {
            self.workspace.close_tabs(terminal_ids)?;
            if let Some((project_id, remaining_terminal_ids)) =
                self.selected_project_work_item_ids()
                && let Some(session) = self
                    .project
                    .project_editor_runtime
                    .workspace_mut()
                    .session_mut(&project_id)
            {
                session.reconcile_work_item_order(&remaining_terminal_ids);
            }
            self.reconcile_active_terminal_with_workspace()?;
        }
        for document_id in file_ids {
            self.close_file_work_item_immediately(document_id)?;
        }
        Ok(())
    }

    pub fn resize_focused_split_from_pointer_delta(
        &mut self,
        direction: SplitDirection,
        delta_x: f32,
        delta_y: f32,
    ) -> Result<Option<f32>, WorkbenchError> {
        let Some(resize) = pointer_resize_for_drag_delta(direction, delta_x, delta_y) else {
            return Ok(None);
        };

        self.workspace
            .resize_focused_split(resize.direction, resize.delta)
            .map(Some)
            .map_err(WorkbenchError::from)
    }

    pub fn visible_toast_titles(&self) -> Vec<String> {
        self.toast_queue.titles()
    }

    pub fn pending_status_notification_titles(&self) -> Vec<String> {
        self.pending_status_notifications
            .iter()
            .map(|item| item.title.clone())
            .collect()
    }

    pub fn dismiss_error_notification(&mut self) {
        self.load_error = None;
    }

    pub fn visible_error_message(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn visible_error_notification_item(&self) -> Option<ToastItem> {
        self.load_error.as_ref().map(|message| ToastItem {
            title: self.ui_text.get(UiTextKey::StatusErrorContext).to_string(),
            context: message.clone(),
            tone: ToastTone::Error,
        })
    }

    pub fn visible_layout_source_message(&self) -> Option<&str> {
        let selected_project_id = self.workspace.selected_project_id()?;
        self.project
            .layout_source_messages
            .get(selected_project_id)
            .map(String::as_str)
    }

    pub fn should_auto_focus_workspace(&self) -> bool {
        self.foreground_input_owner_kind() == InputOwnerKind::Workspace
    }

    pub fn foreground_input_owner_kind(&self) -> InputOwnerKind {
        self.overlays.input_owner_stack.active_owner().active_kind()
    }

    pub fn foreground_input_scope_id(&self) -> Option<String> {
        Some(
            self.overlays
                .input_owner_stack
                .active_owner()
                .active_scope_id()
                .as_str()
                .to_string(),
        )
    }

    pub fn terminal_input_allowed(&self) -> bool {
        self.overlays
            .input_owner_stack
            .active_owner()
            .terminal_input_allowed()
    }

    pub fn take_pending_terminal_focus_for_render(&mut self, pane_id: &str) -> bool {
        if !self.should_auto_focus_workspace() {
            return false;
        }
        let selected = self.workspace.selected_project_id().and_then(|project_id| {
            self.workspace
                .project(project_id)
                .map(|project| (project_id, project.selected_tab_id.as_str()))
        });
        let matches = self
            .terminal
            .pending_terminal_focus
            .as_ref()
            .is_some_and(|target| {
                selected.is_some_and(|(project_id, tab_id)| {
                    &target.project_id == project_id
                        && target.tab_id == tab_id
                        && target.pane_id == pane_id
                })
            });
        if matches {
            self.terminal.pending_terminal_focus = None;
        }
        matches
    }

    pub fn should_use_palette_text_fallback(&self, input_is_focused: bool) -> bool {
        self.palette.active_palette.is_some() && !input_is_focused
    }

    pub fn visible_notification_settings_message(&self) -> &'static str {
        if self.system_notifications_enabled {
            self.ui_text
                .get(UiTextKey::StatusSystemNotificationsEnabled)
        } else {
            self.ui_text
                .get(UiTextKey::StatusSystemNotificationsDisabled)
        }
    }

    pub fn show_settings_file_path_status(&mut self) {
        self.queue_status_notification(
            format!(
                "{}: {}",
                self.ui_text.get(UiTextKey::StatusSettingsFile),
                self.config_paths.settings_file().display()
            ),
            self.ui_text.get(UiTextKey::SettingsGroupAppearance),
        );
        self.load_error = None;
    }

    pub fn show_themes_directory_status(&mut self) {
        self.queue_status_notification(
            format!(
                "{}: {}",
                self.ui_text.get(UiTextKey::StatusThemesDirectory),
                self.config_paths.themes_dir().display()
            ),
            self.ui_text.get(UiTextKey::SettingsGroupAppearance),
        );
        self.load_error = None;
    }

    fn show_layout_file_path_status(&mut self, path: &Path) {
        self.queue_status_notification(
            format!(
                "{}: {}",
                self.ui_text.get(UiTextKey::StatusLayoutFile),
                path.display()
            ),
            self.ui_text.get(UiTextKey::SettingsGroupDefaultLayout),
        );
        self.load_error = None;
    }

    pub fn visible_keybinding_warning_lines(&self) -> Vec<&str> {
        self.settings
            .keybinding_warning_lines
            .iter()
            .map(String::as_str)
            .collect()
    }

    pub fn visible_keybinding_rows(&self) -> Vec<KeybindingRow> {
        self.settings
            .keybindings_editor
            .rows_with_text(&self.ui_text)
    }

    pub fn runtime_keybinding_specs(&self) -> Vec<UiKeybindingSpec> {
        ui_keybinding_specs_from_config(
            self.settings.keybindings_editor.config(),
            &self.command_registry,
        )
    }

    pub fn runtime_command_for_keystroke(&self, keystroke: &Keystroke) -> Option<CommandId> {
        runtime_command_for_keystroke(&self.runtime_keybinding_specs(), keystroke)
    }

    pub(crate) fn set_keybinding_interceptor_subscription(&mut self, subscription: Subscription) {
        self.settings.keybinding_interceptor_subscription = Some(subscription);
    }

    pub fn runtime_command_for_dispatch(&self, keystroke: &Keystroke) -> Option<CommandId> {
        workspace_command_for_keystroke(
            self.foreground_input_owner_kind(),
            keystroke,
            |keystroke| self.runtime_command_for_keystroke(keystroke),
            |keystroke| self.terminal_should_receive_keystroke(keystroke),
        )
    }

    pub fn terminal_should_receive_keystroke(&self, keystroke: &Keystroke) -> bool {
        self.terminal_input_allowed()
            && self.selected_focused_pane_id().is_some()
            && !keystroke.modifiers.platform
            && TerminalKeyEvent::from_gpui_keystroke(keystroke, KeyState::Pressed, false).is_some()
    }

    pub fn set_keybinding_command_keys(
        &mut self,
        command: CommandId,
        keys: Vec<String>,
    ) -> Result<(), WorkbenchError> {
        let previous = self.settings.keybindings_editor.clone();
        self.settings
            .keybindings_editor
            .set_command_keys(command, keys);
        if let Err(error) = self.save_keybindings_editor() {
            self.settings.keybindings_editor = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn delete_keybinding_command_keys(
        &mut self,
        command: CommandId,
    ) -> Result<(), WorkbenchError> {
        self.settings
            .keybindings_editor
            .delete_command_keys(command);
        self.save_keybindings_editor()
    }

    pub fn reset_keybinding_command_keys(
        &mut self,
        command: CommandId,
    ) -> Result<(), WorkbenchError> {
        self.settings.keybindings_editor.reset_command_keys(command);
        self.save_keybindings_editor()
    }

    pub fn visible_empty_workspace_actions(&self) -> Vec<&'static str> {
        let mut actions = EMPTY_WORKSPACE_ACTIONS
            .iter()
            .map(|key| self.ui_text.get(*key))
            .collect::<Vec<_>>();
        actions.insert(2, self.ui_text.get(UiTextKey::RestoreLastSession));
        actions
    }

    pub fn visible_titlebar_info(&self) -> TitlebarInfo {
        let Some(selected_project_id) = self.workspace.selected_project_id() else {
            return TitlebarInfo {
                project_name: self.ui_text.get(UiTextKey::AppName).to_string(),
                compact_path: None,
                git_branch: None,
                git_counters: None,
            };
        };
        let Some(project) = self.workspace.project(selected_project_id) else {
            return TitlebarInfo {
                project_name: self.ui_text.get(UiTextKey::AppName).to_string(),
                compact_path: None,
                git_branch: None,
                git_counters: None,
            };
        };
        let git_status = self.project.project_git_statuses.get(selected_project_id);

        TitlebarInfo {
            project_name: project.layout.project.name.clone(),
            compact_path: Some(compact_path_for_titlebar(&project.location.display_path())),
            git_branch: git_status.and_then(|status| status.branch.clone()),
            git_counters: git_status.and_then(|status| status.summary.compact_counters()),
        }
    }

    pub fn visible_terminal_pane_contexts(&self) -> Vec<TerminalPaneContext> {
        let Some((project_id, project_path, project_title, tab_id, tab_title, layout)) =
            self.selected_tab_layout_clone()
        else {
            return Vec::new();
        };

        let mut contexts = Vec::new();
        let focused_pane_id = self.selected_focused_pane_id().map(ToOwned::to_owned);
        let shell = self.resolved_terminal_shell();
        collect_terminal_pane_contexts(
            &project_id,
            &project_path,
            &project_title,
            &tab_id,
            &tab_title,
            &shell,
            &layout,
            focused_pane_id.as_deref(),
            &self.terminal.terminal_input_gate,
            &mut contexts,
        );
        contexts
    }

    pub fn selected_project_is_empty(&self) -> bool {
        let Some(selected_project_id) = self.workspace.selected_project_id() else {
            return false;
        };
        self.workspace
            .project(selected_project_id)
            .map(|project| project.layout.tabs.is_empty())
            .unwrap_or(false)
    }

    pub fn handle_terminal_pane_started(
        &mut self,
        event: TerminalPaneStartedEvent,
    ) -> Result<(), WorkbenchError> {
        self.workspace.mark_pane_running(
            &ProjectId::new(event.project_id),
            &event.tab_id,
            &event.pane_id,
        )?;
        self.load_error = None;
        Ok(())
    }

    pub fn handle_terminal_pane_exit(
        &mut self,
        event: TerminalPaneExitedEvent,
    ) -> Result<PaneExitCloseOutcome, WorkbenchError> {
        let project_id = ProjectId::new(event.project_id.clone());
        if event.exit_behavior != ProcessExitBehavior::Close {
            self.workspace
                .record_pane_exited(&project_id, &event.tab_id, &event.pane_id)?;
            self.load_error = None;
            return Ok(PaneExitCloseOutcome::PaneKept);
        }

        let outcome =
            self.workspace
                .close_pane_for_exit(&project_id, &event.tab_id, &event.pane_id)?;
        let key = terminal_pane_key(&event.project_id, &event.tab_id, &event.pane_id);
        self.terminal.terminal_panes.remove(&key);
        self.terminal.terminal_pane_subscriptions.remove(&key);

        if self
            .terminal
            .pending_terminal_focus
            .as_ref()
            .is_some_and(|target| {
                target.project_id == project_id
                    && target.tab_id == event.tab_id
                    && target.pane_id == event.pane_id
            })
        {
            self.terminal.pending_terminal_focus = None;
        }
        self.reconcile_project_work_area(&project_id);
        self.reconcile_active_terminal_with_workspace()?;
        self.load_error = None;

        Ok(outcome)
    }

    pub fn focus_visible_terminal_pane(&mut self, pane_id: &str) -> Result<(), WorkbenchError> {
        self.workspace.focus_pane(pane_id)?;
        self.queue_terminal_focus(pane_id);
        Ok(())
    }

    pub fn pending_terminal_focus_pane_id(&self) -> Option<&str> {
        self.terminal
            .pending_terminal_focus
            .as_ref()
            .map(|target| target.pane_id.as_str())
    }

    pub fn pending_editor_focus_document_id(&self) -> Option<&crate::ui::editor::DocumentId> {
        self.project.pending_editor_focus_document_id.as_ref()
    }

    pub fn workspace_arrow_keydown_command(
        key: &str,
        platform: bool,
        control: bool,
        alt: bool,
        shift: bool,
    ) -> Option<CommandId> {
        Self::workspace_arrow_keydown_command_for_owner(
            InputOwnerKind::Workspace,
            key,
            platform,
            control,
            alt,
            shift,
        )
    }

    pub fn workspace_arrow_keydown_command_for_owner(
        owner: InputOwnerKind,
        key: &str,
        platform: bool,
        control: bool,
        alt: bool,
        shift: bool,
    ) -> Option<CommandId> {
        if owner != InputOwnerKind::Workspace {
            return None;
        }
        if !(platform || control) || !alt {
            return None;
        }

        match (key, shift) {
            ("left", false) => Some(CommandId::PaneFocusLeft),
            ("right", false) => Some(CommandId::PaneFocusRight),
            ("up", false) => Some(CommandId::PaneFocusUp),
            ("down", false) => Some(CommandId::PaneFocusDown),
            ("left", true) => Some(CommandId::PaneResizeLeft),
            ("right", true) => Some(CommandId::PaneResizeRight),
            ("up", true) => Some(CommandId::PaneResizeUp),
            ("down", true) => Some(CommandId::PaneResizeDown),
            _ => None,
        }
    }

    pub fn last_opened_layout_file(&self) -> Option<&Path> {
        self.last_opened_layout_file.as_deref()
    }

    pub fn last_opened_keybindings_file(&self) -> Option<&Path> {
        self.last_opened_keybindings_file.as_deref()
    }

    pub fn run_command(&mut self, command_id: CommandId) -> Result<(), WorkbenchError> {
        let availability = command_id.availability_for_context(self.command_context());
        if !availability.enabled {
            self.load_error = Some(
                self.localized_command_disabled_reason(
                    availability
                        .disabled_reason
                        .unwrap_or("Command is unavailable"),
                ),
            );
            return Ok(());
        }

        match command_id {
            CommandId::ProjectCreate => {
                self.request_create_project();
                Ok(())
            }
            CommandId::ProjectOpen => {
                self.request_open_project();
                Ok(())
            }
            CommandId::ProjectOpenSsh => {
                self.open_ssh_project_picker();
                Ok(())
            }
            CommandId::ProjectOpenRecent => {
                self.open_palette(PaletteKind::RecentProject);
                Ok(())
            }
            CommandId::CommandPaletteOpen => {
                self.open_palette(PaletteKind::Command);
                Ok(())
            }
            CommandId::ProjectPalette => {
                self.open_palette(PaletteKind::Project);
                Ok(())
            }
            CommandId::ProjectOpenedPalette => {
                self.open_palette(PaletteKind::OpenedProject);
                Ok(())
            }
            CommandId::ProjectPanelToggle => {
                if let Some(project_id) = self.workspace.selected_project_id().cloned()
                    && let Some(session) = self
                        .project
                        .project_editor_runtime
                        .workspace_mut()
                        .session_mut(&project_id)
                {
                    session.toggle_project_panel();
                }
                Ok(())
            }
            CommandId::ProjectPanelRefresh => {
                if let Some(project_id) = self.workspace.selected_project_id().cloned() {
                    self.queue_project_tree_refresh(project_id);
                }
                Ok(())
            }
            CommandId::GitBranchSwitch => self.open_git_branch_switcher(),
            CommandId::GitDiffOpen => self.open_git_diff_panel(),
            CommandId::FileFind => {
                self.open_palette(PaletteKind::File);
                Ok(())
            }
            CommandId::FileSave => {
                if let Some(WorkItemId::File(document_id)) = self.active_work_item()
                    && !self.documents.pending_document_saves.contains(&document_id)
                {
                    self.documents.pending_document_saves.push(document_id);
                }
                Ok(())
            }
            CommandId::TabPalette => {
                self.open_palette(PaletteKind::Tab);
                Ok(())
            }
            CommandId::TabNext => {
                self.select_next_work_item()?;
                Ok(())
            }
            CommandId::TabPrev => {
                self.select_previous_work_item()?;
                Ok(())
            }
            CommandId::TabClose => self.close_active_work_item(),
            CommandId::PanePalette => {
                self.open_palette(PaletteKind::Pane);
                Ok(())
            }
            CommandId::SettingsOpen => {
                self.open_settings();
                Ok(())
            }
            CommandId::ProjectClose => {
                self.request_close_selected_project()?;
                Ok(())
            }
            CommandId::TabRename => {
                self.open_selected_tab_rename_dialog()?;
                Ok(())
            }
            CommandId::SettingsKeybindings => {
                let path = ensure_keybindings_file(&self.config_paths)?;
                self.last_opened_keybindings_file = Some(path.clone());
                self.queue_status_notification(
                    format!(
                        "{}: {}",
                        self.ui_text.get(UiTextKey::StatusKeybindingsFile),
                        path.display()
                    ),
                    self.ui_text.get(UiTextKey::SettingsGroupKeybindings),
                );
                self.load_error = None;
                Ok(())
            }
            CommandId::SettingsNotifications => {
                self.set_system_notifications_enabled(!self.system_notifications_enabled)?;
                self.queue_status_notification(
                    self.visible_notification_settings_message().to_string(),
                    self.ui_text.get(UiTextKey::SettingsGroupGeneral),
                );
                self.load_error = None;
                Ok(())
            }
            CommandId::TabNew => {
                let tab_id = self.workspace.create_shell_tab()?;
                self.select_work_item(WorkItemId::Terminal(tab_id))?;
                Ok(())
            }
            CommandId::LayoutDefaultEdit => self.open_default_layout_editor(),
            CommandId::LayoutDefaultReset => {
                match self.default_layout_state.reset() {
                    Ok(()) => {
                        self.queue_status_notification(
                            self.ui_text.get(UiTextKey::CommandLayoutDefaultResetTitle),
                            self.default_layout_state.path().display().to_string(),
                        );
                        self.load_error = None;
                    }
                    Err(error) => self.load_error = Some(error.to_string()),
                }
                Ok(())
            }
            CommandId::LayoutDefaultReload => {
                match self.default_layout_state.reload() {
                    Ok(()) => {
                        self.queue_status_notification(
                            self.ui_text.get(UiTextKey::CommandLayoutDefaultReloadTitle),
                            self.default_layout_state.path().display().to_string(),
                        );
                        self.load_error = None;
                    }
                    Err(error) => self.load_error = Some(error.to_string()),
                }
                Ok(())
            }
            CommandId::LayoutProjectEdit => self.open_project_layout_editor(),
            CommandId::LayoutSaveCurrent => {
                let (project_path, layout) = self.selected_project_layout_snapshot()?;
                save_local_layout(&self.config_paths, &project_path, &layout)?;
                Ok(())
            }
            CommandId::LayoutExportProjectConfig => {
                let (project_path, layout) = self.selected_project_layout_snapshot()?;
                export_project_layout(&self.config_paths, &project_path, &layout)?;
                Ok(())
            }
            CommandId::LayoutResetLocalOverride => {
                let (project_path, _layout) = self.selected_project_layout_snapshot()?;
                reset_local_override(&self.config_paths, &project_path)?;
                self.queue_status_notification(
                    self.ui_text
                        .get(UiTextKey::CommandLayoutResetLocalOverrideTitle),
                    project_path.display().to_string(),
                );
                self.load_error = None;
                Ok(())
            }
            CommandId::LayoutOpenFile => {
                let (project_path, _layout) = self.selected_project_layout_snapshot()?;
                let project_layout_file = self.config_paths.project_layout_file(&project_path);
                let local_layout_file = self.config_paths.local_layout_file(&project_path);
                let layout_file = if local_layout_file.exists() {
                    local_layout_file
                } else if project_layout_file.exists() {
                    project_layout_file
                } else {
                    self.default_layout_state.path().to_path_buf()
                };
                platform::reveal_path(&layout_file).map_err(|error| {
                    WorkbenchError::SystemIntegration(format!(
                        "failed to reveal {}: {error}",
                        layout_file.display()
                    ))
                })?;
                self.show_layout_file_path_status(&layout_file);
                self.last_opened_layout_file = Some(layout_file);
                Ok(())
            }
            _ => {
                dispatch_workspace_command(&mut self.workspace, command_id)?;
                self.reconcile_active_terminal_with_workspace()?;
                if should_focus_terminal_after_command(command_id) {
                    self.queue_selected_terminal_focus();
                }
                Ok(())
            }
        }
    }

    pub fn has_pending_project_close(&self) -> bool {
        self.overlays.pending_close_project_id.is_some()
            || self
                .documents
                .pending_dirty_close
                .as_ref()
                .is_some_and(|pending| matches!(pending.intent, DirtyCloseIntent::Project(_)))
    }

    fn request_create_project(&mut self) {
        self.pending_create_project_request = true;
    }

    pub fn take_pending_create_project_request(&mut self) -> bool {
        std::mem::take(&mut self.pending_create_project_request)
    }

    fn handle_pending_create_project_request(&mut self, cx: &mut Context<Self>) {
        if self.take_pending_create_project_request() {
            self.prompt_for_new_project_directory(cx);
        }
    }

    fn request_open_project(&mut self) {
        self.pending_open_project_request = true;
    }

    pub fn take_pending_open_project_request(&mut self) -> bool {
        std::mem::take(&mut self.pending_open_project_request)
    }

    fn handle_pending_open_project_request(&mut self, cx: &mut Context<Self>) {
        if !self.take_pending_open_project_request() {
            return;
        }

        if let Some(project_path) = std::env::var_os("YTTT_OPEN_PROJECT") {
            let _ = self.open_project_path(PathBuf::from(project_path));
        } else {
            self.prompt_for_project_directory(cx);
        }
    }

    pub fn visible_close_project_dialog_text(&self) -> Option<String> {
        if self
            .documents
            .pending_dirty_close
            .as_ref()
            .is_some_and(|pending| matches!(pending.intent, DirtyCloseIntent::Project(_)))
        {
            return self.visible_dirty_close_dialog_text();
        }
        self.overlays.pending_close_project_id.as_ref().map(|_| {
            format!(
                "{}\n{}",
                self.ui_text.get(UiTextKey::CloseProjectTitle),
                self.ui_text.get(UiTextKey::CloseProjectBody)
            )
        })
    }

    pub fn visible_close_project_dialog_actions(&self) -> Vec<String> {
        if self
            .documents
            .pending_dirty_close
            .as_ref()
            .is_some_and(|pending| matches!(pending.intent, DirtyCloseIntent::Project(_)))
        {
            return self.visible_dirty_close_actions();
        }
        if self.overlays.pending_close_project_id.is_some() {
            vec![
                self.ui_text.get(UiTextKey::Cancel).to_string(),
                self.ui_text.get(UiTextKey::CloseProjectAction).to_string(),
            ]
        } else {
            Vec::new()
        }
    }

    pub fn toast_queue(&self) -> &ToastQueue {
        &self.toast_queue
    }

    fn sync_error_notification(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.presented_error_notification == self.load_error {
            return;
        }

        let message = self.load_error.clone();
        self.presented_error_notification = message.clone();
        let Some(message) = message else {
            cx.defer_in(window, |_, window, cx| {
                window.remove_notification::<WorkbenchErrorNotification>(cx);
            });
            return;
        };

        let root = cx.entity();
        let item = ToastItem {
            title: self.ui_text.get(UiTextKey::StatusErrorContext).to_string(),
            context: message.clone(),
            tone: ToastTone::Error,
        };
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let ui_style = appearance.style;
        cx.defer_in(window, move |this, window, cx| {
            if this.load_error.as_deref() != Some(message.as_str()) {
                return;
            }

            let expected_message = message.clone();
            window.push_notification(
                workbench_error_notification(item, theme, ui_style)
                    .id::<WorkbenchErrorNotification>()
                    .on_close(move |_, cx| {
                        root.update(cx, |root, cx| {
                            if root.load_error.as_deref() == Some(expected_message.as_str()) {
                                root.load_error = None;
                                root.presented_error_notification = None;
                                cx.notify();
                            }
                        });
                    }),
                cx,
            );
        });
    }

    fn queue_status_notification(&mut self, title: impl Into<String>, context: impl Into<String>) {
        self.pending_status_notifications.push(ToastItem {
            title: title.into(),
            context: context.into(),
            tone: ToastTone::Success,
        });
    }

    fn flush_pending_status_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let ui_style = appearance.style;
        for item in self.pending_status_notifications.drain(..) {
            window.push_notification(workbench_status_notification(item, theme, ui_style), cx);
        }
    }

    pub fn confirm_pending_project_close(&mut self) -> Result<(), WorkbenchError> {
        let project_id = self
            .overlays
            .pending_close_project_id
            .clone()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let closed = self.workspace.confirm_close_project(&project_id)?;
        self.overlays.pending_close_project_id = None;
        self.cleanup_closed_project(&closed.project_id);
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn cancel_pending_project_close(&mut self) {
        self.overlays.pending_close_project_id = None;
        self.sync_input_owner_state();
    }

    pub fn open_project_path(
        &mut self,
        project_path: impl AsRef<Path>,
    ) -> Result<(), WorkbenchError> {
        match open_project_config(
            &self.config_paths,
            project_path.as_ref(),
            &mut self.default_layout_state,
        ) {
            Ok(opened) => {
                let source_message = layout_source_message(&opened.layout_source);
                let warning_message = if opened.warnings.is_empty() {
                    layout_load_warning_message(self.default_layout_state.warnings())
                } else {
                    layout_load_warning_message(&opened.warnings)
                };
                let opened_path = opened
                    .descriptor
                    .location
                    .local_path()
                    .cloned()
                    .ok_or(WorkbenchError::UnsupportedRemoteProject)?;
                let project_id = self
                    .workspace
                    .open_project(opened.descriptor, opened.layout)?;
                self.project.services.insert(
                    project_id.clone(),
                    ProjectServices::local(opened_path.clone()),
                );
                let selected_terminal_id =
                    self.workspace.project(&project_id).and_then(|project| {
                        project
                            .layout
                            .tab(&project.selected_tab_id)
                            .map(|_| project.selected_tab_id.clone())
                    });
                self.project.project_editor_runtime.open_project(
                    project_id.clone(),
                    opened_path,
                    selected_terminal_id,
                    self.app_settings.project_panel.default_open,
                    self.app_settings.project_panel.width,
                );
                self.queue_selected_terminal_focus();
                self.project
                    .layout_source_messages
                    .insert(project_id, source_message);
                self.recent_projects_config = opened.recent_projects;
                let persistence_error = self.persist_opened_project_paths();
                self.palette.recent_projects =
                    recent_projects_for_palette(&self.recent_projects_config);
                self.load_error = combine_load_messages(warning_message, persistence_error);
                Ok(())
            }
            Err(error) => {
                self.load_error = Some(error.to_string());
                Err(WorkbenchError::from(error))
            }
        }
    }

    pub fn dev_fixture() -> Self {
        let mut workspace = Workspace::new();
        let path = PathBuf::from("/tmp/yttt");
        workspace
            .open_project(
                ProjectDescriptor::new(
                    ProjectId::from_legacy_location(&path.display().to_string()),
                    ProjectLocation::local(path),
                ),
                dev_fixture_layout(),
            )
            .expect("dev fixture layout should be valid");
        let mut root = Self::with_workspace(workspace);
        root.project_file_watching_enabled = false;
        root
    }

    pub fn agent_exit_fixture() -> Self {
        let mut workspace = Workspace::new();
        let path = PathBuf::from("/tmp/yttt-agent-exit");
        workspace
            .open_project(
                ProjectDescriptor::new(
                    ProjectId::from_legacy_location(&path.display().to_string()),
                    ProjectLocation::local(path),
                ),
                agent_exit_fixture_layout(),
            )
            .expect("agent exit fixture layout should be valid");
        let mut root = Self::with_workspace(workspace);
        root.project_file_watching_enabled = false;
        root
    }

    pub fn with_workspace_for_test(workspace: Workspace) -> Self {
        let mut root = Self::with_workspace(workspace);
        root.terminal.start_processes = false;
        root.project_file_watching_enabled = false;
        root
    }

    pub fn with_workspace_for_test_and_config_paths(
        workspace: Workspace,
        config_paths: AppConfigPaths,
    ) -> Self {
        let mut root = Self::with_workspace_and_config_paths(workspace, config_paths, false);
        root.terminal.start_processes = false;
        root.project_file_watching_enabled = false;
        root
    }

    fn with_workspace(workspace: Workspace) -> Self {
        Self::with_workspace_and_config_paths(workspace, AppConfigPaths::for_app(), false)
    }

    fn request_close_selected_project(&mut self) -> Result<CloseProjectDecision, WorkbenchError> {
        let project_id = self
            .workspace
            .selected_project_id()
            .cloned()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        if self
            .project
            .project_editor_runtime
            .documents_for_project(&project_id)
            .next()
            .is_some()
        {
            if !self
                .documents
                .pending_project_close_requests
                .contains(&project_id)
            {
                self.documents
                    .pending_project_close_requests
                    .push(project_id.clone());
            }
            return Ok(CloseProjectDecision::NeedsConfirmation {
                project_id: project_id.clone(),
                running_pane_count: self.project_running_pane_count(&project_id),
            });
        }
        let decision = self.workspace.request_close_project(&project_id)?;
        match &decision {
            CloseProjectDecision::Closed(closed) => {
                self.overlays.pending_close_project_id = None;
                self.cleanup_closed_project(&closed.project_id);
            }
            CloseProjectDecision::NeedsConfirmation { project_id, .. } => {
                self.overlays.pending_close_project_id = Some(project_id.clone());
            }
        }
        self.sync_input_owner_state();

        Ok(decision)
    }

    fn project_running_pane_count(&self, project_id: &ProjectId) -> usize {
        self.workspace
            .project(project_id)
            .map(|project| {
                project
                    .tab_states
                    .iter()
                    .flat_map(|tab| &tab.pane_states)
                    .filter(|pane| {
                        pane.process_state == crate::model::workspace::PaneProcessState::Running
                    })
                    .count()
            })
            .unwrap_or_default()
    }

    fn cleanup_closed_project(&mut self, project_id: &ProjectId) {
        self.project.layout_source_messages.remove(project_id);
        self.project.project_git_statuses.remove(project_id);
        self.project.services.remove(project_id);
        self.project
            .pending_project_tree_loads
            .retain(|(pending_project_id, _)| pending_project_id != project_id);
        self.documents
            .pending_document_saves
            .retain(|document_id| &document_id.project_id != project_id);
        self.documents
            .pending_focus_change_autosaves
            .retain(|document_id| &document_id.project_id != project_id);
        self.documents
            .pending_file_close_requests
            .retain(|document_id| &document_id.project_id != project_id);
        self.documents
            .pending_project_close_requests
            .retain(|pending_project_id| pending_project_id != project_id);
        if self
            .documents
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| &conflict.document_id.project_id == project_id)
        {
            self.documents.pending_file_conflict = None;
        }
        if self.palette.git_branch_project_id.as_ref() == Some(project_id) {
            self.palette.git_branch_generation = self.palette.git_branch_generation.wrapping_add(1);
            self.palette.git_branch_project_id = None;
            self.palette.git_branches.clear();
            self.palette.pending_git_branch_load = None;
            self.palette.pending_git_branch_switch = None;
            if self
                .palette
                .active_palette
                .as_ref()
                .is_some_and(|palette| palette.kind == PaletteKind::GitBranch)
            {
                self.close_palette();
            }
        }
        if self
            .overlays
            .git_diff_panel
            .as_ref()
            .is_some_and(|panel| &panel.project_id == project_id)
        {
            self.overlays.git_diff_generation = self.overlays.git_diff_generation.wrapping_add(1);
            self.overlays.git_diff_panel = None;
            self.overlays.pending_git_diff_load = None;
        }
        self.remove_terminal_panes_for_project(project_id.as_str());
        self.project
            .project_editor_runtime
            .close_project(project_id);
        if self
            .project
            .pending_editor_focus_document_id
            .as_ref()
            .is_some_and(|document_id| &document_id.project_id == project_id)
        {
            self.project.pending_editor_focus_document_id = None;
        }
        let persistence_error = self.persist_opened_project_paths();
        self.load_error = combine_load_messages(self.load_error.take(), persistence_error);
    }

    fn close_active_work_item(&mut self) -> Result<(), WorkbenchError> {
        let Some(active) = self.active_work_item() else {
            return Ok(());
        };
        match active {
            WorkItemId::File(document_id) => {
                if self
                    .project
                    .project_editor_runtime
                    .document(&document_id)
                    .is_some()
                {
                    if !self
                        .documents
                        .pending_file_close_requests
                        .contains(&document_id)
                    {
                        self.documents.pending_file_close_requests.push(document_id);
                    }
                    Ok(())
                } else {
                    self.close_file_work_item_immediately(&document_id)
                }
            }
            WorkItemId::Terminal(tab_id) => {
                self.workspace.select_tab(&tab_id)?;
                dispatch_workspace_command(&mut self.workspace, CommandId::TabClose)?;
                let next = if let Some((project_id, terminal_ids)) =
                    self.selected_project_work_item_ids()
                {
                    self.project
                        .project_editor_runtime
                        .workspace_mut()
                        .session_mut(&project_id)
                        .and_then(|session| {
                            session.reconcile_work_area(&terminal_ids);
                            session.active_work_item().cloned()
                        })
                } else {
                    None
                };
                if let Some(next) = next {
                    self.apply_active_work_item(&next)?;
                } else {
                    self.sync_input_owner_state();
                }
                Ok(())
            }
        }
    }

    fn close_file_work_item_immediately(
        &mut self,
        document_id: &crate::ui::editor::DocumentId,
    ) -> Result<(), WorkbenchError> {
        let Some((project_id, terminal_ids)) = self.selected_project_work_item_ids() else {
            return Ok(());
        };
        let next = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
            .and_then(|session| session.close_file(document_id, &terminal_ids));
        self.project
            .project_editor_runtime
            .remove_document(document_id);
        if self
            .documents
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| &conflict.document_id == document_id)
        {
            self.documents.pending_file_conflict = None;
        }
        if let Some(next) = next {
            self.apply_active_work_item(&next)?;
        } else {
            self.sync_input_owner_state();
        }
        Ok(())
    }

    fn open_selected_tab_rename_dialog(&mut self) -> Result<(), WorkbenchError> {
        let project_id = self
            .workspace
            .selected_project_id()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let project = self
            .workspace
            .project(project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        let tab_id = project.selected_tab_id.clone();
        let tab = project
            .layout
            .tabs
            .iter()
            .find(|tab| tab.id == tab_id)
            .ok_or_else(|| WorkspaceError::TabNotFound(tab_id.clone()))?;
        let value = tab.title.clone();

        self.close_palette();
        self.overlays.pending_tab_rename = Some(PendingTabRename { tab_id, value });
        self.reset_tab_rename_input();
        self.overlays.tab_rename_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    fn clear_tab_rename_dialog(&mut self) {
        self.overlays.pending_tab_rename = None;
        self.reset_tab_rename_input();
    }

    fn clear_keybinding_edit_dialog(&mut self) {
        self.overlays.pending_keybinding_edit = None;
        self.overlays.keybinding_recorder_needs_focus = false;
    }

    fn remove_terminal_panes_for_project(&mut self, project_id: &str) {
        let prefix = format!("{project_id}:");
        self.terminal
            .terminal_panes
            .retain(|key, _pane| !key.starts_with(&prefix));
        self.terminal
            .terminal_pane_subscriptions
            .retain(|key, _subscription| !key.starts_with(&prefix));
    }

    fn selected_project_layout_snapshot(&self) -> Result<(PathBuf, ProjectLayout), WorkbenchError> {
        let project_id = self
            .workspace
            .selected_project_id()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let project = self
            .workspace
            .project(project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;

        let path = project
            .location
            .local_path()
            .cloned()
            .ok_or(WorkbenchError::UnsupportedRemoteProject)?;
        Ok((path, project.layout.clone()))
    }

    fn set_layout_toml_editor_error(&mut self, source: &'static str, error: String) {
        if let Some(session) = &mut self.overlays.layout_toml_editor {
            let editor = session.editor_mut();
            editor.set_error(error.clone());
            editor.set_diagnostics(vec![EditorDiagnostic::new(
                EditorDiagnosticSeverity::Error,
                source,
                error,
            )]);
        }
    }

    fn fail_workspace_error<T>(&mut self, error: WorkspaceError) -> Result<T, WorkbenchError> {
        self.load_error = Some(error.to_string());
        Err(error.into())
    }

    fn workbench_focus_handle(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        if let Some(focus_handle) = &self.focus_handle {
            return focus_handle.clone();
        }

        let focus_handle = cx.focus_handle();
        self.focus_handle = Some(focus_handle.clone());
        focus_handle
    }

    pub(crate) fn register_window_activation_observer(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.window_activation_subscription =
            Some(cx.observe_window_activation(window, Self::on_window_activation_changed));
    }

    fn on_window_activation_changed(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if window.is_window_active() && self.queue_default_active_work_item_focus(cx) {
            window.blur();
            cx.notify();
        }
    }

    fn reset_tab_rename_input(&mut self) {
        self.overlays.tab_rename_input = None;
        self.overlays.tab_rename_input_subscription = None;
        self.overlays.tab_rename_input_needs_focus = false;
    }

    fn reset_settings_search_input(&mut self) {
        self.settings.settings_search_input = None;
        self.settings.settings_search_input_subscription = None;
        self.settings.settings_search_input_needs_focus = false;
        self.settings.settings_language_select = None;
        self.settings.settings_language_select_subscription = None;
        self.settings.settings_shell_select = None;
        self.settings.settings_shell_select_subscription = None;
        self.settings.settings_new_tab_command_input = None;
        self.settings.settings_ui_theme_select = None;
        self.settings.settings_ui_theme_select_subscription = None;
        self.settings.settings_ui_font_family_select = None;
        self.settings.settings_ui_font_family_select_subscription = None;
        self.settings.settings_icon_theme_select = None;
        self.settings.settings_icon_theme_select_subscription = None;
        self.settings.settings_terminal_theme_select = None;
        self.settings.settings_terminal_theme_select_subscription = None;
        self.settings.settings_terminal_cursor_shape_select = None;
        self.settings
            .settings_terminal_cursor_shape_select_subscription = None;
        self.settings.settings_terminal_osc52_policy_select = None;
        self.settings
            .settings_terminal_osc52_policy_select_subscription = None;
        self.settings.settings_editor_language_select = None;
        self.settings.settings_editor_language_select_subscription = None;
        self.settings.settings_font_family_select = None;
        self.settings.settings_font_family_select_subscription = None;
        self.settings.settings_editor_font_family_select = None;
        self.settings
            .settings_editor_font_family_select_subscription = None;
        self.settings.settings_editor_autosave_select = None;
        self.settings.settings_editor_autosave_select_subscription = None;
        self.settings.settings_number_inputs.clear();
        self.settings.settings_number_input_subscriptions.clear();
    }

    fn reset_layout_toml_input(&mut self) {
        self.overlays.layout_toml_input = None;
        self.overlays.layout_toml_input_subscription = None;
        self.overlays.layout_toml_input_needs_focus = false;
    }

    fn queue_terminal_focus(&mut self, pane_id: &str) {
        let Some(project_id) = self.workspace.selected_project_id().cloned() else {
            self.terminal.pending_terminal_focus = None;
            return;
        };
        let Some(tab_id) = self
            .workspace
            .project(&project_id)
            .map(|project| project.selected_tab_id.clone())
        else {
            self.terminal.pending_terminal_focus = None;
            return;
        };
        self.queue_terminal_focus_target(project_id, tab_id, pane_id.to_string());
    }

    fn queue_terminal_focus_target(
        &mut self,
        project_id: ProjectId,
        tab_id: String,
        pane_id: String,
    ) {
        self.terminal.pending_terminal_focus = Some(TerminalPaneTarget {
            project_id,
            tab_id,
            pane_id,
        });
    }

    fn queue_selected_terminal_focus(&mut self) {
        if let Some(pane_id) = self.selected_focused_pane_id().map(ToOwned::to_owned) {
            self.queue_terminal_focus(&pane_id);
        }
    }

    fn selected_project_tree_is_editing(&self, cx: &App) -> bool {
        self.workspace
            .selected_project_id()
            .and_then(|project_id| self.project.project_editor_runtime.tree(project_id))
            .is_some_and(|tree| tree.read(cx).is_editing())
    }

    fn queue_default_active_work_item_focus(&mut self, cx: &App) -> bool {
        if self.selected_project_tree_is_editing(cx) {
            return false;
        }
        let Some(item) = self.active_work_item() else {
            return false;
        };
        let owner_accepts_focus = matches!(
            (&item, self.foreground_input_owner_kind()),
            (WorkItemId::Terminal(_), InputOwnerKind::Workspace)
                | (WorkItemId::File(_), InputOwnerKind::Editor)
        );
        owner_accepts_focus && self.queue_work_item_focus(&item)
    }

    fn queue_work_item_focus(&mut self, item: &WorkItemId) -> bool {
        match item {
            WorkItemId::Terminal(tab_id) => {
                self.project.pending_editor_focus_document_id = None;
                let pane_id = self
                    .workspace
                    .selected_project_id()
                    .and_then(|project_id| self.workspace.project(project_id))
                    .and_then(|project| project.tab_state(tab_id))
                    .and_then(|tab| tab.focused_pane_id.clone());
                let Some(pane_id) = pane_id else {
                    self.terminal.pending_terminal_focus = None;
                    return false;
                };
                self.queue_terminal_focus(&pane_id);
                true
            }
            WorkItemId::File(document_id) => {
                self.terminal.pending_terminal_focus = None;
                self.project.pending_editor_focus_document_id = Some(document_id.clone());
                true
            }
        }
    }

    fn selected_project_work_item_ids(&self) -> Option<(ProjectId, Vec<String>)> {
        let project_id = self.workspace.selected_project_id()?.clone();
        let terminal_ids = self
            .workspace
            .project(&project_id)?
            .layout
            .tabs
            .iter()
            .map(|tab| tab.id.clone())
            .collect();
        Some((project_id, terminal_ids))
    }

    fn select_relative_work_item(
        &mut self,
        forward: bool,
    ) -> Result<Option<WorkItemId>, WorkbenchError> {
        let Some((project_id, terminal_ids)) = self.selected_project_work_item_ids() else {
            return Ok(None);
        };
        let next = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
            .and_then(|session| {
                if forward {
                    session.select_next(&terminal_ids)
                } else {
                    session.select_previous(&terminal_ids)
                }
            });
        if let Some(item) = &next {
            self.apply_active_work_item(item)?;
        }
        Ok(next)
    }

    fn apply_active_work_item(&mut self, item: &WorkItemId) -> Result<(), WorkbenchError> {
        if let WorkItemId::Terminal(tab_id) = item {
            self.workspace.select_tab(tab_id)?;
        }
        self.queue_work_item_focus(item);
        self.sync_input_owner_state();
        Ok(())
    }

    fn reconcile_active_terminal_with_workspace(&mut self) -> Result<(), WorkbenchError> {
        if !matches!(self.active_work_item(), Some(WorkItemId::Terminal(_))) {
            return Ok(());
        }
        let Some((project_id, terminal_ids)) = self.selected_project_work_item_ids() else {
            return Ok(());
        };
        let next = self
            .project
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
            .and_then(|session| {
                session.reconcile_work_area(&terminal_ids);
                session.active_work_item().cloned()
            });
        if let Some(next) = next {
            self.apply_active_work_item(&next)?;
        } else {
            self.terminal.pending_terminal_focus = None;
            self.project.pending_editor_focus_document_id = None;
            self.sync_input_owner_state();
        }
        Ok(())
    }

    fn command_context(&self) -> CommandContext {
        CommandContext {
            has_selected_project: self.workspace.selected_project_id().is_some(),
            active_surface: match self.active_work_item() {
                Some(WorkItemId::Terminal(_)) => ActiveSurface::Terminal,
                Some(WorkItemId::File(_)) => ActiveSurface::File,
                None => ActiveSurface::None,
            },
        }
    }

    fn localized_command_disabled_reason(&self, reason: &str) -> String {
        let key = match reason {
            "Open a project first" => UiTextKey::CommandDisabledOpenProjectFirst,
            "Focus a project file first" => UiTextKey::CommandDisabledFocusProjectFileFirst,
            "Open a terminal or file first" => UiTextKey::CommandDisabledOpenWorkItemFirst,
            "Switch to a terminal tab first" => UiTextKey::CommandDisabledSwitchTerminalFirst,
            _ => UiTextKey::CommandUnavailable,
        };
        self.ui_text.get(key).to_string()
    }

    fn apply_project_git_status(
        &mut self,
        project_id: &ProjectId,
        status: Option<crate::runtime::git_status::ProjectGitStatus>,
    ) {
        if let Some(status) = status {
            self.project
                .project_git_statuses
                .insert(project_id.clone(), status);
        } else {
            self.project.project_git_statuses.remove(project_id);
        }
    }

    fn current_input_owner_registration(&self) -> InputOwnerRegistration {
        if !self.ssh.pending_host_keys.is_empty() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.ssh_host_key"),
            )
        } else if self.overlays.pending_keybinding_edit.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::KeybindingRecorder,
                InputScopeId::new("recorder.keybinding"),
            )
        } else if self.documents.pending_file_conflict.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.file_conflict"),
            )
        } else if self.documents.pending_dirty_close.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.dirty_close"),
            )
        } else if self.overlays.pending_tab_rename.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.rename_tab"),
            )
        } else if self.overlays.pending_close_project_id.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.close_project"),
            )
        } else if self.overlays.layout_toml_editor.is_some() {
            let scope = self
                .overlays
                .layout_toml_editor
                .as_ref()
                .map(|session| session.target().input_scope_id())
                .unwrap_or("editor.project_layout");
            InputOwnerRegistration::blocking(InputOwnerKind::Dialog, InputScopeId::new(scope))
        } else if self.overlays.git_diff_panel.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("overlay.git_diff"),
            )
        } else if self.settings.zed_theme_import_dialog.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.zed_theme_import"),
            )
        } else if self.ssh.project_picker.open {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.ssh_project_picker"),
            )
        } else if self.ssh.manager_open {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.ssh_connections"),
            )
        } else if self.settings.settings_page.is_open {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Settings,
                InputScopeId::new("settings"),
            )
        } else if let Some(active_palette) = &self.palette.active_palette {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Palette,
                InputScopeId::new(palette_input_scope_id(active_palette.kind)),
            )
        } else if let Some(WorkItemId::File(document_id)) = self.active_work_item() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Editor,
                InputScopeId::new(format!(
                    "editor.project_file:{}:{}",
                    document_id.project_id.as_str(),
                    document_id.canonical_path.display()
                )),
            )
        } else {
            InputOwnerRegistration::workspace()
        }
    }

    fn sync_input_owner_state(&mut self) {
        self.overlays.input_owner_stack.clear();
        let registration = self.current_input_owner_registration();
        if registration.kind() != InputOwnerKind::Workspace {
            self.overlays.input_owner_stack.push_owner(registration);
        }
        self.terminal
            .terminal_input_gate
            .sync_from_snapshot(&self.overlays.input_owner_stack.active_owner());
    }

    fn selected_focused_pane_id(&self) -> Option<&str> {
        let project_id = self.workspace.selected_project_id()?;
        let project = self.workspace.project(project_id)?;
        project
            .tab_state(&project.selected_tab_id)
            .and_then(|tab| tab.focused_pane_id.as_deref())
    }

    fn tab_rename_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let rename = self.overlays.pending_tab_rename.as_ref()?;
        let input = if let Some(input) = &self.overlays.tab_rename_input {
            input.clone()
        } else {
            let value = rename.value.clone();
            let placeholder = self.ui_text.get(UiTextKey::RenameTabTitle);
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(value)
            });
            let subscription = cx.subscribe_in(&input, window, Self::on_tab_rename_input_event);
            self.overlays.tab_rename_input = Some(input.clone());
            self.overlays.tab_rename_input_subscription = Some(subscription);
            input
        };

        if self.overlays.tab_rename_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.overlays.tab_rename_input_needs_focus = false;
        }

        Some(input)
    }

    fn on_tab_rename_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if let Some(rename) = &mut self.overlays.pending_tab_rename {
                    rename.value = input.read(cx).value().to_string();
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                let _ = self.confirm_tab_rename_dialog_from_input(cx);
                cx.notify();
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn confirm_tab_rename_dialog_from_input(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        let value = self
            .overlays
            .tab_rename_input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .or_else(|| self.pending_tab_rename_value())
            .unwrap_or_default();

        self.confirm_tab_rename_dialog(&value)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WorkbenchError {
    #[error("{0}")]
    Command(#[from] CommandDispatchError),
    #[error("{0}")]
    Workspace(#[from] WorkspaceError),
    #[error("{0}")]
    CloseProject(#[from] CloseProjectError),
    #[error("{0}")]
    ProjectOpen(Box<ProjectOpenError>),
    #[error("{0}")]
    Keybindings(Box<KeybindingsLoadError>),
    #[error("{0}")]
    SettingsSave(Box<SettingsSaveError>),
    #[error("{0}")]
    KeybindingEdit(Box<KeybindingEditError>),
    #[error("{0}")]
    LayoutTomlEditor(String),
    #[error("{0}")]
    SystemIntegration(String),
    #[error("{0}")]
    RemoteProject(String),
    #[error("remote projects are not available in this operation")]
    UnsupportedRemoteProject,
}

impl From<ProjectOpenError> for WorkbenchError {
    fn from(error: ProjectOpenError) -> Self {
        Self::ProjectOpen(Box::new(error))
    }
}

impl From<KeybindingsLoadError> for WorkbenchError {
    fn from(error: KeybindingsLoadError) -> Self {
        Self::Keybindings(Box::new(error))
    }
}

impl From<SettingsSaveError> for WorkbenchError {
    fn from(error: SettingsSaveError) -> Self {
        Self::SettingsSave(Box::new(error))
    }
}

impl From<KeybindingEditError> for WorkbenchError {
    fn from(error: KeybindingEditError) -> Self {
        Self::KeybindingEdit(Box::new(error))
    }
}

impl Default for WorkbenchView {
    fn default() -> Self {
        Self::new()
    }
}

fn dev_fixture_layout() -> ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "yttt"
        default_tab = "dev"

        [[tabs]]
        id = "dev"
        title = "Dev"

        [tabs.layout]
        type = "split"
        direction = "horizontal"
        ratio = 0.65
        left = { type = "pane", id = "server", title = "server", command = "$SHELL" }
        right = { type = "pane", id = "shell", title = "shell", command = "$SHELL" }

        [[tabs]]
        id = "agent"
        title = "Agent"
        layout = { type = "pane", id = "codex", title = "Codex", command = "codex", kind = "agent", notify_on_exit = true }
    "#,
    )
    .expect("static dev fixture TOML should parse")
}

fn agent_exit_fixture_layout() -> ProjectLayout {
    toml::from_str(
        r#"
        [project]
        name = "yttt-agent-exit"
        default_tab = "agent"

        [[tabs]]
        id = "agent"
        title = "Agent"
        layout = { type = "pane", id = "codex", title = "Codex", command = "sh -c 'sleep 1; exit 0'", kind = "agent", notify_on_exit = true }
    "#,
    )
    .expect("static agent exit fixture TOML should parse")
}
