use gpui::{
    AnyElement, ClickEvent, Context, Div, Entity, FocusHandle, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, PathPromptOptions, Pixels, Point, Render,
    ScrollHandle, SharedString, Subscription, Window, div, prelude::*, px, relative, rgba,
};
use gpui_component::{
    Disableable as _, IconName, IndexPath, Root as ComponentRoot, Sizable as _,
    Theme as ComponentTheme, WindowExt as _,
    button::Button,
    input::{Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    scroll::ScrollableElement as _,
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use yttt_terminal::input::keystroke_to_bytes;

mod action_handlers;
mod dialogs;
mod document_lifecycle;
mod helpers;
pub mod layout_editor;
mod layout_editor_controller;
#[cfg(test)]
mod non_destructive_tests;
mod palette;
mod project_files;
mod render;
mod resize;
mod settings;
pub mod shell;
mod surface;
use dialogs::*;
use helpers::*;
use render::{push_component_notification, split_child};
use settings::{settings_button, settings_overlay};

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

type SettingsStringSelectState = SelectState<SearchableVec<String>>;

const TERMINAL_THEME_FOLLOW_UI: &str = "Follow UI theme";
const ICON_THEME_BUILTIN: &str = "Built-in";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SettingsNumberField {
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
        default_layout::{DefaultLayoutState, DefaultLayoutTemplate, LayoutLoadWarning},
        keybindings::{
            KeybindingLoadWarning, KeybindingsLoadError, ensure_keybindings_file, load_keybindings,
        },
        layout_loader::{
            LayoutSource, PersonalLayout, ProjectOpenError, RecentProjectsConfig,
            export_project_layout, load_recent_projects, open_project_config,
            parse_personal_layout, reset_local_override, save_local_layout,
        },
        paths::AppConfigPaths,
        settings::{
            AppSettings, EditorAutosave, LanguageSetting, SettingsLoadWarning, SettingsSaveError,
            detect_shell_candidates, load_or_create_settings, resolve_default_shell, save_settings,
        },
        theme::{ThemeLoadWarning, ThemeStore, load_theme_store},
    },
    model::{
        ids::ProjectId,
        layout::{LayoutNode, PaneConfig, ProjectLayout, SplitDirection},
        workspace::{
            AgentStatus, CloseProjectDecision, CloseProjectError, PaneExitCloseOutcome, Workspace,
            WorkspaceError,
        },
    },
    palette::{
        ActivePalette, CommandPaletteContext, PaletteItem, PaletteKind, RecentProject,
        TabPaletteSnapshot, command_palette_items_with_text, decode_tab_palette_item_id,
        pane_palette_items_with_text, project_palette_items_with_text, tab_palette_items_with_text,
        unified_tab_palette_items,
    },
    runtime::{
        git_status::{ProjectGitStatus, read_project_git_status},
        notification::{
            NoopSystemNotifier, NotificationEvent, NotificationKind, maybe_notify_system,
        },
    },
    ui::{
        components::{
            ActionEmphasis, workbench_action_button, workbench_agent_notification,
            workbench_inline_notification, workbench_keybinding_badge, workbench_settings_row,
            workbench_status_notification, workbench_switch,
        },
        editor::{
            CodeEditorConfig, CodeEditorLanguageMode, CodeEditorState, CurrentDiskState,
            EditorAppearance, EditorDiagnostic, EditorDiagnosticSeverity, EditorLanguageCatalog,
            EditorLanguageId, LoadedProjectFile, ProjectEditorDocument, ProjectEditorDocumentEvent,
            ProjectEditorModel, ProjectEditorRuntime, ProjectEditorSaveState, ProjectFileIoError,
            ProjectFileLoadRequest, SaveMode, SaveProjectFileOutcome, SaveRequest, WorkItemId,
            code_editor_input_state, project_relative_path, read_project_file, save_project_file,
        },
        i18n::{Locale, UiText, UiTextKey},
        interaction::actions::{
            FileSave, LayoutDefaultEdit, LayoutDefaultReload, LayoutDefaultReset,
            LayoutExportProjectConfig, LayoutOpenFile, LayoutProjectEdit, LayoutResetLocalOverride,
            LayoutSaveCurrent, OpenCommandPalette, OpenPanePalette, OpenProject,
            OpenProjectPalette, OpenTabPalette, PaletteCancel, PaletteConfirm, PaletteSelectNext,
            PaletteSelectPrev, PaneClose, PaneFocusDown, PaneFocusLeft, PaneFocusRight,
            PaneFocusUp, PaneRename, PaneResizeDown, PaneResizeLeft, PaneResizeRight, PaneResizeUp,
            PaneSplitHorizontal, PaneSplitVertical, ProjectClose, SettingsKeybindings,
            SettingsNotifications, SettingsOpen, TabClose, TabNew, TabNext, TabPrev, TabRename,
            UiKeybindingSpec, WORKSPACE_CONTEXT, runtime_command_for_keystroke,
            ui_keybinding_specs_from_config,
        },
        interaction::input_owner::{
            InputOwnerKind, InputOwnerRegistration, InputOwnerStack, InputScopeId,
            TerminalInputGate,
        },
        interaction::key_dispatch::workspace_command_for_keystroke,
        interaction::overlay::capture_overlay_input,
        notifications::{ToastItem, ToastQueue, ToastTone, toast_item_for_event},
        palette::palette_overlay,
        palette::surface::palette_input_placeholder,
        primitives::{
            button::{YtttButtonVariant, yttt_button},
            dialog::yttt_dialog_style,
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
            DirectoryLoadRequest, DirectorySnapshot, ProjectTreeFsError, ProjectTreeLoadState,
            ProjectTreeRenderSnapshot, ProjectTreeRenderText, ProjectTreeView,
            ProjectTreeViewEvent, scan_project_directory,
        },
        settings::font_options::{
            font_family_option_for_setting, font_family_options_from_system,
            font_family_setting_from_option, terminal_font_family_option_for_setting,
            terminal_font_family_options_from_system, terminal_font_family_setting_from_option,
        },
        settings::keybinding_display::primary_display_keybinding_for_current_platform,
        settings::keybindings::{KeybindingEditError, KeybindingRow, KeybindingsEditorState},
        settings::{SettingsGroupId, SettingsPageState, SettingsPanelStyle, settings_panel_style},
        terminal::pane::{
            TerminalPaneContext, TerminalPaneEvent, TerminalPaneExitedEvent, TerminalPaneView,
        },
        theme::icons::{
            IconTheme, available_icon_theme_names as load_icon_theme_names, icon_for_visual,
            load_icon_theme,
        },
        theme::{ThemeRuntime, WorkbenchTheme},
        workbench::layout_editor::{
            LayoutEditorSession, LayoutEditorTarget, ProjectLayoutEditorFormat,
            write_layout_file_atomic,
        },
        workbench::shell::sidebar::project_sidebar,
        workbench::shell::split_view::{pointer_resize_for_drag_delta, split_child_basis},
        workbench::shell::tabs::{
            FileTabSnapshot, ProjectTabsToolbar, WorkbenchTabItem, project_tabs, visible_tab_items,
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
    active_palette: Option<ActivePalette>,
    command_registry: CommandRegistry,
    recent_projects: Vec<RecentProject>,
    load_error: Option<String>,
    layout_source_messages: HashMap<ProjectId, String>,
    keybinding_warning_lines: Vec<String>,
    keybindings_editor: KeybindingsEditorState,
    last_opened_layout_file: Option<PathBuf>,
    last_opened_keybindings_file: Option<PathBuf>,
    pending_close_project_id: Option<ProjectId>,
    pending_document_saves: Vec<crate::ui::editor::DocumentId>,
    pending_focus_change_autosaves: Vec<crate::ui::editor::DocumentId>,
    pending_file_close_requests: Vec<crate::ui::editor::DocumentId>,
    pending_project_close_requests: Vec<ProjectId>,
    pending_file_conflict: Option<PendingFileConflict>,
    pending_dirty_close: Option<PendingDirtyClose>,
    allow_window_close_once: bool,
    pending_open_project_request: bool,
    pending_status_notifications: Vec<ToastItem>,
    pending_tab_rename: Option<PendingTabRename>,
    pending_keybinding_edit: Option<PendingKeybindingEdit>,
    keybinding_interceptor_subscription: Option<Subscription>,
    focus_handle: Option<FocusHandle>,
    input_owner_stack: InputOwnerStack,
    terminal_input_gate: TerminalInputGate,
    palette_input: Option<Entity<InputState>>,
    palette_input_subscription: Option<Subscription>,
    palette_input_needs_focus: bool,
    tab_rename_input: Option<Entity<InputState>>,
    tab_rename_input_subscription: Option<Subscription>,
    tab_rename_input_needs_focus: bool,
    keybinding_edit_input: Option<Entity<InputState>>,
    keybinding_edit_input_subscription: Option<Subscription>,
    keybinding_edit_input_needs_focus: bool,
    settings_search_input: Option<Entity<InputState>>,
    settings_search_input_subscription: Option<Subscription>,
    settings_search_input_needs_focus: bool,
    settings_language_select: Option<Entity<SettingsStringSelectState>>,
    settings_language_select_subscription: Option<Subscription>,
    settings_shell_select: Option<Entity<SettingsStringSelectState>>,
    settings_shell_select_subscription: Option<Subscription>,
    settings_ui_theme_select: Option<Entity<SettingsStringSelectState>>,
    settings_ui_theme_select_subscription: Option<Subscription>,
    settings_icon_theme_select: Option<Entity<SettingsStringSelectState>>,
    settings_icon_theme_select_subscription: Option<Subscription>,
    settings_terminal_theme_select: Option<Entity<SettingsStringSelectState>>,
    settings_terminal_theme_select_subscription: Option<Subscription>,
    settings_editor_language_select: Option<Entity<SettingsStringSelectState>>,
    settings_editor_language_select_subscription: Option<Subscription>,
    settings_font_family_select: Option<Entity<SettingsStringSelectState>>,
    settings_font_family_select_subscription: Option<Subscription>,
    settings_editor_font_family_select: Option<Entity<SettingsStringSelectState>>,
    settings_editor_font_family_select_subscription: Option<Subscription>,
    settings_editor_autosave_select: Option<Entity<SettingsStringSelectState>>,
    settings_editor_autosave_select_subscription: Option<Subscription>,
    settings_number_inputs: HashMap<SettingsNumberField, Entity<InputState>>,
    settings_number_input_subscriptions: HashMap<SettingsNumberField, Vec<Subscription>>,
    layout_toml_editor: Option<LayoutEditorSession>,
    layout_toml_input: Option<Entity<InputState>>,
    layout_toml_input_subscription: Option<Subscription>,
    layout_toml_input_needs_focus: bool,
    palette_scroll_handle: ScrollHandle,
    sidebar_collapsed: bool,
    active_sidebar_resize_drag: Option<ActiveSidebarResizeDrag>,
    active_split_resize_drag: Option<ActiveSplitResizeDrag>,
    pending_terminal_focus_pane_id: Option<String>,
    pending_editor_focus_document_id: Option<crate::ui::editor::DocumentId>,
    terminal_panes: HashMap<String, Entity<TerminalPaneView>>,
    terminal_pane_subscriptions: HashMap<String, Subscription>,
    project_editor_runtime: ProjectEditorRuntime,
    pending_project_tree_loads: Vec<(ProjectId, DirectoryLoadRequest)>,
    project_git_statuses: HashMap<ProjectId, ProjectGitStatus>,
    toast_queue: ToastQueue,
    system_notifier: NoopSystemNotifier,
    system_notifications_enabled: bool,
    ui_text: UiText,
    app_settings: AppSettings,
    theme_runtime: ThemeRuntime,
    icon_theme: IconTheme,
    settings_page: SettingsPageState,
}

const EMPTY_WORKSPACE_ACTIONS: [UiTextKey; 3] = [
    UiTextKey::OpenDirectory,
    UiTextKey::OpenRecent,
    UiTextKey::CommandPalette,
];

fn palette_input_scope_id(kind: PaletteKind) -> &'static str {
    match kind {
        PaletteKind::Command => "palette.command",
        PaletteKind::Project => "palette.project",
        PaletteKind::Tab => "palette.tab",
        PaletteKind::Pane => "palette.pane",
    }
}

struct RenderTerminalPaneInput<'a> {
    project_id: &'a str,
    project_path: &'a Path,
    project_title: &'a str,
    pane: &'a PaneConfig,
    tab_id: &'a str,
    tab_title: &'a str,
    is_focused: bool,
}

struct RenderTerminalTreeInput<'a> {
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
    value: String,
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
        Self::with_workspace_and_config_paths(Workspace::new(), config_paths)
    }

    pub fn from_startup_env() -> Self {
        let mut root = Self::new();
        if let Some(project_path) = std::env::var_os("YTTT_OPEN_PROJECT") {
            let _ = root.open_project_path(PathBuf::from(project_path));
        }
        root
    }

    fn with_workspace_and_config_paths(workspace: Workspace, config_paths: AppConfigPaths) -> Self {
        let default_layout_state = DefaultLayoutState::load_or_create(&config_paths);
        let command_registry = default_registry();
        let recent_projects = load_recent_projects(&config_paths)
            .map(recent_projects_for_palette)
            .unwrap_or_default();
        let (load_error, keybinding_warning_lines) =
            load_keybindings_messages(&config_paths, &command_registry);
        let keybindings_editor = load_keybindings_editor_state(&config_paths, &command_registry);
        let (app_settings, settings_warning_lines) = load_app_settings_messages(&config_paths);
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
        let load_error = combine_load_messages(
            load_error,
            layout_load_warning_message(default_layout_state.warnings()),
        );
        let system_notifications_enabled = app_settings.notifications.system;
        let mut project_editor_runtime = ProjectEditorRuntime::default();
        for project in workspace.opened_projects() {
            let selected_terminal_id = project
                .layout
                .tab(&project.selected_tab_id)
                .map(|_| project.selected_tab_id.clone());
            project_editor_runtime.open_project(
                project.id.clone(),
                project.path.clone(),
                selected_terminal_id,
                app_settings.project_panel.default_open,
                app_settings.project_panel.width,
            );
        }

        Self {
            workspace,
            config_paths,
            default_layout_state,
            active_palette: None,
            command_registry,
            recent_projects,
            load_error,
            layout_source_messages: HashMap::new(),
            keybinding_warning_lines,
            keybindings_editor,
            last_opened_layout_file: None,
            last_opened_keybindings_file: None,
            pending_close_project_id: None,
            pending_document_saves: Vec::new(),
            pending_focus_change_autosaves: Vec::new(),
            pending_file_close_requests: Vec::new(),
            pending_project_close_requests: Vec::new(),
            pending_file_conflict: None,
            pending_dirty_close: None,
            allow_window_close_once: false,
            pending_open_project_request: false,
            pending_status_notifications: Vec::new(),
            pending_tab_rename: None,
            pending_keybinding_edit: None,
            keybinding_interceptor_subscription: None,
            focus_handle: None,
            input_owner_stack: InputOwnerStack::default(),
            terminal_input_gate: TerminalInputGate::default(),
            palette_input: None,
            palette_input_subscription: None,
            palette_input_needs_focus: false,
            tab_rename_input: None,
            tab_rename_input_subscription: None,
            tab_rename_input_needs_focus: false,
            keybinding_edit_input: None,
            keybinding_edit_input_subscription: None,
            keybinding_edit_input_needs_focus: false,
            settings_search_input: None,
            settings_search_input_subscription: None,
            settings_search_input_needs_focus: false,
            settings_language_select: None,
            settings_language_select_subscription: None,
            settings_shell_select: None,
            settings_shell_select_subscription: None,
            settings_ui_theme_select: None,
            settings_ui_theme_select_subscription: None,
            settings_icon_theme_select: None,
            settings_icon_theme_select_subscription: None,
            settings_terminal_theme_select: None,
            settings_terminal_theme_select_subscription: None,
            settings_editor_language_select: None,
            settings_editor_language_select_subscription: None,
            settings_font_family_select: None,
            settings_font_family_select_subscription: None,
            settings_editor_font_family_select: None,
            settings_editor_font_family_select_subscription: None,
            settings_editor_autosave_select: None,
            settings_editor_autosave_select_subscription: None,
            settings_number_inputs: HashMap::new(),
            settings_number_input_subscriptions: HashMap::new(),
            layout_toml_editor: None,
            layout_toml_input: None,
            layout_toml_input_subscription: None,
            layout_toml_input_needs_focus: false,
            palette_scroll_handle: ScrollHandle::new(),
            sidebar_collapsed: false,
            active_sidebar_resize_drag: None,
            active_split_resize_drag: None,
            pending_terminal_focus_pane_id: None,
            pending_editor_focus_document_id: None,
            terminal_panes: HashMap::new(),
            terminal_pane_subscriptions: HashMap::new(),
            project_editor_runtime,
            pending_project_tree_loads: Vec::new(),
            project_git_statuses: HashMap::new(),
            toast_queue: ToastQueue::default(),
            system_notifier: NoopSystemNotifier,
            system_notifications_enabled,
            ui_text: ui_text_for_language(app_settings.general.language),
            app_settings,
            theme_runtime,
            icon_theme,
            settings_page: SettingsPageState::default(),
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    pub fn select_project(&mut self, project_id: &ProjectId) -> Result<(), WorkbenchError> {
        if self.workspace.selected_project_id() != Some(project_id)
            && let Some(WorkItemId::File(document_id)) = self.active_work_item()
        {
            self.queue_focus_change_autosave(document_id);
        }
        self.workspace.select_project(project_id)?;
        self.refresh_selected_project_git_status();
        if let Some(active) = self.active_work_item() {
            self.apply_active_work_item(&active)?;
        }
        Ok(())
    }

    pub fn project_editor_runtime(&self) -> &ProjectEditorRuntime {
        &self.project_editor_runtime
    }

    pub fn project_editor_runtime_mut(&mut self) -> &mut ProjectEditorRuntime {
        &mut self.project_editor_runtime
    }

    pub fn active_work_item(&self) -> Option<WorkItemId> {
        let project_id = self.workspace.selected_project_id()?;
        self.project_editor_runtime
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
        self.project_editor_runtime
            .workspace()
            .session(project_id)
            .map(|session| session.project_panel_width())
    }

    pub fn set_project_panel_width(&mut self, width: f32) -> Result<(), WorkbenchError> {
        let width = width.clamp(PROJECT_FILE_PANEL_MIN_WIDTH, PROJECT_FILE_PANEL_MAX_WIDTH);
        self.app_settings.project_panel.width = width;
        if let Some(project_id) = self.workspace.selected_project_id().cloned()
            && let Some(session) = self
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

    pub fn theme_runtime(&self) -> &ThemeRuntime {
        &self.theme_runtime
    }

    pub fn visible_tab_rename_dialog_title(&self) -> Option<String> {
        self.pending_tab_rename
            .as_ref()
            .map(|_| self.ui_text.get(UiTextKey::RenameTabTitle).to_string())
    }

    pub fn pending_tab_rename_value(&self) -> Option<String> {
        self.pending_tab_rename
            .as_ref()
            .map(|rename| rename.value.clone())
    }

    pub fn pending_keybinding_edit_value(&self) -> Option<String> {
        self.pending_keybinding_edit
            .as_ref()
            .map(|edit| edit.value.clone())
    }

    pub fn confirm_tab_rename_dialog(&mut self, title: &str) -> Result<(), WorkbenchError> {
        let Some(rename) = self.pending_tab_rename.clone() else {
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
        let value = self.keybindings_editor.command_keys(command).join(", ");
        self.pending_keybinding_edit = Some(PendingKeybindingEdit { command, value });
        self.reset_keybinding_edit_input();
        self.keybinding_edit_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn confirm_keybinding_edit_dialog(&mut self, value: &str) -> Result<(), WorkbenchError> {
        let Some(edit) = self.pending_keybinding_edit.clone() else {
            return Ok(());
        };
        self.set_keybinding_command_keys(edit.command, parse_keybinding_edit_value(value))?;
        self.clear_keybinding_edit_dialog();
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
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

    pub fn visible_error_message(&self) -> Option<&str> {
        self.load_error.as_deref()
    }

    pub fn visible_error_notification_item(&self) -> Option<ToastItem> {
        self.load_error.as_ref().map(|message| ToastItem {
            title: message.clone(),
            context: self.ui_text.get(UiTextKey::StatusErrorContext).to_string(),
            tone: ToastTone::Error,
        })
    }

    pub fn visible_layout_source_message(&self) -> Option<&str> {
        let selected_project_id = self.workspace.selected_project_id()?;
        self.layout_source_messages
            .get(selected_project_id)
            .map(String::as_str)
    }

    pub fn should_auto_focus_workspace(&self) -> bool {
        self.foreground_input_owner_kind() == InputOwnerKind::Workspace
    }

    pub fn foreground_input_owner_kind(&self) -> InputOwnerKind {
        self.input_owner_stack.active_owner().active_kind()
    }

    pub fn foreground_input_scope_id(&self) -> Option<String> {
        Some(
            self.input_owner_stack
                .active_owner()
                .active_scope_id()
                .as_str()
                .to_string(),
        )
    }

    pub fn terminal_input_allowed(&self) -> bool {
        self.input_owner_stack
            .active_owner()
            .terminal_input_allowed()
    }

    pub fn take_pending_terminal_focus_for_render(&mut self, pane_id: &str) -> bool {
        if !self.should_auto_focus_workspace() {
            return false;
        }

        if self.pending_terminal_focus_pane_id.as_deref() == Some(pane_id) {
            self.pending_terminal_focus_pane_id = None;
            true
        } else {
            false
        }
    }

    pub fn should_use_palette_text_fallback(&self, input_is_focused: bool) -> bool {
        self.active_palette.is_some() && !input_is_focused
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
        self.keybinding_warning_lines
            .iter()
            .map(String::as_str)
            .collect()
    }

    pub fn visible_keybinding_rows(&self) -> Vec<KeybindingRow> {
        self.keybindings_editor.rows()
    }

    pub fn runtime_keybinding_specs(&self) -> Vec<UiKeybindingSpec> {
        ui_keybinding_specs_from_config(self.keybindings_editor.config(), &self.command_registry)
    }

    pub fn runtime_command_for_keystroke(&self, keystroke: &Keystroke) -> Option<CommandId> {
        runtime_command_for_keystroke(&self.runtime_keybinding_specs(), keystroke)
    }

    pub(crate) fn set_keybinding_interceptor_subscription(&mut self, subscription: Subscription) {
        self.keybinding_interceptor_subscription = Some(subscription);
    }

    pub fn dispatch_runtime_keybinding(&mut self, keystroke: &Keystroke) -> bool {
        let Some(command_id) = workspace_command_for_keystroke(
            self.foreground_input_owner_kind(),
            keystroke,
            |keystroke| self.runtime_command_for_keystroke(keystroke),
            |keystroke| self.terminal_should_receive_keystroke(keystroke),
        ) else {
            return false;
        };

        let _ = self.run_command(command_id);
        true
    }

    pub fn terminal_should_receive_keystroke(&self, keystroke: &Keystroke) -> bool {
        self.terminal_input_allowed()
            && self.selected_focused_pane_id().is_some()
            && !keystroke.modifiers.platform
            && keystroke_to_bytes(keystroke, Default::default()).is_some()
    }

    pub fn set_keybinding_command_keys(
        &mut self,
        command: CommandId,
        keys: Vec<String>,
    ) -> Result<(), WorkbenchError> {
        let previous = self.keybindings_editor.clone();
        self.keybindings_editor.set_command_keys(command, keys);
        if let Err(error) = self.save_keybindings_editor() {
            self.keybindings_editor = previous;
            return Err(error);
        }
        Ok(())
    }

    pub fn delete_keybinding_command_keys(
        &mut self,
        command: CommandId,
    ) -> Result<(), WorkbenchError> {
        self.keybindings_editor.delete_command_keys(command);
        self.save_keybindings_editor()
    }

    pub fn reset_keybinding_command_keys(
        &mut self,
        command: CommandId,
    ) -> Result<(), WorkbenchError> {
        self.keybindings_editor.reset_command_keys(command);
        self.save_keybindings_editor()
    }

    pub fn visible_empty_workspace_actions(&self) -> Vec<&'static str> {
        EMPTY_WORKSPACE_ACTIONS
            .iter()
            .map(|key| self.ui_text.get(*key))
            .collect()
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
        let git_status = self.project_git_statuses.get(selected_project_id);

        TitlebarInfo {
            project_name: project.layout.project.name.clone(),
            compact_path: Some(compact_path_for_titlebar(
                &project.path.display().to_string(),
            )),
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
        collect_terminal_pane_contexts(
            &project_id,
            &project_path,
            &project_title,
            &tab_id,
            &tab_title,
            &layout,
            focused_pane_id.as_deref(),
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

    pub fn handle_terminal_pane_exit(
        &mut self,
        event: TerminalPaneExitedEvent,
    ) -> Result<PaneExitCloseOutcome, WorkbenchError> {
        let project_id = ProjectId::new(event.project_id.clone());
        if !self.app_settings.terminal.close_on_exit {
            self.workspace
                .record_pane_exited(&project_id, &event.tab_id, &event.pane_id)?;
            self.load_error = None;
            return Ok(PaneExitCloseOutcome::PaneKept);
        }

        let outcome =
            self.workspace
                .close_pane_for_exit(&project_id, &event.tab_id, &event.pane_id)?;
        let key = terminal_pane_key(&event.project_id, &event.tab_id, &event.pane_id);
        self.terminal_panes.remove(&key);
        self.terminal_pane_subscriptions.remove(&key);

        if self.pending_terminal_focus_pane_id.as_deref() == Some(event.pane_id.as_str()) {
            self.pending_terminal_focus_pane_id = None;
        }
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
        self.pending_terminal_focus_pane_id.as_deref()
    }

    pub fn pending_editor_focus_document_id(&self) -> Option<&crate::ui::editor::DocumentId> {
        self.pending_editor_focus_document_id.as_ref()
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
            CommandId::ProjectOpen => {
                self.request_open_project();
                Ok(())
            }
            CommandId::ProjectOpenRecent => {
                self.open_palette(PaletteKind::Project);
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
            CommandId::ProjectPanelToggle => {
                if let Some(project_id) = self.workspace.selected_project_id().cloned()
                    && let Some(session) = self
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
            CommandId::FileSave => {
                if let Some(WorkItemId::File(document_id)) = self.active_work_item()
                    && !self.pending_document_saves.contains(&document_id)
                {
                    self.pending_document_saves.push(document_id);
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
                let shell = self.resolved_terminal_shell();
                let tab_id = self.workspace.create_shell_tab_with_command(shell)?;
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
                if local_layout_file.exists() {
                    self.show_layout_file_path_status(&local_layout_file);
                    self.last_opened_layout_file = Some(local_layout_file);
                } else if project_layout_file.exists() {
                    self.show_layout_file_path_status(&project_layout_file);
                    self.last_opened_layout_file = Some(project_layout_file);
                } else {
                    let default_layout_file = self.default_layout_state.path().to_path_buf();
                    self.show_layout_file_path_status(&default_layout_file);
                    self.last_opened_layout_file = Some(default_layout_file);
                }
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
        self.pending_close_project_id.is_some()
            || self
                .pending_dirty_close
                .as_ref()
                .is_some_and(|pending| matches!(pending.intent, DirtyCloseIntent::Project(_)))
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
            .pending_dirty_close
            .as_ref()
            .is_some_and(|pending| matches!(pending.intent, DirtyCloseIntent::Project(_)))
        {
            return self.visible_dirty_close_dialog_text();
        }
        self.pending_close_project_id.as_ref().map(|_| {
            format!(
                "{}\n{}",
                self.ui_text.get(UiTextKey::CloseProjectTitle),
                self.ui_text.get(UiTextKey::CloseProjectBody)
            )
        })
    }

    pub fn visible_close_project_dialog_actions(&self) -> Vec<String> {
        if self
            .pending_dirty_close
            .as_ref()
            .is_some_and(|pending| matches!(pending.intent, DirtyCloseIntent::Project(_)))
        {
            return self.visible_dirty_close_actions();
        }
        if self.pending_close_project_id.is_some() {
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

    fn queue_status_notification(&mut self, title: impl Into<String>, context: impl Into<String>) {
        self.pending_status_notifications.push(ToastItem {
            title: title.into(),
            context: context.into(),
            tone: ToastTone::Success,
        });
    }

    fn flush_pending_status_notifications(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let theme = self.theme_runtime.ui;
        for item in self.pending_status_notifications.drain(..) {
            window.push_notification(workbench_status_notification(item, theme), cx);
        }
    }

    pub fn confirm_pending_project_close(&mut self) -> Result<(), WorkbenchError> {
        let project_id = self
            .pending_close_project_id
            .clone()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let closed = self.workspace.confirm_close_project(&project_id)?;
        self.pending_close_project_id = None;
        self.cleanup_closed_project(&closed.project_id);
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn cancel_pending_project_close(&mut self) {
        self.pending_close_project_id = None;
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
                let opened_path = opened.path.clone();
                let project_id = self.workspace.open_project(opened.path, opened.layout)?;
                let selected_terminal_id =
                    self.workspace.project(&project_id).and_then(|project| {
                        project
                            .layout
                            .tab(&project.selected_tab_id)
                            .map(|_| project.selected_tab_id.clone())
                    });
                self.project_editor_runtime.open_project(
                    project_id.clone(),
                    opened_path.clone(),
                    selected_terminal_id,
                    self.app_settings.project_panel.default_open,
                    self.app_settings.project_panel.width,
                );
                self.refresh_project_git_status(&project_id, &opened_path);
                self.queue_selected_terminal_focus();
                self.layout_source_messages
                    .insert(project_id, source_message);
                self.recent_projects = recent_projects_for_palette(opened.recent_projects);
                self.load_error = warning_message;
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
        workspace
            .open_project(PathBuf::from("/tmp/yttt"), dev_fixture_layout())
            .expect("dev fixture layout should be valid");
        Self::with_workspace(workspace)
    }

    pub fn agent_exit_fixture() -> Self {
        let mut workspace = Workspace::new();
        workspace
            .open_project(
                PathBuf::from("/tmp/yttt-agent-exit"),
                agent_exit_fixture_layout(),
            )
            .expect("agent exit fixture layout should be valid");
        Self::with_workspace(workspace)
    }

    pub fn with_workspace_for_test(workspace: Workspace) -> Self {
        Self::with_workspace(workspace)
    }

    pub fn with_workspace_for_test_and_config_paths(
        workspace: Workspace,
        config_paths: AppConfigPaths,
    ) -> Self {
        Self::with_workspace_and_config_paths(workspace, config_paths)
    }

    fn with_workspace(workspace: Workspace) -> Self {
        Self::with_workspace_and_config_paths(workspace, AppConfigPaths::for_app())
    }

    fn request_close_selected_project(&mut self) -> Result<CloseProjectDecision, WorkbenchError> {
        let project_id = self
            .workspace
            .selected_project_id()
            .cloned()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        if self
            .project_editor_runtime
            .documents_for_project(&project_id)
            .next()
            .is_some()
        {
            if !self.pending_project_close_requests.contains(&project_id) {
                self.pending_project_close_requests.push(project_id.clone());
            }
            return Ok(CloseProjectDecision::NeedsConfirmation {
                project_id: project_id.clone(),
                running_pane_count: self.project_running_pane_count(&project_id),
            });
        }
        let decision = self.workspace.request_close_project(&project_id)?;
        match &decision {
            CloseProjectDecision::Closed(closed) => {
                self.pending_close_project_id = None;
                self.cleanup_closed_project(&closed.project_id);
            }
            CloseProjectDecision::NeedsConfirmation { project_id, .. } => {
                self.pending_close_project_id = Some(project_id.clone());
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
        self.layout_source_messages.remove(project_id);
        self.project_git_statuses.remove(project_id);
        self.pending_project_tree_loads
            .retain(|(pending_project_id, _)| pending_project_id != project_id);
        self.pending_document_saves
            .retain(|document_id| &document_id.project_id != project_id);
        self.pending_focus_change_autosaves
            .retain(|document_id| &document_id.project_id != project_id);
        self.pending_file_close_requests
            .retain(|document_id| &document_id.project_id != project_id);
        self.pending_project_close_requests
            .retain(|pending_project_id| pending_project_id != project_id);
        if self
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| &conflict.document_id.project_id == project_id)
        {
            self.pending_file_conflict = None;
        }
        self.remove_terminal_panes_for_project(project_id.as_str());
        self.project_editor_runtime.close_project(project_id);
        if self
            .pending_editor_focus_document_id
            .as_ref()
            .is_some_and(|document_id| &document_id.project_id == project_id)
        {
            self.pending_editor_focus_document_id = None;
        }
    }

    fn close_active_work_item(&mut self) -> Result<(), WorkbenchError> {
        let Some(active) = self.active_work_item() else {
            return Ok(());
        };
        match active {
            WorkItemId::File(document_id) => {
                if self.project_editor_runtime.document(&document_id).is_some() {
                    if !self.pending_file_close_requests.contains(&document_id) {
                        self.pending_file_close_requests.push(document_id);
                    }
                    Ok(())
                } else {
                    self.close_file_work_item_immediately(&document_id)
                }
            }
            WorkItemId::Terminal(tab_id) => {
                self.workspace.select_tab(&tab_id)?;
                dispatch_workspace_command(&mut self.workspace, CommandId::TabClose)?;
                let selected_tab_id = self
                    .workspace
                    .selected_project_id()
                    .and_then(|project_id| self.workspace.project(project_id))
                    .map(|project| project.selected_tab_id.clone());
                if let Some(selected_tab_id) = selected_tab_id {
                    self.select_work_item(WorkItemId::Terminal(selected_tab_id))?;
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
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&project_id)
            .and_then(|session| session.close_file(document_id, &terminal_ids));
        self.project_editor_runtime.remove_document(document_id);
        if self
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| &conflict.document_id == document_id)
        {
            self.pending_file_conflict = None;
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
        self.pending_tab_rename = Some(PendingTabRename { tab_id, value });
        self.reset_tab_rename_input();
        self.tab_rename_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    fn clear_tab_rename_dialog(&mut self) {
        self.pending_tab_rename = None;
        self.reset_tab_rename_input();
    }

    fn clear_keybinding_edit_dialog(&mut self) {
        self.pending_keybinding_edit = None;
        self.reset_keybinding_edit_input();
    }

    fn remove_terminal_panes_for_project(&mut self, project_id: &str) {
        let prefix = format!("{project_id}:");
        self.terminal_panes
            .retain(|key, _pane| !key.starts_with(&prefix));
        self.terminal_pane_subscriptions
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

        Ok((project.path.clone(), project.layout.clone()))
    }

    fn set_layout_toml_editor_error(&mut self, source: &'static str, error: String) {
        if let Some(session) = &mut self.layout_toml_editor {
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

    fn reset_tab_rename_input(&mut self) {
        self.tab_rename_input = None;
        self.tab_rename_input_subscription = None;
        self.tab_rename_input_needs_focus = false;
    }

    fn reset_keybinding_edit_input(&mut self) {
        self.keybinding_edit_input = None;
        self.keybinding_edit_input_subscription = None;
        self.keybinding_edit_input_needs_focus = false;
    }

    fn reset_settings_search_input(&mut self) {
        self.settings_search_input = None;
        self.settings_search_input_subscription = None;
        self.settings_search_input_needs_focus = false;
        self.settings_language_select = None;
        self.settings_language_select_subscription = None;
        self.settings_shell_select = None;
        self.settings_shell_select_subscription = None;
        self.settings_ui_theme_select = None;
        self.settings_ui_theme_select_subscription = None;
        self.settings_icon_theme_select = None;
        self.settings_icon_theme_select_subscription = None;
        self.settings_terminal_theme_select = None;
        self.settings_terminal_theme_select_subscription = None;
        self.settings_editor_language_select = None;
        self.settings_editor_language_select_subscription = None;
        self.settings_font_family_select = None;
        self.settings_font_family_select_subscription = None;
        self.settings_editor_font_family_select = None;
        self.settings_editor_font_family_select_subscription = None;
        self.settings_editor_autosave_select = None;
        self.settings_editor_autosave_select_subscription = None;
        self.settings_number_inputs.clear();
        self.settings_number_input_subscriptions.clear();
    }

    fn reset_layout_toml_input(&mut self) {
        self.layout_toml_input = None;
        self.layout_toml_input_subscription = None;
        self.layout_toml_input_needs_focus = false;
    }

    fn queue_terminal_focus(&mut self, pane_id: &str) {
        self.pending_terminal_focus_pane_id = Some(pane_id.to_string());
    }

    fn queue_selected_terminal_focus(&mut self) {
        if let Some(pane_id) = self.selected_focused_pane_id().map(ToOwned::to_owned) {
            self.queue_terminal_focus(&pane_id);
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
        match item {
            WorkItemId::Terminal(tab_id) => {
                self.workspace.select_tab(tab_id)?;
                self.pending_editor_focus_document_id = None;
                self.queue_selected_terminal_focus();
            }
            WorkItemId::File(document_id) => {
                self.pending_terminal_focus_pane_id = None;
                self.pending_editor_focus_document_id = Some(document_id.clone());
            }
        }
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
        let selected_terminal = self.workspace.project(&project_id).and_then(|project| {
            terminal_ids
                .contains(&project.selected_tab_id)
                .then(|| WorkItemId::Terminal(project.selected_tab_id.clone()))
        });
        let next = if let Some(selected_terminal) = selected_terminal {
            self.project_editor_runtime
                .workspace_mut()
                .session_mut(&project_id)
                .and_then(|session| {
                    session
                        .select_work_item(selected_terminal.clone(), &terminal_ids)
                        .then_some(selected_terminal)
                })
        } else {
            self.project_editor_runtime
                .workspace_mut()
                .session_mut(&project_id)
                .and_then(|session| session.select_next(&terminal_ids))
        };
        if let Some(next) = next {
            self.apply_active_work_item(&next)?;
        } else {
            self.pending_terminal_focus_pane_id = None;
            self.pending_editor_focus_document_id = None;
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

    fn refresh_project_git_status(&mut self, project_id: &ProjectId, project_path: &Path) {
        if let Some(status) = read_project_git_status(project_path) {
            self.project_git_statuses.insert(project_id.clone(), status);
        } else {
            self.project_git_statuses.remove(project_id);
        }
    }

    fn refresh_selected_project_git_status(&mut self) {
        let Some(project_id) = self.workspace.selected_project_id().cloned() else {
            return;
        };
        let Some(project_path) = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };

        self.refresh_project_git_status(&project_id, &project_path);
    }

    fn current_input_owner_registration(&self) -> InputOwnerRegistration {
        if self.pending_keybinding_edit.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::KeybindingRecorder,
                InputScopeId::new("recorder.keybinding"),
            )
        } else if self.pending_file_conflict.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.file_conflict"),
            )
        } else if self.pending_dirty_close.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.dirty_close"),
            )
        } else if self.pending_tab_rename.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.rename_tab"),
            )
        } else if self.pending_close_project_id.is_some() {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Dialog,
                InputScopeId::new("dialog.close_project"),
            )
        } else if self.layout_toml_editor.is_some() {
            let scope = self
                .layout_toml_editor
                .as_ref()
                .map(|session| session.target().input_scope_id())
                .unwrap_or("editor.project_layout");
            InputOwnerRegistration::blocking(InputOwnerKind::Dialog, InputScopeId::new(scope))
        } else if self.settings_page.is_open {
            InputOwnerRegistration::blocking(
                InputOwnerKind::Settings,
                InputScopeId::new("settings"),
            )
        } else if let Some(active_palette) = &self.active_palette {
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
        self.input_owner_stack.clear();
        let registration = self.current_input_owner_registration();
        if registration.kind() != InputOwnerKind::Workspace {
            self.input_owner_stack.push_owner(registration);
        }
        self.terminal_input_gate
            .sync_from_snapshot(&self.input_owner_stack.active_owner());
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
        let rename = self.pending_tab_rename.as_ref()?;
        let input = if let Some(input) = &self.tab_rename_input {
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
            self.tab_rename_input = Some(input.clone());
            self.tab_rename_input_subscription = Some(subscription);
            input
        };

        if self.tab_rename_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.tab_rename_input_needs_focus = false;
        }

        Some(input)
    }

    fn keybinding_edit_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let edit = self.pending_keybinding_edit.as_ref()?;
        let input = if let Some(input) = &self.keybinding_edit_input {
            input.clone()
        } else {
            let value = edit.value.clone();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("cmd-l, ctrl-l")
                    .default_value(value)
            });
            let subscription =
                cx.subscribe_in(&input, window, Self::on_keybinding_edit_input_event);
            self.keybinding_edit_input = Some(input.clone());
            self.keybinding_edit_input_subscription = Some(subscription);
            input
        };

        if self.keybinding_edit_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.keybinding_edit_input_needs_focus = false;
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
                if let Some(rename) = &mut self.pending_tab_rename {
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

    fn on_keybinding_edit_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if let Some(edit) = &mut self.pending_keybinding_edit {
                    edit.value = input.read(cx).value().to_string();
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                let value = input.read(cx).value().to_string();
                let _ = self.confirm_keybinding_edit_dialog(&value);
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
            .tab_rename_input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .or_else(|| self.pending_tab_rename_value())
            .unwrap_or_default();

        self.confirm_tab_rename_dialog(&value)
    }

    fn confirm_keybinding_edit_dialog_from_input(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(), WorkbenchError> {
        let value = self
            .keybinding_edit_input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .or_else(|| self.pending_keybinding_edit_value())
            .unwrap_or_default();

        self.confirm_keybinding_edit_dialog(&value)
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
