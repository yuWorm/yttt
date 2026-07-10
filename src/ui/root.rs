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

mod dialogs;
mod helpers;
pub mod layout_editor;
#[cfg(test)]
mod non_destructive_tests;
use dialogs::*;
use helpers::*;

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
    time::Duration,
};

type SettingsStringSelectState = SelectState<SearchableVec<String>>;

const TERMINAL_THEME_FOLLOW_UI: &str = "Follow UI theme";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum SettingsNumberField {
    FontSize,
    LineHeight,
    Padding,
    Scrollback,
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
        actions::{
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
        font_options::{
            terminal_font_family_option_for_setting, terminal_font_family_options_from_system,
            terminal_font_family_setting_from_option,
        },
        i18n::{Locale, UiText, UiTextKey},
        input_owner::{
            InputOwnerKind, InputOwnerRegistration, InputOwnerStack, InputScopeId,
            TerminalInputGate,
        },
        interaction::key_dispatch::workspace_command_for_keystroke,
        keybinding_display::primary_display_keybinding_for_current_platform,
        keybindings_editor::{KeybindingEditError, KeybindingRow, KeybindingsEditorState},
        overlay::capture_overlay_input,
        palette::palette_overlay,
        palette_surface::palette_input_placeholder,
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
        root::layout_editor::{
            LayoutEditorSession, LayoutEditorTarget, ProjectLayoutEditorFormat,
            write_layout_file_atomic,
        },
        settings::{SettingsGroupId, SettingsPageState, SettingsPanelStyle, settings_panel_style},
        sidebar::project_sidebar,
        split_view::{pointer_resize_for_drag_delta, split_child_basis},
        tabs::{
            FileTabSnapshot, WorkbenchTabItem, project_tabs, visible_tab_items,
            visible_work_item_tabs as merge_work_item_tabs,
        },
        terminal_pane::{
            TerminalPaneContext, TerminalPaneEvent, TerminalPaneExitedEvent, TerminalPaneView,
        },
        theme::{ThemeRuntime, WorkbenchTheme},
        titlebar::{TitlebarInfo, compact_path_for_titlebar, workbench_titlebar},
        toast::{ToastItem, ToastQueue, ToastTone, toast_item_for_event},
    },
};

pub use crate::ui::overlay::overlay_input_capture_policy;

pub struct RootView {
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
    settings_terminal_theme_select: Option<Entity<SettingsStringSelectState>>,
    settings_terminal_theme_select_subscription: Option<Subscription>,
    settings_editor_language_select: Option<Entity<SettingsStringSelectState>>,
    settings_editor_language_select_subscription: Option<Subscription>,
    settings_font_family_select: Option<Entity<SettingsStringSelectState>>,
    settings_font_family_select_subscription: Option<Subscription>,
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

impl RootView {
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
            settings_terminal_theme_select: None,
            settings_terminal_theme_select_subscription: None,
            settings_editor_language_select: None,
            settings_editor_language_select_subscription: None,
            settings_font_family_select: None,
            settings_font_family_select_subscription: None,
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
            settings_page: SettingsPageState::default(),
        }
    }

    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    pub fn workspace_mut(&mut self) -> &mut Workspace {
        &mut self.workspace
    }

    pub fn select_project(&mut self, project_id: &ProjectId) -> Result<(), RootViewError> {
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

    pub fn refresh_project_tree_state(
        &mut self,
        project_id: &ProjectId,
    ) -> Option<DirectoryLoadRequest> {
        let request = self
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)?
            .file_tree_mut()
            .refresh();
        self.project_editor_runtime
            .track_tree_load(project_id.clone(), request.generation);
        Some(request)
    }

    pub fn apply_project_tree_snapshot(
        &mut self,
        project_id: &ProjectId,
        generation: u64,
        snapshot: DirectorySnapshot,
    ) -> bool {
        if !self
            .project_editor_runtime
            .tree_load_is_current(project_id, generation)
        {
            return false;
        }
        self.project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)
            .is_some_and(|session| session.file_tree_mut().apply_snapshot(generation, snapshot))
    }

    pub fn apply_project_tree_error(
        &mut self,
        project_id: &ProjectId,
        generation: u64,
        relative_directory: &Path,
        error: impl Into<String>,
    ) -> bool {
        if !self
            .project_editor_runtime
            .tree_load_is_current(project_id, generation)
        {
            return false;
        }
        self.project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)
            .is_some_and(|session| {
                session
                    .file_tree_mut()
                    .apply_error(generation, relative_directory, error)
            })
    }

    fn project_tree_render_snapshot(
        &self,
        project_id: &ProjectId,
    ) -> Option<ProjectTreeRenderSnapshot> {
        let session = self
            .project_editor_runtime
            .workspace()
            .session(project_id)?;
        Some(ProjectTreeRenderSnapshot::from_tree_with_text(
            session.file_tree(),
            self.project_git_statuses.get(project_id),
            &ProjectTreeRenderText {
                loading: self.ui_text.get(UiTextKey::ProjectFilesLoading).to_string(),
                empty_directory: self
                    .ui_text
                    .get(UiTextKey::ProjectFilesEmptyDirectory)
                    .to_string(),
                retry: self.ui_text.get(UiTextKey::ProjectFilesRetry).to_string(),
            },
        ))
    }

    fn ensure_project_tree_view(
        &mut self,
        project_id: &ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<ProjectTreeView>> {
        if let Some(tree) = self.project_editor_runtime.tree(project_id).cloned() {
            if let Some(snapshot) = self.project_tree_render_snapshot(project_id) {
                tree.update(cx, |tree, tree_cx| tree.sync(snapshot, tree_cx));
            }
            return Some(tree);
        }

        let request = self
            .project_editor_runtime
            .workspace_mut()
            .session_mut(project_id)?
            .file_tree_mut()
            .request_expand(Path::new(""));
        if let Some(request) = &request {
            self.project_editor_runtime
                .track_tree_load(project_id.clone(), request.generation);
        }
        let snapshot = self.project_tree_render_snapshot(project_id)?;
        let tree = cx.new(|tree_cx| ProjectTreeView::new(snapshot, tree_cx));
        let event_project_id = project_id.clone();
        let subscription = cx.subscribe_in(&tree, window, move |this, tree, event, window, cx| {
            this.on_project_tree_view_event(&event_project_id, tree, event, window, cx);
        });
        self.project_editor_runtime
            .insert_tree(project_id.clone(), tree.clone(), subscription);
        if let Some(request) = request {
            self.spawn_project_directory_scan(project_id.clone(), request, window, cx);
        }
        Some(tree)
    }

    fn on_project_tree_view_event(
        &mut self,
        project_id: &ProjectId,
        _tree: &Entity<ProjectTreeView>,
        event: &ProjectTreeViewEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            ProjectTreeViewEvent::ToggleDirectory { path, expanded } => {
                let request = self
                    .project_editor_runtime
                    .workspace_mut()
                    .session_mut(project_id)
                    .and_then(|session| {
                        let tree = session.file_tree_mut();
                        tree.select(Some(path.clone()));
                        if *expanded {
                            tree.request_expand(path)
                        } else {
                            tree.collapse(path);
                            None
                        }
                    });
                if let Some(request) = request {
                    self.project_editor_runtime
                        .track_tree_load(project_id.clone(), request.generation);
                    self.spawn_project_directory_scan(project_id.clone(), request, window, cx);
                }
            }
            ProjectTreeViewEvent::OpenFile(path) => {
                if let Some(session) = self
                    .project_editor_runtime
                    .workspace_mut()
                    .session_mut(project_id)
                {
                    session.file_tree_mut().select(Some(path.clone()));
                }
                self.spawn_project_file_open(project_id.clone(), path.clone(), window, cx);
            }
            ProjectTreeViewEvent::Refresh => {
                self.refresh_project_tree(project_id.clone(), window, cx);
            }
        }
        cx.notify();
    }

    fn refresh_project_tree(
        &mut self,
        project_id: ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let project_path = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone());
        if let Some(request) = self.refresh_project_tree_state(&project_id) {
            self.spawn_project_directory_scan(project_id.clone(), request, window, cx);
        }
        self.check_project_documents_for_external_changes(&project_id, window, cx);
        if let Some(project_path) = project_path {
            self.refresh_project_git_status(&project_id, &project_path);
        }
    }

    fn queue_project_tree_refresh(&mut self, project_id: ProjectId) {
        if let Some(request) = self.refresh_project_tree_state(&project_id) {
            self.pending_project_tree_loads.push((project_id, request));
        }
    }

    fn flush_pending_project_tree_loads(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_project_tree_loads);
        for (project_id, request) in pending {
            self.check_project_documents_for_external_changes(&project_id, window, cx);
            self.spawn_project_directory_scan(project_id, request, window, cx);
        }
    }

    fn spawn_project_directory_scan(
        &mut self,
        project_id: ProjectId,
        request: DirectoryLoadRequest,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .project_editor_runtime
            .tree_load_is_current(&project_id, request.generation)
        {
            return;
        }
        let Some(project_root) = self
            .workspace
            .project(&project_id)
            .map(|project| project.path.clone())
        else {
            return;
        };
        let relative_directory = request.relative_directory.clone();
        let generation = request.generation;
        let show_hidden = self.app_settings.project_panel.show_hidden;
        let scan_relative_directory = relative_directory.clone();
        let io_task = cx.background_spawn(async move {
            scan_project_directory(&project_root, &scan_relative_directory, show_hidden)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, _window, cx| {
                match result {
                    Ok(snapshot) => {
                        root.apply_project_tree_snapshot(&project_id, generation, snapshot);
                    }
                    Err(error) => {
                        let message = root.localized_project_tree_error(&error);
                        root.apply_project_tree_error(
                            &project_id,
                            generation,
                            &relative_directory,
                            message,
                        );
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn localized_project_tree_error(&self, error: &ProjectTreeFsError) -> String {
        format!(
            "{}: {error}",
            self.ui_text.get(UiTextKey::ProjectFilesDirectoryError)
        )
    }

    fn localized_project_file_error(&self, error: &ProjectFileIoError) -> String {
        let summary = match error {
            ProjectFileIoError::PathOutsideProject { .. } => {
                self.ui_text.get(UiTextKey::ProjectFileOutsideProject)
            }
            ProjectFileIoError::FileTooLarge { .. } => {
                self.ui_text.get(UiTextKey::ProjectFileTooLarge)
            }
            ProjectFileIoError::BinaryContent { .. } => {
                self.ui_text.get(UiTextKey::ProjectFileUnsupportedBinary)
            }
            ProjectFileIoError::InvalidUtf8 { .. } => {
                self.ui_text.get(UiTextKey::ProjectFileInvalidEncoding)
            }
            ProjectFileIoError::NotAFile { .. } | ProjectFileIoError::Io { .. } => {
                self.ui_text.get(UiTextKey::StatusErrorContext)
            }
        };
        format!("{summary}: {error}")
    }

    pub fn begin_project_file_open(
        &mut self,
        project_id: &ProjectId,
        relative_path: &Path,
    ) -> Option<ProjectFileLoadRequest> {
        let project_root = self.workspace.project(project_id)?.path.clone();
        if self
            .project_editor_runtime
            .workspace()
            .session(project_id)
            .is_none()
        {
            return None;
        }
        let document_id = crate::ui::editor::DocumentId {
            project_id: project_id.clone(),
            canonical_path: project_root.join(relative_path),
        };
        let generation = self
            .project_editor_runtime
            .begin_file_load(document_id.clone())?;
        Some(ProjectFileLoadRequest {
            document_id,
            project_root,
            relative_path: relative_path.to_path_buf(),
            generation,
        })
    }

    pub fn cancel_project_file_open(&mut self, request: &ProjectFileLoadRequest) -> bool {
        self.project_editor_runtime
            .finish_file_load(&request.document_id, request.generation)
    }

    pub fn apply_project_file_open_error(
        &mut self,
        request: &ProjectFileLoadRequest,
        error: impl Into<String>,
    ) -> bool {
        if !self
            .project_editor_runtime
            .finish_file_load(&request.document_id, request.generation)
        {
            return false;
        }
        self.load_error = Some(error.into());
        true
    }

    fn spawn_project_file_open(
        &mut self,
        project_id: ProjectId,
        relative_path: PathBuf,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let requested_document_id =
            self.workspace
                .project(&project_id)
                .map(|project| crate::ui::editor::DocumentId {
                    project_id: project_id.clone(),
                    canonical_path: project.path.join(&relative_path),
                });
        if let Some(document_id) = requested_document_id
            && (self.project_editor_runtime.document(&document_id).is_some()
                || self
                    .project_editor_runtime
                    .workspace()
                    .session(&project_id)
                    .is_some_and(|session| session.file_ids().contains(&document_id)))
        {
            let _ = self.select_work_item(WorkItemId::File(document_id));
            cx.notify();
            return;
        }
        let Some(request) = self.begin_project_file_open(&project_id, &relative_path) else {
            return;
        };
        let project_root = request.project_root.clone();
        let read_relative_path = request.relative_path.clone();
        let io_task = cx
            .background_spawn(async move { read_project_file(&project_root, &read_relative_path) });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                match result {
                    Ok(loaded) => {
                        root.apply_project_file_open_success(&request, loaded, window, cx);
                    }
                    Err(error) => {
                        let message = root.localized_project_file_error(&error);
                        root.apply_project_file_open_error(&request, message);
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_project_file_open_success(
        &mut self,
        request: &ProjectFileLoadRequest,
        loaded: LoadedProjectFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self
            .project_editor_runtime
            .finish_file_load(&request.document_id, request.generation)
            || self
                .workspace
                .project(&request.document_id.project_id)
                .is_none()
        {
            return false;
        }
        let document_id = crate::ui::editor::DocumentId {
            project_id: request.document_id.project_id.clone(),
            canonical_path: loaded.canonical_path.clone(),
        };
        if self.project_editor_runtime.document(&document_id).is_none() {
            let language_mode = if self.app_settings.editor.auto_detect_language {
                CodeEditorLanguageMode::Auto
            } else {
                CodeEditorLanguageMode::from(self.app_settings.editor.default_language.clone())
            };
            let title = loaded
                .relative_path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| loaded.relative_path.to_string_lossy().into_owned());
            let config = CodeEditorConfig::new(title, language_mode)
                .with_tab_size(self.app_settings.editor.tab_size)
                .with_soft_wrap(self.app_settings.editor.soft_wrap)
                .with_line_number(self.app_settings.editor.line_numbers);
            let model = ProjectEditorModel::new(
                document_id.clone(),
                CodeEditorState::new(&loaded.canonical_path, config, loaded.text),
                loaded.fingerprint,
            );
            let appearance = EditorAppearance::from(&self.app_settings.editor);
            let document = cx.new(|document_cx| {
                ProjectEditorDocument::new(model, appearance, window, document_cx)
            });
            let subscription =
                cx.subscribe_in(&document, window, Self::on_project_editor_document_event);
            self.project_editor_runtime.insert_document(
                document_id.clone(),
                document,
                subscription,
            );
        }
        let opened_id = self
            .project_editor_runtime
            .workspace_mut()
            .session_mut(&document_id.project_id)
            .map(|session| session.open_file(document_id.canonical_path.clone()));
        let Some(opened_id) = opened_id else {
            self.project_editor_runtime.remove_document(&document_id);
            return false;
        };
        let _ = self.select_work_item(WorkItemId::File(opened_id));
        self.load_error = None;
        true
    }

    fn flush_pending_document_saves(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_document_saves);
        for document_id in pending {
            self.save_document(document_id, false, SaveContinuation::None, window, cx);
        }
    }

    fn flush_pending_focus_change_autosaves(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let pending = std::mem::take(&mut self.pending_focus_change_autosaves);
        for document_id in pending {
            self.autosave_document(document_id, None, window, cx);
        }
    }

    fn flush_pending_file_close_requests(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_file_close_requests);
        for document_id in pending {
            let is_dirty = self
                .project_editor_runtime
                .document(&document_id)
                .is_some_and(|document| document.read(cx).model().is_dirty());
            if is_dirty {
                if self.pending_dirty_close.is_none() {
                    self.pending_dirty_close = Some(PendingDirtyClose {
                        intent: DirtyCloseIntent::File(document_id.clone()),
                        dirty_documents: vec![document_id],
                        running_pane_count: 0,
                        saving_documents: HashSet::new(),
                    });
                    self.sync_input_owner_state();
                }
            } else if let Err(error) = self.close_file_work_item_immediately(&document_id) {
                self.load_error = Some(error.to_string());
            }
        }
    }

    fn flush_pending_project_close_requests(&mut self, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.pending_project_close_requests);
        for project_id in pending {
            if self.workspace.project(&project_id).is_none() {
                continue;
            }
            let dirty_documents = self
                .project_editor_runtime
                .documents_for_project(&project_id)
                .filter_map(|(document_id, document)| {
                    document
                        .read(cx)
                        .model()
                        .is_dirty()
                        .then(|| document_id.clone())
                })
                .collect::<Vec<_>>();
            let running_pane_count = self.project_running_pane_count(&project_id);
            if !dirty_documents.is_empty() || running_pane_count > 0 {
                if self.pending_dirty_close.is_none() {
                    self.pending_close_project_id = None;
                    self.pending_dirty_close = Some(PendingDirtyClose {
                        intent: DirtyCloseIntent::Project(project_id),
                        dirty_documents,
                        running_pane_count,
                        saving_documents: HashSet::new(),
                    });
                }
            } else {
                match self.workspace.request_close_project(&project_id) {
                    Ok(CloseProjectDecision::Closed(closed)) => {
                        self.cleanup_closed_project(&closed.project_id)
                    }
                    Ok(CloseProjectDecision::NeedsConfirmation { project_id, .. }) => {
                        self.pending_close_project_id = Some(project_id);
                    }
                    Err(error) => self.load_error = Some(error.to_string()),
                }
            }
        }
        self.sync_input_owner_state();
    }

    pub fn has_pending_dirty_close(&self) -> bool {
        self.pending_dirty_close.is_some()
    }

    pub fn request_window_close(&mut self, cx: &mut Context<Self>) -> bool {
        if std::mem::take(&mut self.allow_window_close_once) {
            return true;
        }
        if self.pending_dirty_close.is_some() {
            return false;
        }
        let project_ids = self
            .workspace
            .opened_projects()
            .iter()
            .map(|project| project.id.clone())
            .collect::<Vec<_>>();
        let dirty_documents = project_ids
            .iter()
            .flat_map(|project_id| {
                self.project_editor_runtime
                    .documents_for_project(project_id)
                    .filter_map(|(document_id, document)| {
                        document
                            .read(cx)
                            .model()
                            .is_dirty()
                            .then(|| document_id.clone())
                    })
            })
            .collect::<Vec<_>>();
        let running_pane_count = project_ids
            .iter()
            .map(|project_id| self.project_running_pane_count(project_id))
            .sum();
        if dirty_documents.is_empty() && running_pane_count == 0 {
            return true;
        }
        self.pending_close_project_id = None;
        self.pending_dirty_close = Some(PendingDirtyClose {
            intent: DirtyCloseIntent::Window,
            dirty_documents,
            running_pane_count,
            saving_documents: HashSet::new(),
        });
        self.sync_input_owner_state();
        cx.notify();
        false
    }

    pub fn visible_dirty_close_actions(&self) -> Vec<String> {
        let Some(pending) = self.pending_dirty_close.as_ref() else {
            return Vec::new();
        };
        let save = if matches!(pending.intent, DirtyCloseIntent::File(_)) {
            UiTextKey::FileSaveAction
        } else {
            UiTextKey::SaveAllAndContinue
        };
        let discard = if matches!(pending.intent, DirtyCloseIntent::File(_)) {
            UiTextKey::Discard
        } else {
            UiTextKey::DiscardAndContinue
        };
        vec![
            self.ui_text.get(UiTextKey::Cancel).to_string(),
            self.ui_text.get(discard).to_string(),
            self.ui_text.get(save).to_string(),
        ]
    }

    pub fn visible_dirty_close_dialog_text(&self) -> Option<String> {
        let pending = self.pending_dirty_close.as_ref()?;
        let title = match &pending.intent {
            DirtyCloseIntent::File(_) => self.ui_text.get(UiTextKey::UnsavedChangesTitle),
            DirtyCloseIntent::Project(_) => self.ui_text.get(UiTextKey::CloseProjectTitle),
            DirtyCloseIntent::Window => self.ui_text.get(UiTextKey::CloseWindowTitle),
        };
        let mut lines = vec![title.to_string()];
        if !pending.dirty_documents.is_empty() {
            let count = self.localized_close_count(
                pending.dirty_documents.len(),
                UiTextKey::UnsavedFileSingular,
                UiTextKey::UnsavedFilePlural,
            );
            let file_names = pending
                .dirty_documents
                .iter()
                .map(|document_id| {
                    document_id
                        .canonical_path
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| document_id.canonical_path.display().to_string())
                })
                .collect::<Vec<_>>()
                .join(", ");
            lines.push(format!("{count}: {file_names}"));
        }
        if pending.running_pane_count > 0 {
            lines.push(self.localized_close_count(
                pending.running_pane_count,
                UiTextKey::RunningProcessSingular,
                UiTextKey::RunningProcessPlural,
            ));
        }
        Some(lines.join("\n"))
    }

    fn dirty_close_has_save_error(&self, cx: &Context<Self>) -> bool {
        self.pending_dirty_close.as_ref().is_some_and(|pending| {
            pending.dirty_documents.iter().any(|document_id| {
                self.project_editor_runtime
                    .document(document_id)
                    .is_some_and(|document| document.read(cx).model().editor().error().is_some())
            })
        })
    }

    fn localized_close_count(
        &self,
        count: usize,
        singular: UiTextKey,
        plural: UiTextKey,
    ) -> String {
        let unit = self.ui_text.get(if count == 1 { singular } else { plural });
        if unit.starts_with('个') {
            format!("{count}{unit}")
        } else {
            format!("{count} {unit}")
        }
    }

    pub fn cancel_pending_dirty_close(&mut self) {
        self.pending_dirty_close = None;
        self.sync_input_owner_state();
    }

    pub fn save_pending_dirty_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = &mut self.pending_dirty_close else {
            return;
        };
        let document_ids = pending.dirty_documents.clone();
        pending.saving_documents = document_ids.iter().cloned().collect();
        if document_ids.is_empty() {
            self.finish_pending_dirty_close(window, cx);
            return;
        }
        for document_id in document_ids {
            self.save_document(
                document_id,
                false,
                SaveContinuation::CompletePendingClose,
                window,
                cx,
            );
        }
        cx.notify();
    }

    fn finish_pending_dirty_close(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_dirty_close.take() else {
            return;
        };
        match pending.intent {
            DirtyCloseIntent::File(document_id) => {
                if let Err(error) = self.close_file_work_item_immediately(&document_id) {
                    self.load_error = Some(error.to_string());
                }
            }
            DirtyCloseIntent::Project(project_id) => {
                match self.workspace.confirm_close_project(&project_id) {
                    Ok(closed) => self.cleanup_closed_project(&closed.project_id),
                    Err(error) => self.load_error = Some(error.to_string()),
                }
            }
            DirtyCloseIntent::Window => {
                self.allow_window_close_once = true;
                window.remove_window();
            }
        }
        self.sync_input_owner_state();
        cx.notify();
    }

    pub fn discard_pending_dirty_close(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let Some(pending) = self.pending_dirty_close.take() else {
            return;
        };
        match pending.intent {
            DirtyCloseIntent::File(document_id) => {
                if let Err(error) = self.close_file_work_item_immediately(&document_id) {
                    self.load_error = Some(error.to_string());
                }
            }
            DirtyCloseIntent::Project(project_id) => {
                match self.workspace.confirm_close_project(&project_id) {
                    Ok(closed) => self.cleanup_closed_project(&closed.project_id),
                    Err(error) => self.load_error = Some(error.to_string()),
                }
            }
            DirtyCloseIntent::Window => {
                self.allow_window_close_once = true;
                _window.remove_window();
            }
        }
        self.sync_input_owner_state();
        cx.notify();
    }

    fn queue_focus_change_autosave(&mut self, document_id: crate::ui::editor::DocumentId) {
        if self.app_settings.editor.autosave == EditorAutosave::OnFocusChange
            && !self.pending_focus_change_autosaves.contains(&document_id)
        {
            self.pending_focus_change_autosaves.push(document_id);
        }
    }

    fn schedule_delayed_autosave(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        generation: u64,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.app_settings.editor.autosave != EditorAutosave::AfterDelay {
            self.project_editor_runtime
                .cancel_autosave_task(&document_id);
            return;
        }
        let delay = Duration::from_millis(self.app_settings.editor.autosave_delay_ms);
        let task_document_id = document_id.clone();
        let task = cx.spawn_in(window, async move |this, cx| {
            cx.background_executor().timer(delay).await;
            let _ = this.update_in(cx, |root, window, cx| {
                if root.app_settings.editor.autosave == EditorAutosave::AfterDelay {
                    root.autosave_document(task_document_id, Some(generation), window, cx);
                }
            });
        });
        self.project_editor_runtime
            .replace_autosave_task(document_id, task);
    }

    fn autosave_document(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        expected_generation: Option<u64>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| conflict.document_id == document_id)
        {
            return;
        }
        let Some(document) = self.project_editor_runtime.document(&document_id).cloned() else {
            return;
        };
        let (generation, dirty, saving) = {
            let document = document.read(cx);
            (
                document.model().generation(),
                document.model().is_dirty(),
                !matches!(document.model().save_state(), ProjectEditorSaveState::Idle),
            )
        };
        if expected_generation.is_some_and(|expected| expected != generation) || !dirty {
            return;
        }
        if saving {
            self.project_editor_runtime
                .request_follow_up_autosave(document_id, generation);
            return;
        }
        self.save_document(document_id, false, SaveContinuation::None, window, cx);
    }

    fn run_follow_up_autosave(
        &mut self,
        document_id: &crate::ui::editor::DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(generation) = self
            .project_editor_runtime
            .take_follow_up_autosave(document_id)
        {
            self.autosave_document(document_id.clone(), Some(generation), window, cx);
        }
    }

    pub fn save_active_document(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(WorkItemId::File(document_id)) = self.active_work_item() else {
            return;
        };
        self.save_document(document_id, false, SaveContinuation::None, window, cx);
    }

    fn save_document(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        force: bool,
        continuation: SaveContinuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self.project_editor_runtime.document(&document_id).cloned() else {
            return;
        };
        let request = document.update(cx, |document, _cx| {
            if !force && !matches!(document.model().save_state(), ProjectEditorSaveState::Idle) {
                return None;
            }
            Some(document.model_mut().begin_save())
        });
        let Some(request) = request else {
            return;
        };
        self.spawn_project_file_save_request(request, force, continuation, window, cx);
    }

    fn project_file_root(&self, document_id: &crate::ui::editor::DocumentId) -> Option<PathBuf> {
        Some(
            self.workspace
                .project(&document_id.project_id)?
                .path
                .clone(),
        )
    }

    fn spawn_project_file_save_request(
        &mut self,
        request: SaveRequest,
        force: bool,
        continuation: SaveContinuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_root) = self.project_file_root(&request.document_id) else {
            let message = self
                .ui_text
                .get(UiTextKey::ProjectFileOutsideProject)
                .to_string();
            if let Some(document) = self
                .project_editor_runtime
                .document(&request.document_id)
                .cloned()
            {
                document.update(cx, |document, _| {
                    document.model_mut().fail_save(&request, message.clone());
                });
            }
            self.load_error = Some(message);
            return;
        };
        let text = request.text.clone();
        let expected_fingerprint = request.expected_fingerprint.clone();
        let canonical_path = request.document_id.canonical_path.clone();
        let io_task = cx.background_spawn(async move {
            let relative_path = project_relative_path(&project_root, &canonical_path)?;
            let mode = if force {
                SaveMode::Force
            } else {
                SaveMode::Check(&expected_fingerprint)
            };
            save_project_file(&project_root, &relative_path, &text, mode)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                root.apply_project_file_save_result(request, result, continuation, window, cx);
                cx.notify();
            });
        })
        .detach();
    }

    fn apply_project_file_save_result(
        &mut self,
        request: SaveRequest,
        result: Result<SaveProjectFileOutcome, ProjectFileIoError>,
        continuation: SaveContinuation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(document) = self
            .project_editor_runtime
            .document(&request.document_id)
            .cloned()
        else {
            return;
        };
        let continuation = if self
            .pending_dirty_close
            .as_ref()
            .is_some_and(|pending| pending.saving_documents.contains(&request.document_id))
        {
            SaveContinuation::CompletePendingClose
        } else {
            continuation
        };
        match result {
            Ok(SaveProjectFileOutcome::Saved(fingerprint)) => {
                let completed = document.update(cx, |document, _| {
                    document.model_mut().finish_save(&request, fingerprint)
                });
                if !completed {
                    return;
                }
                if self
                    .pending_file_conflict
                    .as_ref()
                    .is_some_and(|conflict| conflict.document_id == request.document_id)
                {
                    self.pending_file_conflict = None;
                }
                let file_name = request
                    .document_id
                    .canonical_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| request.document_id.canonical_path.display().to_string());
                self.queue_status_notification(
                    format!("{}: {file_name}", self.ui_text.get(UiTextKey::FileSaved)),
                    self.ui_text.get(UiTextKey::ProjectFiles),
                );
                self.load_error = None;
                self.complete_save_continuation(continuation, &request.document_id, window, cx);
                self.flush_pending_status_notifications(window, cx);
                self.run_follow_up_autosave(&request.document_id, window, cx);
            }
            Ok(SaveProjectFileOutcome::Conflict(current_disk)) => {
                self.project_editor_runtime
                    .take_follow_up_autosave(&request.document_id);
                let is_dirty = document.read(cx).model().is_dirty();
                if !is_dirty && matches!(current_disk, CurrentDiskState::Present(_)) {
                    document.update(cx, |document, _| {
                        document.model_mut().cancel_save(&request);
                    });
                    self.spawn_project_file_reload(request.document_id, None, window, cx);
                    return;
                }
                self.pending_file_conflict = Some(PendingFileConflict {
                    document_id: request.document_id.clone(),
                    request,
                    current_disk,
                    continuation,
                });
                self.load_error = None;
                self.sync_input_owner_state();
            }
            Err(error) => {
                self.project_editor_runtime
                    .take_follow_up_autosave(&request.document_id);
                let message = format!(
                    "{}: {}",
                    self.ui_text.get(UiTextKey::FileSaveFailed),
                    self.localized_project_file_error(&error)
                );
                document.update(cx, |document, _| {
                    document.model_mut().fail_save(&request, message.clone());
                });
                if continuation == SaveContinuation::CompletePendingClose
                    && let Some(pending) = &mut self.pending_dirty_close
                {
                    pending.saving_documents.remove(&request.document_id);
                }
                self.load_error = Some(message);
            }
        }
    }

    fn complete_save_continuation(
        &mut self,
        continuation: SaveContinuation,
        document_id: &crate::ui::editor::DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match continuation {
            SaveContinuation::None => {}
            SaveContinuation::CompletePendingClose => {
                let still_dirty = self
                    .project_editor_runtime
                    .document(document_id)
                    .is_some_and(|document| document.read(cx).model().is_dirty());
                let should_finish = if let Some(pending) = &mut self.pending_dirty_close {
                    pending.saving_documents.remove(document_id);
                    if !still_dirty {
                        pending
                            .dirty_documents
                            .retain(|pending_id| pending_id != document_id);
                    }
                    pending.dirty_documents.is_empty() && pending.saving_documents.is_empty()
                } else {
                    false
                };
                if should_finish {
                    self.finish_pending_dirty_close(window, cx);
                }
            }
        }
    }

    fn spawn_project_file_reload(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        restore_conflict_on_error: Option<PendingFileConflict>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(project_root) = self.project_file_root(&document_id) else {
            if let Some(conflict) = restore_conflict_on_error {
                self.pending_file_conflict = Some(conflict);
            }
            return;
        };
        let canonical_path = document_id.canonical_path.clone();
        let io_task = cx.background_spawn(async move {
            let relative_path = project_relative_path(&project_root, &canonical_path)?;
            read_project_file(&project_root, &relative_path)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                let document = root.project_editor_runtime.document(&document_id).cloned();
                match (document, result) {
                    (Some(document), Ok(loaded)) => {
                        document.update(cx, |document, document_cx| {
                            document.replace_from_disk(
                                loaded.text,
                                loaded.fingerprint,
                                window,
                                document_cx,
                            );
                        });
                        root.pending_file_conflict = None;
                        root.load_error = None;
                        root.sync_input_owner_state();
                    }
                    (Some(_document), Err(error)) => {
                        let message = root.localized_project_file_error(&error);
                        if let Some(conflict) = restore_conflict_on_error {
                            root.pending_file_conflict = Some(conflict);
                        }
                        root.load_error = Some(message);
                    }
                    (None, _) => {}
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn check_project_documents_for_external_changes(
        &mut self,
        project_id: &ProjectId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document_ids = self
            .project_editor_runtime
            .documents_for_project(project_id)
            .map(|(document_id, _)| document_id.clone())
            .collect::<Vec<_>>();
        for document_id in document_ids {
            self.check_document_for_external_changes(document_id, window, cx);
        }
    }

    fn check_document_for_external_changes(
        &mut self,
        document_id: crate::ui::editor::DocumentId,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| conflict.document_id == document_id)
        {
            return;
        }
        let Some(document) = self.project_editor_runtime.document(&document_id).cloned() else {
            return;
        };
        let (expected_fingerprint, save_is_idle) = {
            let document = document.read(cx);
            (
                document.model().disk_fingerprint().clone(),
                matches!(document.model().save_state(), ProjectEditorSaveState::Idle),
            )
        };
        if !save_is_idle {
            return;
        }
        let Some(project_root) = self.project_file_root(&document_id) else {
            return;
        };
        let canonical_path = document_id.canonical_path.clone();
        let io_task = cx.background_spawn(async move {
            let relative_path = project_relative_path(&project_root, &canonical_path)?;
            read_project_file(&project_root, &relative_path)
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = io_task.await;
            let _ = this.update_in(cx, |root, window, cx| {
                if root
                    .pending_file_conflict
                    .as_ref()
                    .is_some_and(|conflict| conflict.document_id == document_id)
                {
                    return;
                }
                let Some(document) = root.project_editor_runtime.document(&document_id).cloned()
                else {
                    return;
                };
                if document.read(cx).model().disk_fingerprint() != &expected_fingerprint
                    || !matches!(
                        document.read(cx).model().save_state(),
                        ProjectEditorSaveState::Idle
                    )
                {
                    return;
                }
                match result {
                    Ok(loaded) if loaded.fingerprint == expected_fingerprint => {}
                    Ok(loaded) if document.read(cx).model().is_dirty() => {
                        let request =
                            document.update(cx, |document, _| document.model_mut().begin_save());
                        root.pending_file_conflict = Some(PendingFileConflict {
                            document_id: document_id.clone(),
                            request,
                            current_disk: CurrentDiskState::Present(loaded.fingerprint),
                            continuation: SaveContinuation::None,
                        });
                        root.load_error = None;
                        root.sync_input_owner_state();
                    }
                    Ok(loaded) => {
                        document.update(cx, |document, document_cx| {
                            document.replace_from_disk(
                                loaded.text,
                                loaded.fingerprint,
                                window,
                                document_cx,
                            );
                        });
                        root.load_error = None;
                    }
                    Err(ProjectFileIoError::Io { source, .. })
                        if source.kind() == std::io::ErrorKind::NotFound =>
                    {
                        let request =
                            document.update(cx, |document, _| document.model_mut().begin_save());
                        root.pending_file_conflict = Some(PendingFileConflict {
                            document_id: document_id.clone(),
                            request,
                            current_disk: CurrentDiskState::Missing,
                            continuation: SaveContinuation::None,
                        });
                        root.load_error = None;
                        root.sync_input_owner_state();
                    }
                    Err(error) => {
                        root.load_error = Some(root.localized_project_file_error(&error));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub fn has_pending_file_conflict(&self) -> bool {
        self.pending_file_conflict.is_some()
    }

    pub fn visible_file_conflict_dialog_text(&self) -> Option<String> {
        let conflict = self.pending_file_conflict.as_ref()?;
        let title = if matches!(conflict.current_disk, CurrentDiskState::Missing) {
            self.ui_text.get(UiTextKey::FileDeletedOnDisk)
        } else {
            self.ui_text.get(UiTextKey::FileChangedOnDisk)
        };
        Some(format!(
            "{title}\n{}",
            conflict.document_id.canonical_path.display()
        ))
    }

    pub fn visible_file_conflict_dialog_actions(&self) -> Vec<String> {
        let Some(conflict) = self.pending_file_conflict.as_ref() else {
            return Vec::new();
        };
        let mut actions = vec![self.ui_text.get(UiTextKey::Cancel).to_string()];
        if !matches!(conflict.current_disk, CurrentDiskState::Missing) {
            actions.push(self.ui_text.get(UiTextKey::FileReload).to_string());
        }
        actions.push(
            self.ui_text
                .get(
                    if matches!(conflict.current_disk, CurrentDiskState::Missing) {
                        UiTextKey::FileRecreate
                    } else {
                        UiTextKey::FileOverwrite
                    },
                )
                .to_string(),
        );
        actions
    }

    pub fn pending_document_save_count(&self) -> usize {
        self.pending_document_saves.len()
    }

    pub fn pending_file_conflict_is_missing(&self) -> bool {
        self.pending_file_conflict
            .as_ref()
            .is_some_and(|conflict| matches!(conflict.current_disk, CurrentDiskState::Missing))
    }

    pub fn overwrite_pending_file_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(conflict) = self.pending_file_conflict.take() else {
            return;
        };
        self.spawn_project_file_save_request(
            conflict.request,
            true,
            conflict.continuation,
            window,
            cx,
        );
        self.sync_input_owner_state();
        cx.notify();
    }

    pub fn reload_pending_file_conflict(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(mut conflict) = self.pending_file_conflict.take() else {
            return;
        };
        if conflict.continuation == SaveContinuation::CompletePendingClose {
            self.pending_dirty_close = None;
            conflict.continuation = SaveContinuation::None;
        }
        let document_id = conflict.document_id.clone();
        self.spawn_project_file_reload(document_id, Some(conflict), window, cx);
        self.sync_input_owner_state();
        cx.notify();
    }

    pub fn cancel_pending_file_conflict(&mut self, cx: &mut Context<Self>) {
        let Some(conflict) = self.pending_file_conflict.take() else {
            return;
        };
        if let Some(document) = self
            .project_editor_runtime
            .document(&conflict.document_id)
            .cloned()
        {
            document.update(cx, |document, _| {
                document.model_mut().cancel_save(&conflict.request);
            });
        }
        if conflict.continuation == SaveContinuation::CompletePendingClose
            && let Some(pending) = &mut self.pending_dirty_close
        {
            pending.saving_documents.remove(&conflict.document_id);
        }
        self.sync_input_owner_state();
    }

    pub fn active_work_item(&self) -> Option<WorkItemId> {
        let project_id = self.workspace.selected_project_id()?;
        self.project_editor_runtime
            .workspace()
            .session(project_id)?
            .active_work_item()
            .cloned()
    }

    pub fn select_work_item(&mut self, item: WorkItemId) -> Result<bool, RootViewError> {
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

    pub fn select_next_work_item(&mut self) -> Result<Option<WorkItemId>, RootViewError> {
        self.select_relative_work_item(true)
    }

    pub fn select_previous_work_item(&mut self) -> Result<Option<WorkItemId>, RootViewError> {
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

    pub fn selected_project_panel_width(&self) -> Option<f32> {
        let project_id = self.workspace.selected_project_id()?;
        self.project_editor_runtime
            .workspace()
            .session(project_id)
            .map(|session| session.project_panel_width())
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

    pub fn persist_sidebar_width(&mut self, side: SidebarSide) -> Result<(), RootViewError> {
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

    pub fn active_palette(&self) -> Option<&ActivePalette> {
        self.active_palette.as_ref()
    }

    pub fn open_palette(&mut self, kind: PaletteKind) {
        self.active_palette = Some(ActivePalette::new(kind));
        self.reset_palette_input();
        self.palette_scroll_handle = ScrollHandle::new();
        self.palette_input_needs_focus = true;
        self.sync_input_owner_state();
    }

    pub fn close_palette(&mut self) {
        self.active_palette = None;
        self.reset_palette_input();
        self.sync_input_owner_state();
    }

    pub fn set_palette_query(&mut self, query: impl Into<String>) {
        if let Some(active_palette) = &mut self.active_palette {
            active_palette.query = query.into();
            active_palette.selected_index = 0;
            self.reset_palette_input();
        }
    }

    pub fn sync_palette_query_from_input_value(&mut self, query: impl Into<String>) -> bool {
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        let query = query.into();
        if active_palette.query != query {
            active_palette.query = query;
            active_palette.selected_index = 0;
        }
        true
    }

    pub fn confirm_palette_selection(&mut self) -> Result<(), RootViewError> {
        let Some(active_palette) = self.active_palette.clone() else {
            return Ok(());
        };
        let items = self.palette_items(active_palette.kind);
        let Some(item) = active_palette.selected_item(&items).cloned() else {
            return Ok(());
        };

        if !item.enabled {
            let reason = item
                .disabled_reason
                .as_deref()
                .unwrap_or(self.ui_text.get(UiTextKey::CommandUnavailable));
            self.load_error = Some(format!("Command unavailable: {reason}"));
            return Ok(());
        }

        match active_palette.kind {
            PaletteKind::Command => {
                let opens_palette = opens_palette_command(item.command);
                self.run_command(item.command)?;
                if opens_palette {
                    return Ok(());
                }
            }
            PaletteKind::Project => {
                let project_id = self
                    .workspace
                    .opened_projects()
                    .iter()
                    .find(|project| project.id.as_str() == item.id)
                    .map(|project| project.id.clone());
                if let Some(project_id) = project_id {
                    self.select_project(&project_id)?;
                } else if item.command == CommandId::ProjectOpenRecent {
                    self.open_project_path(PathBuf::from(&item.id))?;
                }
            }
            PaletteKind::Tab => {
                let project_id = self
                    .workspace
                    .selected_project_id()
                    .cloned()
                    .ok_or(WorkspaceError::NoSelectedProject)?;
                if let Some(work_item) = decode_tab_palette_item_id(&item.id, &project_id) {
                    self.select_work_item(work_item)?;
                }
            }
            PaletteKind::Pane => {
                self.focus_visible_terminal_pane(&item.id)?;
            }
        }

        self.close_palette();
        Ok(())
    }

    pub fn active_palette_items(&self) -> Vec<PaletteItem> {
        let Some(active_palette) = &self.active_palette else {
            return Vec::new();
        };

        self.palette_items(active_palette.kind)
    }

    pub fn visible_palette_titles(&self) -> Vec<String> {
        let Some(active_palette) = &self.active_palette else {
            return Vec::new();
        };
        let items = self.palette_items(active_palette.kind);

        active_palette
            .filtered_items(&items)
            .into_iter()
            .map(|item| item.title.clone())
            .collect()
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

    pub fn confirm_tab_rename_dialog(&mut self, title: &str) -> Result<(), RootViewError> {
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

    pub fn open_keybinding_edit_dialog(&mut self, command: CommandId) -> Result<(), RootViewError> {
        let value = self.keybindings_editor.command_keys(command).join(", ");
        self.pending_keybinding_edit = Some(PendingKeybindingEdit { command, value });
        self.reset_keybinding_edit_input();
        self.keybinding_edit_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn confirm_keybinding_edit_dialog(&mut self, value: &str) -> Result<(), RootViewError> {
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
    ) -> Result<(), RootViewError> {
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
    ) -> Result<(), RootViewError> {
        self.handle_work_item_tab_click(WorkItemId::Terminal(tab_id.to_string()), click_count)
    }

    pub fn handle_work_item_tab_click(
        &mut self,
        work_item: WorkItemId,
        click_count: usize,
    ) -> Result<(), RootViewError> {
        if !self.select_work_item(work_item.clone())? {
            return Ok(());
        }
        if click_count >= 2 && matches!(work_item, WorkItemId::Terminal(_)) {
            self.run_command(CommandId::TabRename)?;
        }
        self.load_error = None;
        Ok(())
    }

    pub fn close_project_tab(&mut self, tab_id: &str) -> Result<(), RootViewError> {
        self.close_work_item_tab(WorkItemId::Terminal(tab_id.to_string()))
    }

    pub fn close_work_item_tab(&mut self, work_item: WorkItemId) -> Result<(), RootViewError> {
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
    ) -> Result<Option<f32>, RootViewError> {
        let Some(resize) = pointer_resize_for_drag_delta(direction, delta_x, delta_y) else {
            return Ok(None);
        };

        self.workspace
            .resize_focused_split(resize.direction, resize.delta)
            .map(Some)
            .map_err(RootViewError::from)
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

    pub fn system_notifications_enabled(&self) -> bool {
        self.system_notifications_enabled
    }

    pub fn terminal_close_on_exit(&self) -> bool {
        self.app_settings.terminal.close_on_exit
    }

    pub fn terminal_show_scrollbar(&self) -> bool {
        self.app_settings.terminal.show_scrollbar
    }

    pub fn editor_auto_detect_language(&self) -> bool {
        self.app_settings.editor.auto_detect_language
    }

    pub fn editor_default_language(&self) -> &str {
        &self.app_settings.editor.default_language
    }

    pub fn editor_autosave(&self) -> EditorAutosave {
        self.app_settings.editor.autosave
    }

    pub fn editor_lsp_enabled(&self) -> bool {
        self.app_settings.editor.lsp.enabled
    }

    pub fn editor_lsp_command(&self) -> &str {
        &self.app_settings.editor.lsp.command
    }

    pub fn settings_is_open(&self) -> bool {
        self.settings_page.is_open
    }

    pub fn open_settings(&mut self) {
        self.close_palette();
        self.settings_page.is_open = true;
        self.settings_search_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
    }

    pub fn close_settings(&mut self) {
        self.settings_page.is_open = false;
        self.reset_settings_search_input();
        self.sync_input_owner_state();
    }

    pub fn set_system_notifications_enabled(&mut self, enabled: bool) -> Result<(), RootViewError> {
        self.app_settings.notifications.system = enabled;
        save_settings(&self.config_paths, &self.app_settings)?;
        self.system_notifications_enabled = enabled;
        Ok(())
    }

    pub fn set_language(&mut self, language: LanguageSetting) -> Result<(), RootViewError> {
        self.app_settings.general.language = language;
        save_settings(&self.config_paths, &self.app_settings)?;
        self.ui_text = ui_text_for_language(language);
        self.reset_palette_input();
        self.reset_settings_search_input();
        Ok(())
    }

    pub fn set_terminal_shell(&mut self, shell: &str) -> Result<(), RootViewError> {
        self.app_settings.terminal.shell = shell.to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_ui_theme_name(&mut self, theme_name: &str) -> Result<(), RootViewError> {
        self.app_settings.theme.name = theme_name.to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_theme_name(
        &mut self,
        theme_name: Option<&str>,
    ) -> Result<(), RootViewError> {
        self.app_settings.theme.terminal = theme_name.map(ToString::to_string);
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_font_family(&mut self, font_family: &str) -> Result<(), RootViewError> {
        self.app_settings.terminal.font_family = font_family.to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_font_size(&mut self, font_size: f32) -> Result<(), RootViewError> {
        self.app_settings.terminal.font_size = font_size;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_line_height(&mut self, line_height: f32) -> Result<(), RootViewError> {
        self.app_settings.terminal.line_height = line_height;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_padding(&mut self, padding: f32) -> Result<(), RootViewError> {
        self.app_settings.terminal.padding = padding;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_scrollback(&mut self, scrollback: usize) -> Result<(), RootViewError> {
        self.app_settings.terminal.scrollback = scrollback;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_close_on_exit(&mut self, close_on_exit: bool) -> Result<(), RootViewError> {
        self.app_settings.terminal.close_on_exit = close_on_exit;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_terminal_show_scrollbar(
        &mut self,
        show_scrollbar: bool,
    ) -> Result<(), RootViewError> {
        self.app_settings.terminal.show_scrollbar = show_scrollbar;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_auto_detect_language(
        &mut self,
        auto_detect_language: bool,
    ) -> Result<(), RootViewError> {
        self.app_settings.editor.auto_detect_language = auto_detect_language;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_default_language(
        &mut self,
        default_language: &str,
    ) -> Result<(), RootViewError> {
        self.app_settings.editor.default_language = default_language.trim().to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_lsp_enabled(&mut self, enabled: bool) -> Result<(), RootViewError> {
        self.app_settings.editor.lsp.enabled = enabled;
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_editor_lsp_command(&mut self, command: &str) -> Result<(), RootViewError> {
        self.app_settings.editor.lsp.command = command.trim().to_string();
        self.save_app_settings_and_refresh_runtime()
    }

    pub fn set_settings_search_query(&mut self, query: impl Into<String>) {
        self.settings_page.search_query = query.into();
        let selected_group_visible = self
            .settings_page
            .visible_groups(&self.ui_text)
            .iter()
            .any(|group| group.id == self.settings_page.selected_group);
        if !selected_group_visible
            && let Some(first_group) = self.settings_page.visible_groups(&self.ui_text).first()
        {
            self.settings_page.selected_group = first_group.id;
        }
    }

    pub fn select_settings_group(&mut self, group_id: &str) -> Result<(), String> {
        let group = SettingsGroupId::from_id(group_id)
            .ok_or_else(|| format!("Unknown settings group: {group_id}"))?;
        self.settings_page.selected_group = group;
        Ok(())
    }

    pub fn visible_settings_group_titles(&self) -> Vec<&'static str> {
        self.settings_page
            .visible_groups(&self.ui_text)
            .into_iter()
            .map(|group| group.title)
            .collect()
    }

    pub fn selected_settings_group_title(&self) -> Option<&'static str> {
        Some(self.settings_page.selected_group.title(&self.ui_text))
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

    pub fn layout_toml_editor_is_open(&self) -> bool {
        self.layout_toml_editor.is_some()
    }

    pub fn layout_toml_editor_path(&self) -> Option<&Path> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.editor().path())
    }

    pub fn layout_toml_editor_value(&self) -> Option<&str> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.editor().value())
    }

    pub fn visible_layout_toml_editor_error(&self) -> Option<&str> {
        self.layout_toml_editor
            .as_ref()
            .and_then(|session| session.editor().error())
    }

    pub fn visible_layout_toml_editor_diagnostics(&self) -> Vec<EditorDiagnostic> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.editor().diagnostics().to_vec())
            .unwrap_or_default()
    }

    pub fn layout_editor_target_kind(&self) -> Option<&'static str> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.target().kind())
    }

    pub fn visible_layout_toml_editor_language_id(&self) -> Option<EditorLanguageId> {
        self.layout_toml_editor
            .as_ref()
            .map(|session| session.editor().language_id())
    }

    pub fn open_layout_toml_editor(&mut self) -> Result<(), RootViewError> {
        self.open_default_layout_editor()
    }

    pub fn open_default_layout_editor(&mut self) -> Result<(), RootViewError> {
        let path = self.default_layout_state.path().to_path_buf();
        let value = fs::read_to_string(&path).map_err(|source| {
            RootViewError::LayoutTomlEditor(format!(
                "failed to read layout TOML at {}: {source}",
                path.display()
            ))
        })?;

        self.layout_toml_editor = Some(LayoutEditorSession::new(
            LayoutEditorTarget::Default,
            CodeEditorState::new(
                path,
                CodeEditorConfig::new(
                    "Edit default layout TOML",
                    CodeEditorLanguageMode::Explicit(EditorLanguageId::Toml),
                )
                .placeholder_text("Edit layout TOML...")
                .with_rows(24)
                .with_soft_wrap(false),
                value,
            ),
        ));
        self.finish_opening_layout_editor();
        Ok(())
    }

    pub fn open_project_layout_editor(&mut self) -> Result<(), RootViewError> {
        let project_id = self
            .workspace
            .selected_project_id()
            .cloned()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let project = self
            .workspace
            .project(&project_id)
            .ok_or_else(|| WorkspaceError::ProjectNotFound(project_id.as_str().to_string()))?;
        let project_path = project.path.clone();
        let effective_layout = project.layout.clone();
        let project_file = self.config_paths.project_layout_file(&project_path);
        let personal_file = self.config_paths.local_layout_file(&project_path);

        let (path, format, value, diagnostic) = if personal_file.exists() {
            let value = read_layout_editor_source(&personal_file)?;
            match parse_personal_layout(&personal_file, &value) {
                Ok(PersonalLayout::Patch(_)) => (
                    personal_file,
                    ProjectLayoutEditorFormat::PersonalPatch,
                    value,
                    None,
                ),
                Ok(PersonalLayout::Replace(_)) => (
                    personal_file,
                    ProjectLayoutEditorFormat::PersonalReplace,
                    value,
                    None,
                ),
                Err(error) => {
                    let message = error.to_string();
                    (
                        personal_file,
                        ProjectLayoutEditorFormat::InvalidPersonal,
                        value,
                        Some(message),
                    )
                }
            }
        } else if project_file.exists() {
            let value = read_layout_editor_source(&project_file)?;
            (
                project_file,
                ProjectLayoutEditorFormat::ProjectConfig,
                value,
                None,
            )
        } else {
            let path = save_local_layout(&self.config_paths, &project_path, &effective_layout)?;
            let value = read_layout_editor_source(&path)?;
            (
                path,
                ProjectLayoutEditorFormat::PersonalReplace,
                value,
                None,
            )
        };

        let mut editor = CodeEditorState::new(
            path.clone(),
            CodeEditorConfig::new(
                "Edit project layout TOML",
                CodeEditorLanguageMode::Explicit(EditorLanguageId::Toml),
            )
            .placeholder_text("Edit layout TOML...")
            .with_rows(24)
            .with_soft_wrap(false),
            value,
        );
        if let Some(message) = diagnostic {
            editor.set_error(message.clone());
            editor.set_diagnostics(vec![EditorDiagnostic::new(
                EditorDiagnosticSeverity::Error,
                "personal-layout",
                message,
            )]);
        }
        self.layout_toml_editor = Some(LayoutEditorSession::new(
            LayoutEditorTarget::Project {
                project_id,
                path,
                format,
            },
            editor,
        ));
        self.finish_opening_layout_editor();
        Ok(())
    }

    fn finish_opening_layout_editor(&mut self) {
        self.reset_layout_toml_input();
        self.layout_toml_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
    }

    pub fn set_layout_toml_editor_value(&mut self, value: impl Into<String>) {
        if let Some(session) = &mut self.layout_toml_editor {
            session.editor_mut().set_value(value);
            self.reset_layout_toml_input();
        }
    }

    pub fn save_layout_toml_editor(&mut self) -> Result<(), RootViewError> {
        let Some(session) = self.layout_toml_editor.clone() else {
            return Ok(());
        };
        let editor = session.editor();

        match session.target() {
            LayoutEditorTarget::Default => {
                let template = match toml::from_str::<DefaultLayoutTemplate>(editor.value()) {
                    Ok(template) => template,
                    Err(error) => {
                        self.set_layout_toml_editor_error(
                            "toml",
                            format!("failed to parse layout TOML: {error}"),
                        );
                        return Ok(());
                    }
                };
                if let Err(error) = template.validate() {
                    self.set_layout_toml_editor_error(
                        "layout",
                        format!("invalid layout TOML: {error}"),
                    );
                    return Ok(());
                }
                if let Err(error) = self.default_layout_state.save(template) {
                    let message = error.to_string();
                    self.set_layout_toml_editor_error("layout", message.clone());
                    self.load_error = Some(message);
                    return Ok(());
                }
            }
            LayoutEditorTarget::Project { path, format, .. } => {
                if let Err((source, message)) =
                    validate_project_editor_source(path, *format, editor.value())
                {
                    self.set_layout_toml_editor_error(source, message);
                    return Ok(());
                }
                if let Err(error) = write_layout_file_atomic(path, editor.value()) {
                    let message = error.to_string();
                    self.set_layout_toml_editor_error("filesystem", message.clone());
                    self.load_error = Some(message);
                    return Ok(());
                }
            }
        }

        self.layout_toml_editor = None;
        self.reset_layout_toml_input();
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn cancel_layout_toml_editor(&mut self) {
        self.layout_toml_editor = None;
        self.reset_layout_toml_input();
        self.queue_selected_terminal_focus();
        self.sync_input_owner_state();
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
    ) -> Result<(), RootViewError> {
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
    ) -> Result<(), RootViewError> {
        self.keybindings_editor.delete_command_keys(command);
        self.save_keybindings_editor()
    }

    pub fn reset_keybinding_command_keys(
        &mut self,
        command: CommandId,
    ) -> Result<(), RootViewError> {
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
    ) -> Result<PaneExitCloseOutcome, RootViewError> {
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

    pub fn focus_visible_terminal_pane(&mut self, pane_id: &str) -> Result<(), RootViewError> {
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

    pub fn run_command(&mut self, command_id: CommandId) -> Result<(), RootViewError> {
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
                if let Some(project_id) = self.workspace.selected_project_id().cloned() {
                    if let Some(session) = self
                        .project_editor_runtime
                        .workspace_mut()
                        .session_mut(&project_id)
                    {
                        session.toggle_project_panel();
                    }
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

    pub fn confirm_pending_project_close(&mut self) -> Result<(), RootViewError> {
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
    ) -> Result<(), RootViewError> {
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
                Err(RootViewError::from(error))
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

    fn palette_items(&self, kind: PaletteKind) -> Vec<PaletteItem> {
        match kind {
            PaletteKind::Command => self.command_palette_items(),
            PaletteKind::Project => project_palette_items_with_text(
                &self.workspace,
                &self.recent_projects,
                &self.ui_text,
            ),
            PaletteKind::Tab => self.selected_work_item_palette_items(),
            PaletteKind::Pane => {
                if matches!(self.active_work_item(), Some(WorkItemId::File(_))) {
                    Vec::new()
                } else {
                    pane_palette_items_with_text(&self.workspace, &self.ui_text).unwrap_or_default()
                }
            }
        }
    }

    fn command_palette_items(&self) -> Vec<PaletteItem> {
        let mut items = command_palette_items_with_text(
            &self.command_registry,
            CommandPaletteContext::from_command_context(self.command_context()),
            &self.ui_text,
        );

        for item in &mut items {
            item.keybinding = self.display_keybinding_for_command(item.command);
        }

        items
    }

    fn selected_work_item_palette_items(&self) -> Vec<PaletteItem> {
        let Some(project_id) = self.workspace.selected_project_id() else {
            return Vec::new();
        };
        let Some(project) = self.workspace.project(project_id) else {
            return Vec::new();
        };
        let mut snapshots = tab_palette_items_with_text(&self.workspace, &self.ui_text)
            .unwrap_or_default()
            .into_iter()
            .map(|item| {
                TabPaletteSnapshot::terminal(item.id, item.title, item.subtitle, item.status)
            })
            .collect::<Vec<_>>();
        let active = self.active_work_item();
        if let Some(session) = self.project_editor_runtime.workspace().session(project_id) {
            snapshots.extend(session.file_ids().iter().cloned().map(|document_id| {
                let relative_path = document_id
                    .canonical_path
                    .strip_prefix(&project.path)
                    .unwrap_or(&document_id.canonical_path)
                    .to_path_buf();
                let status = (active.as_ref() == Some(&WorkItemId::File(document_id.clone())))
                    .then(|| self.ui_text.get(UiTextKey::PaletteStatusActive).to_string());
                TabPaletteSnapshot::file(document_id, relative_path, status)
            }));
        }
        unified_tab_palette_items(&snapshots)
    }

    fn display_keybinding_for_command(&self, command: CommandId) -> Option<String> {
        primary_display_keybinding_for_current_platform(
            &self.keybindings_editor.command_keys(command),
        )
    }

    fn request_close_selected_project(&mut self) -> Result<CloseProjectDecision, RootViewError> {
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

    fn close_active_work_item(&mut self) -> Result<(), RootViewError> {
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
    ) -> Result<(), RootViewError> {
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

    fn open_selected_tab_rename_dialog(&mut self) -> Result<(), RootViewError> {
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

    fn selected_project_layout_snapshot(&self) -> Result<(PathBuf, ProjectLayout), RootViewError> {
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

    fn fail_workspace_error<T>(&mut self, error: WorkspaceError) -> Result<T, RootViewError> {
        self.load_error = Some(error.to_string());
        Err(error.into())
    }

    fn select_next_palette_item(&mut self) -> bool {
        let Some(kind) = self.active_palette.as_ref().map(|palette| palette.kind) else {
            return false;
        };
        let items = self.palette_items(kind);
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        active_palette.select_next(&items);
        true
    }

    fn select_prev_palette_item(&mut self) -> bool {
        let Some(kind) = self.active_palette.as_ref().map(|palette| palette.kind) else {
            return false;
        };
        let items = self.palette_items(kind);
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        active_palette.select_prev(&items);
        true
    }

    fn append_palette_query(&mut self, value: char) -> bool {
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        active_palette.query.push(value);
        active_palette.selected_index = 0;
        true
    }

    fn pop_palette_query(&mut self) -> bool {
        let Some(active_palette) = &mut self.active_palette else {
            return false;
        };

        active_palette.query.pop();
        active_palette.selected_index = 0;
        true
    }

    fn root_focus_handle(&mut self, cx: &mut Context<Self>) -> FocusHandle {
        if let Some(focus_handle) = &self.focus_handle {
            return focus_handle.clone();
        }

        let focus_handle = cx.focus_handle();
        self.focus_handle = Some(focus_handle.clone());
        focus_handle
    }

    fn reset_palette_input(&mut self) {
        self.palette_input = None;
        self.palette_input_subscription = None;
        self.palette_input_needs_focus = false;
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
        self.settings_terminal_theme_select = None;
        self.settings_terminal_theme_select_subscription = None;
        self.settings_editor_language_select = None;
        self.settings_editor_language_select_subscription = None;
        self.settings_font_family_select = None;
        self.settings_font_family_select_subscription = None;
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
    ) -> Result<Option<WorkItemId>, RootViewError> {
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

    fn apply_active_work_item(&mut self, item: &WorkItemId) -> Result<(), RootViewError> {
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

    fn reconcile_active_terminal_with_workspace(&mut self) -> Result<(), RootViewError> {
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

    fn refresh_theme_runtime_from_settings(&mut self) {
        match load_theme_store(&self.config_paths) {
            Ok(loaded) => {
                self.theme_runtime = ThemeRuntime::resolve(&self.app_settings, &loaded.store);
            }
            Err(error) => {
                self.load_error = Some(error.to_string());
            }
        }
    }

    fn save_app_settings_and_refresh_runtime(&mut self) -> Result<(), RootViewError> {
        save_settings(&self.config_paths, &self.app_settings)?;
        self.refresh_theme_runtime_from_settings();
        Ok(())
    }

    fn sync_terminal_pane_configs(&mut self, cx: &mut Context<Self>) {
        let terminal_config = self.theme_runtime.to_terminal_config();
        let theme = self.theme_runtime.ui;
        for pane in self.terminal_panes.values() {
            pane.update(cx, |pane, cx| {
                pane.update_terminal_appearance(terminal_config.clone(), theme, cx);
            });
        }
    }

    fn sync_gpui_component_theme(&self, cx: &mut Context<Self>) {
        ComponentTheme::global_mut(cx).apply_config(&Rc::new(
            self.theme_runtime.to_gpui_component_theme_config(),
        ));
    }

    fn save_keybindings_editor(&mut self) -> Result<(), RootViewError> {
        self.keybindings_editor.save(&self.config_paths)?;
        self.keybinding_warning_lines.clear();
        Ok(())
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

    fn resolved_terminal_shell(&self) -> String {
        let candidates = detect_shell_candidates();
        resolve_default_shell(&self.app_settings.terminal.shell, &candidates)
    }

    fn available_theme_names(&self) -> Vec<String> {
        load_theme_store(&self.config_paths)
            .map(|loaded| loaded.store.theme_names())
            .unwrap_or_else(|_| ThemeStore::builtin().theme_names())
    }

    fn available_editor_language_names(&self) -> Vec<String> {
        EditorLanguageCatalog::builtin()
            .all_languages()
            .iter()
            .map(|language| language.id.as_str().to_string())
            .collect()
    }

    fn settings_number_value(&self, field: SettingsNumberField) -> String {
        let terminal = &self.app_settings.terminal;
        match field {
            SettingsNumberField::FontSize => format!("{:.1}", terminal.font_size),
            SettingsNumberField::LineHeight => format!("{:.2}", terminal.line_height),
            SettingsNumberField::Padding => format!("{:.1}", terminal.padding),
            SettingsNumberField::Scrollback => terminal.scrollback.to_string(),
        }
    }

    fn apply_settings_number_value(
        &mut self,
        field: SettingsNumberField,
        value: &str,
    ) -> Result<(), RootViewError> {
        let value = value.trim();
        match field {
            SettingsNumberField::FontSize => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_terminal_font_size(value)?;
                }
            }
            SettingsNumberField::LineHeight => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_terminal_line_height(value)?;
                }
            }
            SettingsNumberField::Padding => {
                if let Ok(value) = value.parse::<f32>() {
                    self.set_terminal_padding(value)?;
                }
            }
            SettingsNumberField::Scrollback => {
                if let Ok(value) = value.parse::<usize>() {
                    self.set_terminal_scrollback(value)?;
                }
            }
        }
        Ok(())
    }

    fn stepped_settings_number_value(
        &self,
        field: SettingsNumberField,
        value: &str,
        action: StepAction,
    ) -> String {
        let sign = match action {
            StepAction::Increment => 1.0,
            StepAction::Decrement => -1.0,
        };
        match field {
            SettingsNumberField::FontSize => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.terminal.font_size);
                format!("{:.1}", (value + sign).max(1.0))
            }
            SettingsNumberField::LineHeight => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.terminal.line_height);
                format!("{:.2}", (value + sign * 0.05).max(0.5))
            }
            SettingsNumberField::Padding => {
                let value = value
                    .trim()
                    .parse::<f32>()
                    .unwrap_or(self.app_settings.terminal.padding);
                format!("{:.1}", (value + sign).max(0.0))
            }
            SettingsNumberField::Scrollback => {
                let value = value
                    .trim()
                    .parse::<isize>()
                    .unwrap_or(self.app_settings.terminal.scrollback as isize);
                ((value + (sign as isize) * 1000).max(1000)).to_string()
            }
        }
    }

    fn settings_number_field_for_input(
        &self,
        input: &Entity<InputState>,
    ) -> Option<SettingsNumberField> {
        self.settings_number_inputs
            .iter()
            .find_map(|(field, entity)| (entity.entity_id() == input.entity_id()).then_some(*field))
    }

    fn palette_input_contains_focus(&self, window: &Window, cx: &Context<Self>) -> bool {
        self.palette_input
            .as_ref()
            .is_some_and(|input| input.read(cx).focus_handle(cx).contains_focused(window, cx))
    }

    fn selected_focused_pane_id(&self) -> Option<&str> {
        let project_id = self.workspace.selected_project_id()?;
        let project = self.workspace.project(project_id)?;
        project
            .tab_state(&project.selected_tab_id)
            .and_then(|tab| tab.focused_pane_id.as_deref())
    }

    fn palette_query_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let active_palette = self.active_palette.as_ref()?;
        let input = if let Some(input) = &self.palette_input {
            input.clone()
        } else {
            let placeholder = palette_input_placeholder(active_palette.kind, &self.ui_text);
            let query = active_palette.query.clone();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(placeholder)
                    .default_value(query)
            });
            let subscription = cx.subscribe_in(&input, window, Self::on_palette_input_event);
            self.palette_input = Some(input.clone());
            self.palette_input_subscription = Some(subscription);
            input
        };

        if self.palette_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.palette_input_needs_focus = false;
        }

        Some(input)
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

    fn settings_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        if !self.settings_page.is_open {
            return None;
        }

        let input = if let Some(input) = &self.settings_search_input {
            input.clone()
        } else {
            let query = self.settings_page.search_query.clone();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder(self.ui_text.get(UiTextKey::SettingsSearchPlaceholder))
                    .default_value(query)
            });
            let subscription =
                cx.subscribe_in(&input, window, Self::on_settings_search_input_event);
            self.settings_search_input = Some(input.clone());
            self.settings_search_input_subscription = Some(subscription);
            input
        };

        if self.settings_search_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.settings_search_input_needs_focus = false;
        }

        Some(input)
    }

    fn settings_language_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let items = language_setting_labels();
        let selected = language_setting_label(self.app_settings.general.language).to_string();

        if let Some(select) = &self.settings_language_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx
                .new(|cx| SelectState::new(SearchableVec::new(items), selected_index, window, cx));
            let subscription =
                cx.subscribe_in(&select, window, Self::on_settings_language_select_event);
            self.settings_language_select = Some(select.clone());
            self.settings_language_select_subscription = Some(subscription);
            select
        }
    }

    fn settings_shell_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let mut items = vec!["Auto".to_string()];
        for shell in detect_shell_candidates() {
            push_unique_string(&mut items, shell);
        }
        let selected = if self.app_settings.terminal.shell == crate::config::settings::AUTO_SHELL {
            "Auto".to_string()
        } else {
            self.app_settings.terminal.shell.clone()
        };
        if selected != "Auto" {
            push_unique_string(&mut items, selected.clone());
        }

        if let Some(select) = &self.settings_shell_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx
                .new(|cx| SelectState::new(SearchableVec::new(items), selected_index, window, cx));
            let subscription =
                cx.subscribe_in(&select, window, Self::on_settings_shell_select_event);
            self.settings_shell_select = Some(select.clone());
            self.settings_shell_select_subscription = Some(subscription);
            select
        }
    }

    fn settings_ui_theme_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let items = self.available_theme_names();
        let selected = self.theme_runtime.theme_name.clone();

        if let Some(select) = &self.settings_ui_theme_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), selected_index, window, cx)
                    .searchable(true)
            });
            let subscription =
                cx.subscribe_in(&select, window, Self::on_settings_ui_theme_select_event);
            self.settings_ui_theme_select = Some(select.clone());
            self.settings_ui_theme_select_subscription = Some(subscription);
            select
        }
    }

    fn settings_terminal_theme_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let mut items = vec![TERMINAL_THEME_FOLLOW_UI.to_string()];
        for theme_name in self.available_theme_names() {
            push_unique_string(&mut items, theme_name);
        }
        let selected = self
            .app_settings
            .theme
            .terminal
            .clone()
            .unwrap_or_else(|| TERMINAL_THEME_FOLLOW_UI.to_string());

        if let Some(select) = &self.settings_terminal_theme_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), selected_index, window, cx)
                    .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                Self::on_settings_terminal_theme_select_event,
            );
            self.settings_terminal_theme_select = Some(select.clone());
            self.settings_terminal_theme_select_subscription = Some(subscription);
            select
        }
    }

    fn settings_editor_language_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let mut items = self.available_editor_language_names();
        let selected = self.app_settings.editor.default_language.clone();
        push_unique_string(&mut items, selected.clone());

        if let Some(select) = &self.settings_editor_language_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), selected_index, window, cx)
                    .searchable(true)
            });
            let subscription = cx.subscribe_in(
                &select,
                window,
                Self::on_settings_editor_language_select_event,
            );
            self.settings_editor_language_select = Some(select.clone());
            self.settings_editor_language_select_subscription = Some(subscription);
            select
        }
    }

    fn settings_font_family_select(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<SettingsStringSelectState> {
        let selected =
            terminal_font_family_option_for_setting(&self.app_settings.terminal.font_family);
        let items = terminal_font_family_options_from_system(
            &self.app_settings.terminal.font_family,
            cx.text_system().all_font_names(),
        );

        if let Some(select) = &self.settings_font_family_select {
            select.clone()
        } else {
            let selected_index = selected_index_for_settings_option(&items, &selected);
            let select = cx.new(|cx| {
                SelectState::new(SearchableVec::new(items), selected_index, window, cx)
                    .searchable(true)
            });
            let subscription =
                cx.subscribe_in(&select, window, Self::on_settings_font_family_select_event);
            self.settings_font_family_select = Some(select.clone());
            self.settings_font_family_select_subscription = Some(subscription);
            select
        }
    }

    fn settings_number_input(
        &mut self,
        field: SettingsNumberField,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = self.settings_number_inputs.get(&field) {
            return input.clone();
        }

        let value = self.settings_number_value(field);
        let input = cx.new(|cx| InputState::new(window, cx).default_value(value));
        let input_subscription =
            cx.subscribe_in(&input, window, Self::on_settings_number_input_event);
        let step_subscription =
            cx.subscribe_in(&input, window, Self::on_settings_number_step_event);
        self.settings_number_inputs.insert(field, input.clone());
        self.settings_number_input_subscriptions
            .insert(field, vec![input_subscription, step_subscription]);
        input
    }

    fn layout_toml_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<Entity<InputState>> {
        let editor = self.layout_toml_editor.as_ref()?.editor();

        let input = if let Some(input) = &self.layout_toml_input {
            input.clone()
        } else {
            let input = cx.new(|cx| code_editor_input_state(window, cx, editor));
            let subscription = cx.subscribe_in(&input, window, Self::on_layout_toml_input_event);
            self.layout_toml_input = Some(input.clone());
            self.layout_toml_input_subscription = Some(subscription);
            input
        };

        if self.layout_toml_input_needs_focus {
            input.update(cx, |input, cx| input.focus(window, cx));
            self.layout_toml_input_needs_focus = false;
        }

        Some(input)
    }

    fn active_terminal_split_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        self.prune_terminal_panes();

        let Some((project_id, project_path, project_title, tab_id, tab_title, layout)) =
            self.selected_tab_layout_clone()
        else {
            return project_empty_terminal_state(cx, &self.ui_text, &self.theme_runtime.ui);
        };

        let focused_pane_id = self.selected_focused_pane_id().map(ToOwned::to_owned);
        let tree_input = RenderTerminalTreeInput {
            project_id: &project_id,
            project_path: &project_path,
            project_title: &project_title,
            tab_id: &tab_id,
            tab_title: &tab_title,
            focused_pane_id: focused_pane_id.as_deref(),
        };

        div()
            .flex()
            .flex_1()
            .bg(self.theme_runtime.ui.terminal_background)
            .text_color(self.theme_runtime.ui.text)
            .child(self.terminal_split_view_for_layout(&layout, &tree_input, window, cx))
    }

    fn active_work_item_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        let Some(WorkItemId::File(document_id)) = self.active_work_item() else {
            return self.active_terminal_split_view(window, cx);
        };
        let document = self.project_editor_runtime.document(&document_id).cloned();
        if self.pending_editor_focus_document_id.as_ref() == Some(&document_id)
            && self.foreground_input_owner_kind() == InputOwnerKind::Editor
        {
            if let Some(document) = &document {
                document.update(cx, |document, document_cx| {
                    document.focus(window, document_cx);
                });
                self.pending_editor_focus_document_id = None;
            }
        }

        div()
            .debug_selector(|| "active-file-editor".to_string())
            .flex()
            .flex_1()
            .bg(self.theme_runtime.ui.surface)
            .children(document)
    }

    fn workbench_tab_items(&self, cx: &Context<Self>) -> Vec<WorkbenchTabItem> {
        let terminal_items = visible_tab_items(&self.workspace);
        let Some(project_id) = self.workspace.selected_project_id() else {
            return Vec::new();
        };
        let Some(project) = self.workspace.project(project_id) else {
            return Vec::new();
        };
        let file_items = self
            .project_editor_runtime
            .workspace()
            .session(project_id)
            .map(|session| {
                session
                    .file_ids()
                    .iter()
                    .map(|document_id| FileTabSnapshot {
                        id: document_id.clone(),
                        relative_path: document_id
                            .canonical_path
                            .strip_prefix(&project.path)
                            .unwrap_or(&document_id.canonical_path)
                            .to_path_buf(),
                        dirty: self
                            .project_editor_runtime
                            .document(document_id)
                            .is_some_and(|document| document.read(cx).model().is_dirty()),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let active = self.active_work_item();
        merge_work_item_tabs(&terminal_items, &file_items, active.as_ref())
    }

    pub fn selected_project_panel_visible(&self) -> bool {
        let Some(project_id) = self.workspace.selected_project_id() else {
            return false;
        };
        self.project_editor_runtime
            .workspace()
            .session(project_id)
            .is_some_and(|session| session.project_panel_visible())
    }

    fn project_file_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Option<Div> {
        let project_id = self.workspace.selected_project_id()?.clone();
        let project_name = self
            .workspace
            .project(&project_id)?
            .layout
            .project
            .name
            .clone();
        let tree = self.ensure_project_tree_view(&project_id, window, cx)?;
        let session = self
            .project_editor_runtime
            .workspace()
            .session(&project_id)?;
        let panel_width = session.project_panel_width();
        let root_load_state = session.file_tree().directory_load_state(Path::new(""));
        let root_is_empty = session.file_tree().visible_rows().is_empty();
        let has_root_snapshot = session.file_tree().has_snapshot(Path::new(""));
        let theme = self.theme_runtime.ui;

        let content = match root_load_state {
            ProjectTreeLoadState::Loading | ProjectTreeLoadState::Unloaded
                if !has_root_snapshot =>
            {
                div()
                    .debug_selector(|| "project-file-panel-loading".to_string())
                    .flex()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .px_4()
                    .text_sm()
                    .text_color(theme.text_subtle)
                    .child(self.ui_text.get(UiTextKey::ProjectFilesLoading))
            }
            ProjectTreeLoadState::Error(error) if !has_root_snapshot => {
                let retry_project_id = project_id.clone();
                div()
                    .debug_selector(|| "project-file-panel-error".to_string())
                    .flex()
                    .flex_col()
                    .flex_1()
                    .items_center()
                    .justify_center()
                    .gap_3()
                    .px_4()
                    .text_center()
                    .text_sm()
                    .text_color(theme.text_muted)
                    .child(error)
                    .child(
                        yttt_button(
                            "project-file-panel-retry",
                            self.ui_text.get(UiTextKey::ProjectFilesRetry),
                            YtttButtonVariant::Secondary,
                            theme,
                            cx,
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                this.refresh_project_tree(retry_project_id.clone(), window, cx);
                                cx.notify();
                            },
                        )),
                    )
            }
            ProjectTreeLoadState::Loaded if root_is_empty => div()
                .debug_selector(|| "project-file-panel-empty".to_string())
                .flex()
                .flex_1()
                .items_center()
                .justify_center()
                .px_4()
                .text_sm()
                .text_color(theme.text_subtle)
                .child(self.ui_text.get(UiTextKey::ProjectFilesEmptyDirectory)),
            _ => div()
                .debug_selector(|| "project-file-tree".to_string())
                .flex()
                .flex_1()
                .overflow_hidden()
                .child(tree),
        };

        let refresh_project_id = project_id;
        let resize_handle = self.sidebar_resize_handle(SidebarSide::Right, cx);
        Some(
            div()
                .debug_selector(|| "project-file-panel".to_string())
                .flex()
                .flex_col()
                .flex_none()
                .relative()
                .h_full()
                .w(px(panel_width))
                .overflow_hidden()
                .bg(theme.sidebar_background)
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_between()
                        .h(px(40.0))
                        .flex_none()
                        .border_b_1()
                        .border_color(theme.border)
                        .px_3()
                        .child(
                            div()
                                .flex()
                                .flex_col()
                                .overflow_hidden()
                                .child(
                                    div()
                                        .text_sm()
                                        .font_weight(FontWeight::MEDIUM)
                                        .truncate()
                                        .child(self.ui_text.get(UiTextKey::ProjectFiles)),
                                )
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(theme.text_subtle)
                                        .truncate()
                                        .child(project_name),
                                ),
                        )
                        .child(
                            yttt_button(
                                "project-file-panel-refresh",
                                self.ui_text.get(UiTextKey::ProjectFilesRefresh),
                                YtttButtonVariant::Ghost,
                                theme,
                                cx,
                            )
                            .on_click(cx.listener(
                                move |this, _, window, cx| {
                                    this.refresh_project_tree(
                                        refresh_project_id.clone(),
                                        window,
                                        cx,
                                    );
                                    cx.notify();
                                },
                            )),
                        ),
                )
                .child(content)
                .child(resize_handle),
        )
    }

    fn terminal_split_view_for_layout(
        &mut self,
        layout: &LayoutNode,
        tree_input: &RenderTerminalTreeInput<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        match layout {
            LayoutNode::Pane(pane) => self.render_terminal_pane(
                RenderTerminalPaneInput {
                    project_id: tree_input.project_id,
                    project_path: tree_input.project_path,
                    project_title: tree_input.project_title,
                    pane,
                    tab_id: tree_input.tab_id,
                    tab_title: tree_input.tab_title,
                    is_focused: tree_input.focused_pane_id == Some(pane.id.as_str()),
                },
                window,
                cx,
            ),
            LayoutNode::Split(split) => {
                let basis = split_child_basis(split.ratio);
                let mut container = div().flex().flex_1();
                if split.direction == SplitDirection::Vertical {
                    container = container.flex_col();
                }

                let left = self.terminal_split_view_for_layout(&split.left, tree_input, window, cx);
                let right =
                    self.terminal_split_view_for_layout(&split.right, tree_input, window, cx);

                container
                    .child(split_child(left, basis.left))
                    .child(self.split_resize_handle(split.direction, cx))
                    .child(split_child(right, basis.right))
            }
        }
    }

    fn selected_tab_layout_clone(
        &self,
    ) -> Option<(String, PathBuf, String, String, String, LayoutNode)> {
        let selected_project_id = self.workspace.selected_project_id()?;
        let project = self.workspace.project(selected_project_id)?;
        let tab = project
            .layout
            .tabs
            .iter()
            .find(|tab| tab.id == project.selected_tab_id)?;

        Some((
            selected_project_id.as_str().to_string(),
            project.path.clone(),
            project.layout.project.name.clone(),
            project.selected_tab_id.clone(),
            tab.title.clone(),
            tab.layout.clone(),
        ))
    }

    fn render_terminal_pane(
        &mut self,
        input: RenderTerminalPaneInput<'_>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Div {
        let key = terminal_pane_key(input.project_id, input.tab_id, &input.pane.id);
        let pane_view = if let Some(pane_view) = self.terminal_panes.get(&key) {
            pane_view.clone()
        } else {
            let context = TerminalPaneContext {
                project_id: input.project_id.to_string(),
                project_path: input.project_path.to_path_buf(),
                project_title: input.project_title.to_string(),
                tab_id: input.tab_id.to_string(),
                tab_title: input.tab_title.to_string(),
                pane: input.pane.clone(),
                is_focused: input.is_focused,
                terminal_input_gate: self.terminal_input_gate.clone(),
            };
            let terminal_config = self.theme_runtime.to_terminal_config();
            let theme = self.theme_runtime.ui;
            let pane_view = cx.new(|cx| TerminalPaneView::new(context, terminal_config, theme, cx));
            let subscription = cx.subscribe_in(&pane_view, window, Self::on_terminal_pane_event);
            self.terminal_pane_subscriptions
                .insert(key.clone(), subscription);
            self.terminal_panes.insert(key, pane_view.clone());
            pane_view
        };

        let pane_id = input.pane.id.clone();
        if self
            .pending_terminal_focus_pane_id
            .as_deref()
            .is_some_and(|pending| pending == pane_id)
            && self.should_auto_focus_workspace()
            && pane_view.update(cx, |pane, cx| pane.focus_terminal(window, cx))
        {
            self.pending_terminal_focus_pane_id = None;
        }

        let border_color = if input.is_focused {
            self.theme_runtime.ui.focused_pane_border
        } else {
            rgba(0x00000000)
        };
        let terminal_input_allowed = self.terminal_input_allowed();
        let mut wrapper = div()
            .flex()
            .flex_1()
            .relative()
            .border_1()
            .border_color(border_color)
            .bg(self.theme_runtime.ui.terminal_background);
        wrapper.interactivity().on_mouse_down(
            MouseButton::Left,
            cx.listener(move |this, _, _window, cx| {
                if !this.terminal_input_allowed() {
                    cx.stop_propagation();
                    return;
                }
                let _ = this.focus_visible_terminal_pane(&pane_id);
                cx.notify();
            }),
        );
        wrapper = wrapper.child(pane_view);
        if !terminal_input_allowed {
            wrapper = wrapper.child(
                div()
                    .absolute()
                    .inset_0()
                    .bg(rgba(0x00000000))
                    .on_mouse_down(MouseButton::Left, |_event, _window, cx| {
                        cx.stop_propagation();
                    }),
            );
        }
        wrapper
    }

    fn prune_terminal_panes(&mut self) {
        let mut live_keys = HashSet::new();
        for project in self.workspace.opened_projects() {
            for tab in &project.layout.tabs {
                collect_terminal_pane_keys(
                    project.id.as_str(),
                    &tab.id,
                    &tab.layout,
                    &mut live_keys,
                );
            }
        }

        self.terminal_panes
            .retain(|key, _pane| live_keys.contains(key));
        self.terminal_pane_subscriptions
            .retain(|key, _subscription| live_keys.contains(key));
    }

    fn on_terminal_pane_event(
        &mut self,
        _pane: &Entity<TerminalPaneView>,
        event: &TerminalPaneEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TerminalPaneEvent::Notification(event) => {
                let root = cx.entity();
                let event = event.clone();
                self.handle_terminal_notification(event.clone());
                push_component_notification(root, event, _window, cx);
                cx.notify();
            }
            TerminalPaneEvent::Exited(event) => {
                if let Err(error) = self.handle_terminal_pane_exit(event.clone()) {
                    self.load_error = Some(error.to_string());
                }
                cx.notify();
            }
        }
    }

    fn split_resize_handle(&self, direction: SplitDirection, cx: &mut Context<Self>) -> AnyElement {
        let style = Self::visible_split_handle_style(direction);
        let theme = self.theme_runtime.ui;
        let mut handle = div()
            .id(match direction {
                SplitDirection::Horizontal => "horizontal-split-resize-handle",
                SplitDirection::Vertical => "vertical-split-resize-handle",
            })
            .flex()
            .items_center()
            .justify_center()
            .flex_none()
            .bg(rgba(0x00000000))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.begin_split_resize_drag(direction, event.position);
                    cx.stop_propagation();
                }),
            );

        handle = match direction {
            SplitDirection::Horizontal => handle.w(style.hit_area_width).cursor_ew_resize().child(
                div()
                    .w(style.visible_line_width)
                    .h_full()
                    .bg(theme.split_line),
            ),
            SplitDirection::Vertical => handle.h(style.hit_area_width).cursor_ns_resize().child(
                div()
                    .h(style.visible_line_width)
                    .w_full()
                    .bg(theme.split_line),
            ),
        };

        handle.into_any_element()
    }

    fn sidebar_resize_handle(&self, side: SidebarSide, cx: &mut Context<Self>) -> AnyElement {
        let (id, offset) = match side {
            SidebarSide::Left => ("project-sidebar-resize-handle", px(0.0)),
            SidebarSide::Right => ("project-file-panel-resize-handle", px(0.0)),
        };
        let line_color = if self
            .active_sidebar_resize_drag
            .is_some_and(|drag| drag.side == side)
        {
            self.theme_runtime.ui.split_line_active
        } else {
            self.theme_runtime.ui.split_line
        };
        let handle = div()
            .id(id)
            .debug_selector(move || id.to_string())
            .absolute()
            .top_0()
            .bottom_0()
            .flex()
            .items_center()
            .justify_center()
            .w(px(SIDEBAR_RESIZE_HIT_AREA_WIDTH))
            .cursor_ew_resize()
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(move |this, event: &MouseDownEvent, _window, cx| {
                    this.begin_sidebar_resize_drag(side, event.position);
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .child(div().h_full().w(px(1.0)).bg(line_color));

        match side {
            SidebarSide::Left => handle.right(offset),
            SidebarSide::Right => handle.left(offset),
        }
        .into_any_element()
    }

    pub fn visible_split_handle_style(_direction: SplitDirection) -> SplitHandleStyle {
        let theme = WorkbenchTheme::dark();
        SplitHandleStyle {
            visible_line_width: theme.split_line_width,
            hit_area_width: theme.split_hit_area_width,
        }
    }

    fn begin_split_resize_drag(&mut self, direction: SplitDirection, position: Point<Pixels>) {
        self.active_sidebar_resize_drag = None;
        self.active_split_resize_drag = Some(ActiveSplitResizeDrag {
            direction,
            last_position: position,
        });
    }

    fn begin_sidebar_resize_drag(&mut self, side: SidebarSide, position: Point<Pixels>) {
        self.active_split_resize_drag = None;
        self.active_sidebar_resize_drag = Some(ActiveSidebarResizeDrag {
            side,
            last_position: position,
        });
    }

    fn resize_from_sidebar_drag(&mut self, side: SidebarSide, position: Point<Pixels>) {
        let Some(active_drag) = self.active_sidebar_resize_drag else {
            self.begin_sidebar_resize_drag(side, position);
            return;
        };
        if active_drag.side != side {
            self.begin_sidebar_resize_drag(side, position);
            return;
        }

        let delta_x = f32::from(position.x - active_drag.last_position.x);
        self.resize_sidebar_from_pointer_delta(side, delta_x);
        self.begin_sidebar_resize_drag(side, position);
    }

    fn resize_from_split_drag(&mut self, direction: SplitDirection, position: Point<Pixels>) {
        let Some(active_drag) = self.active_split_resize_drag else {
            self.begin_split_resize_drag(direction, position);
            return;
        };
        if active_drag.direction != direction {
            self.begin_split_resize_drag(direction, position);
            return;
        }

        let delta_x = f32::from(position.x - active_drag.last_position.x);
        let delta_y = f32::from(position.y - active_drag.last_position.y);
        match self.resize_focused_split_from_pointer_delta(direction, delta_x, delta_y) {
            Ok(Some(_)) => self.begin_split_resize_drag(direction, position),
            Ok(None) => {}
            Err(error) => {
                self.load_error = Some(error.to_string());
                self.begin_split_resize_drag(direction, position);
            }
        }
    }

    fn on_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(active_drag) = self.active_sidebar_resize_drag {
            if !event.dragging() {
                self.active_sidebar_resize_drag = None;
                cx.notify();
                return;
            }

            self.resize_from_sidebar_drag(active_drag.side, event.position);
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let Some(active_drag) = self.active_split_resize_drag else {
            cx.propagate();
            return;
        };

        if !event.dragging() {
            self.active_split_resize_drag = None;
            cx.notify();
            return;
        }

        self.resize_from_split_drag(active_drag.direction, event.position);
        cx.stop_propagation();
        cx.notify();
    }

    fn on_resize_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(active_drag) = self.active_sidebar_resize_drag.take() {
            if let Err(error) = self.persist_sidebar_width(active_drag.side) {
                self.load_error = Some(error.to_string());
            }
            cx.stop_propagation();
            cx.notify();
            return;
        }
        if self.active_split_resize_drag.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_palette_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                let query = input.read(cx).value().to_string();
                if self.sync_palette_query_from_input_value(query) {
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } => {
                let _ = self.confirm_palette_selection();
                self.handle_pending_open_project_request(cx);
                self.flush_pending_status_notifications(window, cx);
                cx.notify();
            }
            InputEvent::Focus | InputEvent::Blur => {}
        }
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

    fn on_settings_search_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                self.set_settings_search_query(input.read(cx).value().to_string());
                cx.notify();
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn on_settings_language_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let Some(language) = language_setting_from_label(value) else {
            return;
        };
        if let Err(error) = self.set_language(language) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    fn on_settings_shell_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let shell = if value == "Auto" {
            crate::config::settings::AUTO_SHELL
        } else {
            value.as_str()
        };
        if let Err(error) = self.set_terminal_shell(shell) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    fn on_settings_ui_theme_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        if let Err(error) = self.set_ui_theme_name(value) {
            self.load_error = Some(error.to_string());
        }
        self.sync_gpui_component_theme(cx);
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    fn on_settings_terminal_theme_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let terminal_theme = (value != TERMINAL_THEME_FOLLOW_UI).then_some(value.as_str());
        if let Err(error) = self.set_terminal_theme_name(terminal_theme) {
            self.load_error = Some(error.to_string());
        }
        self.sync_gpui_component_theme(cx);
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    fn on_settings_editor_language_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        if let Err(error) = self.set_editor_default_language(value) {
            self.load_error = Some(error.to_string());
        }
        cx.notify();
    }

    fn on_settings_font_family_select_event(
        &mut self,
        _select: &Entity<SettingsStringSelectState>,
        event: &SelectEvent<SearchableVec<String>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let SelectEvent::Confirm(Some(value)) = event else {
            return;
        };
        let font_family = terminal_font_family_setting_from_option(value);
        if let Err(error) = self.set_terminal_font_family(&font_family) {
            self.load_error = Some(error.to_string());
        }
        self.sync_gpui_component_theme(cx);
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    fn on_settings_number_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change | InputEvent::PressEnter { .. } | InputEvent::Blur => {
                let Some(field) = self.settings_number_field_for_input(input) else {
                    return;
                };
                let value = input.read(cx).value().to_string();
                if let Err(error) = self.apply_settings_number_value(field, &value) {
                    self.load_error = Some(error.to_string());
                }
                self.sync_gpui_component_theme(cx);
                self.sync_terminal_pane_configs(cx);
                cx.notify();
            }
            InputEvent::Focus => {}
        }
    }

    fn on_settings_number_step_event(
        &mut self,
        input: &Entity<InputState>,
        event: &NumberInputEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(field) = self.settings_number_field_for_input(input) else {
            return;
        };
        let NumberInputEvent::Step(action) = event;
        let value = input.read(cx).value().to_string();
        let stepped = self.stepped_settings_number_value(field, &value, *action);
        input.update(cx, |input, cx| {
            input.set_value(stepped.clone(), window, cx);
        });
        if let Err(error) = self.apply_settings_number_value(field, &stepped) {
            self.load_error = Some(error.to_string());
        }
        self.sync_gpui_component_theme(cx);
        self.sync_terminal_pane_configs(cx);
        cx.notify();
    }

    fn on_layout_toml_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            InputEvent::Change => {
                if let Some(session) = &mut self.layout_toml_editor {
                    session
                        .editor_mut()
                        .set_value(input.read(cx).value().to_string());
                    cx.notify();
                }
            }
            InputEvent::PressEnter { .. } | InputEvent::Focus | InputEvent::Blur => {}
        }
    }

    fn confirm_tab_rename_dialog_from_input(
        &mut self,
        cx: &mut Context<Self>,
    ) -> Result<(), RootViewError> {
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
    ) -> Result<(), RootViewError> {
        let value = self
            .keybinding_edit_input
            .as_ref()
            .map(|input| input.read(cx).value().to_string())
            .or_else(|| self.pending_keybinding_edit_value())
            .unwrap_or_default();

        self.confirm_keybinding_edit_dialog(&value)
    }

    fn on_open_command_palette(
        &mut self,
        _: &OpenCommandPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Command);
        cx.notify();
    }

    fn on_open_project_palette(
        &mut self,
        _: &OpenProjectPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Project);
        cx.notify();
    }

    fn on_open_project(&mut self, _: &OpenProject, _window: &mut Window, cx: &mut Context<Self>) {
        self.request_open_project();
        self.handle_pending_open_project_request(cx);
        cx.notify();
    }

    fn prompt_for_project_directory(&mut self, cx: &mut Context<Self>) {
        let picked_paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some("Open Directory".into()),
        });

        cx.spawn(async move |this, cx| match picked_paths.await {
            Ok(Ok(Some(paths))) => {
                if let Some(project_path) = paths.into_iter().next() {
                    let _ = this.update(cx, |this, cx| {
                        let _ = this.open_project_path(project_path);
                        cx.notify();
                    });
                }
            }
            Ok(Ok(None)) => {}
            Ok(Err(error)) => {
                let _ = this.update(cx, |this, cx| {
                    this.load_error = Some(format!("Failed to open directory picker: {error}"));
                    cx.notify();
                });
            }
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    this.load_error = Some(format!("Directory picker was interrupted: {error}"));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn on_open_tab_palette(
        &mut self,
        _: &OpenTabPalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Tab);
        cx.notify();
    }

    fn on_open_pane_palette(
        &mut self,
        _: &OpenPanePalette,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_palette(PaletteKind::Pane);
        cx.notify();
    }

    fn on_palette_select_next(
        &mut self,
        _: &PaletteSelectNext,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.select_next_palette_item() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_palette_select_prev(
        &mut self,
        _: &PaletteSelectPrev,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.select_prev_palette_item() {
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_palette_confirm(
        &mut self,
        _: &PaletteConfirm,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending_tab_rename.is_some() {
            let _ = self.confirm_tab_rename_dialog_from_input(cx);
            cx.notify();
            return;
        }

        if self.pending_close_project_id.is_some() {
            let _ = self.confirm_pending_project_close();
            cx.notify();
            return;
        }

        if self.active_palette.is_some() {
            let _ = self.confirm_palette_selection();
            self.handle_pending_open_project_request(cx);
            self.flush_pending_status_notifications(window, cx);
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_palette_cancel(
        &mut self,
        _: &PaletteCancel,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.layout_toml_editor.is_some() {
            self.cancel_layout_toml_editor();
            cx.notify();
            return;
        }

        if self.pending_tab_rename.is_some() {
            self.cancel_tab_rename_dialog();
            cx.notify();
            return;
        }

        if self.pending_close_project_id.is_some() {
            self.cancel_pending_project_close();
            cx.notify();
            return;
        }

        if self.active_palette.is_some() {
            self.close_palette();
            cx.notify();
        } else if self.settings_page.is_open {
            self.close_settings();
            cx.notify();
        } else {
            cx.propagate();
        }
    }

    fn on_tab_new(&mut self, _: &TabNew, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::TabNew, cx);
    }

    fn on_project_close(&mut self, _: &ProjectClose, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::ProjectClose, cx);
    }

    fn on_tab_close(&mut self, _: &TabClose, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::TabClose, cx);
    }

    fn on_tab_rename(&mut self, _: &TabRename, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::TabRename, cx);
    }

    fn on_tab_next(&mut self, _: &TabNext, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::TabNext, cx);
    }

    fn on_tab_prev(&mut self, _: &TabPrev, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::TabPrev, cx);
    }

    fn on_pane_split_vertical(
        &mut self,
        _: &PaneSplitVertical,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneSplitVertical, cx);
    }

    fn on_pane_split_horizontal(
        &mut self,
        _: &PaneSplitHorizontal,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneSplitHorizontal, cx);
    }

    fn on_pane_close(&mut self, _: &PaneClose, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::PaneClose, cx);
    }

    fn on_pane_rename(&mut self, _: &PaneRename, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::PaneRename, cx);
    }

    fn on_pane_focus_left(
        &mut self,
        _: &PaneFocusLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneFocusLeft, cx);
    }

    fn on_pane_focus_right(
        &mut self,
        _: &PaneFocusRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneFocusRight, cx);
    }

    fn on_pane_focus_up(&mut self, _: &PaneFocusUp, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::PaneFocusUp, cx);
    }

    fn on_pane_focus_down(
        &mut self,
        _: &PaneFocusDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneFocusDown, cx);
    }

    fn on_pane_resize_left(
        &mut self,
        _: &PaneResizeLeft,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneResizeLeft, cx);
    }

    fn on_pane_resize_right(
        &mut self,
        _: &PaneResizeRight,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneResizeRight, cx);
    }

    fn on_pane_resize_up(
        &mut self,
        _: &PaneResizeUp,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneResizeUp, cx);
    }

    fn on_pane_resize_down(
        &mut self,
        _: &PaneResizeDown,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::PaneResizeDown, cx);
    }

    fn on_layout_save_current(
        &mut self,
        _: &LayoutSaveCurrent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutSaveCurrent, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    fn on_layout_default_edit(
        &mut self,
        _: &LayoutDefaultEdit,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutDefaultEdit, cx);
    }

    fn on_layout_default_reset(
        &mut self,
        _: &LayoutDefaultReset,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutDefaultReset, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    fn on_layout_default_reload(
        &mut self,
        _: &LayoutDefaultReload,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutDefaultReload, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    fn on_layout_project_edit(
        &mut self,
        _: &LayoutProjectEdit,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutProjectEdit, cx);
    }

    fn on_layout_reset_local_override(
        &mut self,
        _: &LayoutResetLocalOverride,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutResetLocalOverride, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    fn on_layout_export_project_config(
        &mut self,
        _: &LayoutExportProjectConfig,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutExportProjectConfig, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    fn on_layout_open_file(
        &mut self,
        _: &LayoutOpenFile,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutOpenFile, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    fn on_settings_keybindings(
        &mut self,
        _: &SettingsKeybindings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::SettingsKeybindings, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    fn on_file_save(&mut self, _: &FileSave, window: &mut Window, cx: &mut Context<Self>) {
        self.save_active_document(window, cx);
        cx.notify();
    }

    fn on_settings_open(&mut self, _: &SettingsOpen, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::SettingsOpen, cx);
    }

    fn on_settings_notifications(
        &mut self,
        _: &SettingsNotifications,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::SettingsNotifications, cx);
        self.flush_pending_status_notifications(window, cx);
    }

    fn on_project_editor_document_event(
        &mut self,
        document: &Entity<ProjectEditorDocument>,
        event: &ProjectEditorDocumentEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let document_id = document.read(cx).model().document_id().clone();
        match event {
            ProjectEditorDocumentEvent::Changed { generation } => {
                self.schedule_delayed_autosave(document_id, *generation, window, cx);
            }
            ProjectEditorDocumentEvent::Focused => {
                let _ = self.select_work_item(WorkItemId::File(document_id.clone()));
                self.check_document_for_external_changes(document_id, window, cx);
            }
            ProjectEditorDocumentEvent::Blurred => {
                self.queue_focus_change_autosave(document_id);
            }
        }
        cx.notify();
    }

    fn dispatch_command_action(&mut self, command_id: CommandId, cx: &mut Context<Self>) {
        if self.active_palette.is_some()
            || self.pending_tab_rename.is_some()
            || self.pending_keybinding_edit.is_some()
            || self.layout_toml_editor.is_some()
        {
            cx.propagate();
            return;
        }

        let _ = self.run_command(command_id);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        if self.layout_toml_editor.is_some() {
            cx.propagate();
            return;
        }

        if self.active_palette.is_none() {
            if let Some(command_id) = Self::workspace_arrow_keydown_command_for_owner(
                self.foreground_input_owner_kind(),
                &event.keystroke.key,
                event.keystroke.modifiers.platform,
                event.keystroke.modifiers.control,
                event.keystroke.modifiers.alt,
                event.keystroke.modifiers.shift,
            ) {
                let _ = self.run_command(command_id);
                cx.stop_propagation();
                cx.notify();
                return;
            }

            cx.propagate();
            return;
        }

        if !self.should_use_palette_text_fallback(self.palette_input_contains_focus(window, cx)) {
            cx.propagate();
            return;
        }

        if event.keystroke.key == "backspace" {
            self.pop_palette_query();
            cx.stop_propagation();
            cx.notify();
            return;
        }

        let has_command_modifier = event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.function;
        if has_command_modifier {
            cx.propagate();
            return;
        }

        let Some(key_char) = event.keystroke.key_char.as_deref() else {
            cx.propagate();
            return;
        };
        let mut chars = key_char.chars();
        let Some(value) = chars.next() else {
            cx.propagate();
            return;
        };
        if chars.next().is_none() && !value.is_control() && self.append_palette_query(value) {
            cx.stop_propagation();
            cx.notify();
        } else {
            cx.propagate();
        }
    }
}

fn read_layout_editor_source(path: &Path) -> Result<String, RootViewError> {
    fs::read_to_string(path).map_err(|source| {
        RootViewError::LayoutTomlEditor(format!(
            "failed to read layout TOML at {}: {source}",
            path.display()
        ))
    })
}

fn validate_project_editor_source(
    path: &Path,
    format: ProjectLayoutEditorFormat,
    source: &str,
) -> Result<(), (&'static str, String)> {
    match format {
        ProjectLayoutEditorFormat::ProjectConfig => {
            let layout = toml::from_str::<ProjectLayout>(source)
                .map_err(|error| ("toml", format!("failed to parse layout TOML: {error}")))?;
            layout
                .validate()
                .map_err(|error| ("layout", format!("invalid layout TOML: {error}")))
        }
        ProjectLayoutEditorFormat::PersonalPatch
        | ProjectLayoutEditorFormat::PersonalReplace
        | ProjectLayoutEditorFormat::InvalidPersonal => {
            let personal = parse_personal_layout(path, source)
                .map_err(|error| ("personal-layout", error.to_string()))?;
            match (format, personal) {
                (ProjectLayoutEditorFormat::PersonalPatch, PersonalLayout::Patch(_))
                | (ProjectLayoutEditorFormat::PersonalReplace, PersonalLayout::Replace(_))
                | (ProjectLayoutEditorFormat::InvalidPersonal, _) => Ok(()),
                (ProjectLayoutEditorFormat::PersonalPatch, PersonalLayout::Replace(_)) => Err((
                    "personal-layout",
                    "personal layout editor requires mode = \"patch\"".to_string(),
                )),
                (ProjectLayoutEditorFormat::PersonalReplace, PersonalLayout::Patch(_)) => Err((
                    "personal-layout",
                    "personal layout editor requires mode = \"replace\"".to_string(),
                )),
                (ProjectLayoutEditorFormat::ProjectConfig, _) => unreachable!(),
            }
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum RootViewError {
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

impl From<ProjectOpenError> for RootViewError {
    fn from(error: ProjectOpenError) -> Self {
        Self::ProjectOpen(Box::new(error))
    }
}

impl From<KeybindingsLoadError> for RootViewError {
    fn from(error: KeybindingsLoadError) -> Self {
        Self::Keybindings(Box::new(error))
    }
}

impl From<SettingsSaveError> for RootViewError {
    fn from(error: SettingsSaveError) -> Self {
        Self::SettingsSave(Box::new(error))
    }
}

impl From<KeybindingEditError> for RootViewError {
    fn from(error: KeybindingEditError) -> Self {
        Self::KeybindingEdit(Box::new(error))
    }
}

impl Default for RootView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.flush_pending_project_tree_loads(window, cx);
        self.flush_pending_document_saves(window, cx);
        self.flush_pending_focus_change_autosaves(window, cx);
        self.flush_pending_file_close_requests(window, cx);
        self.flush_pending_project_close_requests(cx);
        self.sync_input_owner_state();
        let focus_handle = self.root_focus_handle(cx);

        let body = if self.workspace.opened_projects().is_empty() {
            empty_workspace(cx, &self.ui_text, &self.theme_runtime.ui)
        } else {
            let tab_items = self.workbench_tab_items(cx);
            let project_panel_visible = self.selected_project_panel_visible();
            let split_view = self.active_work_item_view(window, cx);
            let project_file_panel = project_panel_visible
                .then(|| self.project_file_panel(window, cx))
                .flatten();

            let workbench = div()
                .flex()
                .flex_1()
                .relative()
                .bg(self.theme_runtime.ui.app_background)
                .text_color(self.theme_runtime.ui.text)
                .child({
                    let sidebar = project_sidebar(
                        &self.workspace,
                        self.theme_runtime.ui,
                        self.ui_text,
                        focus_handle.clone(),
                        self.app_settings.project_panel.project_sidebar_width,
                        self.sidebar_collapsed,
                        cx.listener(|this, _, _window, cx| {
                            this.toggle_sidebar();
                            cx.notify();
                        }),
                        |project_id| {
                            let project_id = ProjectId::new(project_id);
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.select_project(&project_id);
                                cx.notify();
                            })
                        },
                        |project_id| {
                            let project_id = ProjectId::new(project_id);
                            cx.listener(move |this, _: &MouseDownEvent, _window, cx| {
                                let _ = this.select_project(&project_id);
                                cx.notify();
                            })
                        },
                    );
                    let container = div().relative().flex_none().h_full().child(sidebar);
                    if self.sidebar_collapsed {
                        container
                    } else {
                        container.child(self.sidebar_resize_handle(SidebarSide::Left, cx))
                    }
                })
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .min_w_0()
                        .child(project_tabs(
                            tab_items,
                            self.theme_runtime.ui,
                            project_panel_visible,
                            self.ui_text.get(if project_panel_visible {
                                UiTextKey::ProjectFilesHide
                            } else {
                                UiTextKey::ProjectFilesShow
                            }),
                            |work_item| {
                                cx.listener(move |this, event: &ClickEvent, _window, cx| {
                                    let _ = this.handle_work_item_tab_click(
                                        work_item.clone(),
                                        event.click_count(),
                                    );
                                    cx.notify();
                                })
                            },
                            |work_item| {
                                cx.listener(move |this, _event: &ClickEvent, _window, cx| {
                                    cx.stop_propagation();
                                    let _ = this.close_work_item_tab(work_item.clone());
                                    cx.notify();
                                })
                            },
                            cx.listener(|this, _, _window, cx| {
                                let _ = this.run_command(CommandId::TabNew);
                                cx.notify();
                            }),
                            cx.listener(|this, _, _window, cx| {
                                let _ = this.run_command(CommandId::PaneSplitVertical);
                                cx.notify();
                            }),
                            cx.listener(|this, _, _window, cx| {
                                let _ = this.run_command(CommandId::PaneSplitHorizontal);
                                cx.notify();
                            }),
                            cx.listener(|this, _, _window, cx| {
                                let _ = this.run_command(CommandId::ProjectPanelToggle);
                                cx.notify();
                            }),
                        ))
                        .child(split_view),
                );
            if let Some(project_file_panel) = project_file_panel {
                workbench.child(project_file_panel)
            } else {
                workbench
            }
        };

        let mut root = div()
            .flex()
            .flex_col()
            .size_full()
            .relative()
            .bg(self.theme_runtime.ui.app_background)
            .text_color(self.theme_runtime.ui.text)
            .child(workbench_titlebar(
                self.visible_titlebar_info(),
                self.theme_runtime.ui,
            ))
            .child(body);

        if let Some(active_palette) = self.active_palette.clone() {
            let items = self.palette_items(active_palette.kind);
            if let Some(query_input) = self.palette_query_input(window, cx) {
                root = root.child(palette_overlay(
                    &active_palette,
                    &items,
                    &self.ui_text,
                    &query_input,
                    &self.palette_scroll_handle,
                    self.theme_runtime.ui,
                    |selected_index| {
                        cx.listener(move |this, _, window, cx| {
                            if let Some(active_palette) = &mut this.active_palette {
                                active_palette.selected_index = selected_index;
                            }
                            let _ = this.confirm_palette_selection();
                            this.handle_pending_open_project_request(cx);
                            this.flush_pending_status_notifications(window, cx);
                            cx.notify();
                        })
                    },
                ));
            }
        }
        if let Some(error_item) = self.visible_error_notification_item() {
            root = root.child(error_notification_overlay(
                error_item,
                self.theme_runtime.ui,
            ));
        }
        if self.settings_page.is_open {
            if let Some(search_input) = self.settings_search_input(window, cx) {
                root = root.child(settings_overlay(self, &search_input, window, cx));
            }
        }
        if self.layout_toml_editor.is_some() {
            if let Some(input) = self.layout_toml_input(window, cx) {
                root = root.child(layout_toml_editor_overlay(self, &input, cx));
            }
        }
        if self.pending_tab_rename.is_some() {
            if let Some(input) = self.tab_rename_input(window, cx) {
                root = root.child(tab_rename_dialog(
                    cx,
                    &self.ui_text,
                    &input,
                    self.theme_runtime.ui,
                ));
            }
        }
        if self.pending_keybinding_edit.is_some() {
            if let Some(input) = self.keybinding_edit_input(window, cx) {
                root = root.child(keybinding_edit_dialog(cx, &input, self.theme_runtime.ui));
            }
        }
        if let Some(text) = self.visible_dirty_close_dialog_text() {
            let mut lines = text.lines();
            let title = lines.next().unwrap_or_default().to_string();
            let details = lines.map(str::to_string).collect::<Vec<_>>();
            let file_intent = self
                .pending_dirty_close
                .as_ref()
                .is_some_and(|pending| matches!(pending.intent, DirtyCloseIntent::File(_)));
            root = root.child(dirty_close_dialog(
                cx,
                &self.ui_text,
                self.theme_runtime.ui,
                title,
                details,
                file_intent,
                self.dirty_close_has_save_error(cx),
            ));
        }
        if self.pending_close_project_id.is_some() {
            root = root.child(close_project_dialog(
                cx,
                &self.ui_text,
                self.theme_runtime.ui,
            ));
        }
        if let Some(conflict) = self.pending_file_conflict.as_ref() {
            root = root.child(file_conflict_dialog(
                cx,
                &self.ui_text,
                self.theme_runtime.ui,
                conflict.document_id.canonical_path.display().to_string(),
                matches!(conflict.current_disk, CurrentDiskState::Missing),
            ));
        }
        if let Some(notification_layer) = ComponentRoot::render_notification_layer(window, cx) {
            root = root.child(notification_layer);
        }
        if let Some(sheet_layer) = ComponentRoot::render_sheet_layer(window, cx) {
            root = root.child(sheet_layer);
        }
        if let Some(dialog_layer) = ComponentRoot::render_dialog_layer(window, cx) {
            root = root.child(dialog_layer);
        }

        if self.should_auto_focus_workspace() && !focus_handle.contains_focused(window, cx) {
            focus_handle.focus(window);
        }

        root.track_focus(&focus_handle)
            .key_context(WORKSPACE_CONTEXT)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_mouse_move(cx.listener(Self::on_resize_mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::on_resize_mouse_up))
            .on_action(cx.listener(Self::on_open_project))
            .on_action(cx.listener(Self::on_open_command_palette))
            .on_action(cx.listener(Self::on_open_project_palette))
            .on_action(cx.listener(Self::on_open_tab_palette))
            .on_action(cx.listener(Self::on_open_pane_palette))
            .on_action(cx.listener(Self::on_palette_select_next))
            .on_action(cx.listener(Self::on_palette_select_prev))
            .on_action(cx.listener(Self::on_palette_confirm))
            .on_action(cx.listener(Self::on_palette_cancel))
            .on_action(cx.listener(Self::on_project_close))
            .on_action(cx.listener(Self::on_tab_new))
            .on_action(cx.listener(Self::on_tab_close))
            .on_action(cx.listener(Self::on_tab_rename))
            .on_action(cx.listener(Self::on_tab_next))
            .on_action(cx.listener(Self::on_tab_prev))
            .on_action(cx.listener(Self::on_file_save))
            .on_action(cx.listener(Self::on_pane_split_vertical))
            .on_action(cx.listener(Self::on_pane_split_horizontal))
            .on_action(cx.listener(Self::on_pane_close))
            .on_action(cx.listener(Self::on_pane_rename))
            .on_action(cx.listener(Self::on_pane_focus_left))
            .on_action(cx.listener(Self::on_pane_focus_right))
            .on_action(cx.listener(Self::on_pane_focus_up))
            .on_action(cx.listener(Self::on_pane_focus_down))
            .on_action(cx.listener(Self::on_pane_resize_left))
            .on_action(cx.listener(Self::on_pane_resize_right))
            .on_action(cx.listener(Self::on_pane_resize_up))
            .on_action(cx.listener(Self::on_pane_resize_down))
            .on_action(cx.listener(Self::on_layout_default_edit))
            .on_action(cx.listener(Self::on_layout_default_reset))
            .on_action(cx.listener(Self::on_layout_default_reload))
            .on_action(cx.listener(Self::on_layout_project_edit))
            .on_action(cx.listener(Self::on_layout_save_current))
            .on_action(cx.listener(Self::on_layout_export_project_config))
            .on_action(cx.listener(Self::on_layout_reset_local_override))
            .on_action(cx.listener(Self::on_layout_open_file))
            .on_action(cx.listener(Self::on_settings_open))
            .on_action(cx.listener(Self::on_settings_keybindings))
            .on_action(cx.listener(Self::on_settings_notifications))
    }
}

fn split_child(child: Div, basis: f32) -> Div {
    div()
        .flex()
        .flex_col()
        .flex_basis(relative(basis))
        .flex_shrink()
        .overflow_hidden()
        .child(child)
}

fn layout_toml_editor_overlay(
    root: &RootView,
    input: &Entity<InputState>,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let editor_theme = root.theme_runtime.editor;
    let Some(session) = root.layout_toml_editor.as_ref() else {
        return div();
    };
    let editor = session.editor();
    let title = editor.config().title().to_string();
    let path = editor.path().display().to_string();
    let error = editor.error().map(ToOwned::to_owned);

    capture_overlay_input(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000099))
            .child(
                div()
                    .flex()
                    .flex_col()
                    .w(relative(0.72))
                    .max_w(px(1040.))
                    .h(px(680.))
                    .max_h(relative(0.86))
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(theme.surface)
                    .text_color(theme.text)
                    .overflow_hidden()
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .border_b_1()
                            .border_color(theme.border)
                            .px_5()
                            .py_4()
                            .child(
                                div()
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .min_w_0()
                                    .child(
                                        div()
                                            .text_lg()
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .child(title),
                                    )
                                    .child(
                                        div()
                                            .text_xs()
                                            .text_color(theme.text_subtle)
                                            .truncate()
                                            .child(path),
                                    ),
                            )
                            .child(settings_button(
                                "layout-toml-editor-close",
                                "Cancel",
                                false,
                                theme,
                                cx,
                                cx.listener(|this, _, _window, cx| {
                                    this.cancel_layout_toml_editor();
                                    cx.notify();
                                }),
                            )),
                    )
                    .child(
                        div()
                            .flex()
                            .flex_col()
                            .flex_1()
                            .min_h_0()
                            .p_4()
                            .gap_3()
                            .child(
                                div()
                                    .flex_1()
                                    .min_h_0()
                                    .rounded_sm()
                                    .border_1()
                                    .border_color(theme.border)
                                    .bg(editor_theme.background)
                                    .overflow_hidden()
                                    .child(
                                        Input::new(input)
                                            .h_full()
                                            .appearance(false)
                                            .bordered(false)
                                            .focus_bordered(false),
                                    ),
                            )
                            .when_some(error, |this, error| {
                                this.child(
                                    div()
                                        .rounded_sm()
                                        .border_1()
                                        .border_color(theme.danger)
                                        .bg(theme.surface_elevated)
                                        .px_3()
                                        .py_2()
                                        .text_xs()
                                        .text_color(theme.danger)
                                        .child(error),
                                )
                            }),
                    )
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_end()
                            .gap_2()
                            .border_t_1()
                            .border_color(theme.border)
                            .px_5()
                            .py_3()
                            .child(settings_button(
                                "layout-toml-editor-cancel",
                                "Cancel",
                                false,
                                theme,
                                cx,
                                cx.listener(|this, _, _window, cx| {
                                    this.cancel_layout_toml_editor();
                                    cx.notify();
                                }),
                            ))
                            .child(settings_button(
                                "layout-toml-editor-save",
                                "Save",
                                true,
                                theme,
                                cx,
                                cx.listener(|this, _, _window, cx| {
                                    let _ = this.save_layout_toml_editor();
                                    cx.notify();
                                }),
                            )),
                    ),
            ),
    )
}

fn settings_overlay(
    root: &mut RootView,
    search_input: &Entity<InputState>,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let style = settings_panel_style();
    let panel = yttt_panel_style(YtttPanelKind::Settings, theme);

    capture_overlay_input(
        div()
            .absolute()
            .inset_0()
            .flex()
            .items_center()
            .justify_center()
            .bg(rgba(0x00000066))
            .child(
                div()
                    .flex()
                    .w(panel.width)
                    .h(panel.height.unwrap_or(style.height))
                    .max_w(panel.max_width)
                    .max_h(panel.max_height)
                    .rounded(panel.radius)
                    .border_1()
                    .border_color(panel.border)
                    .bg(panel.background)
                    .text_color(theme.text)
                    .overflow_hidden()
                    .child(settings_sidebar(root, search_input, style, cx))
                    .child(settings_content(root, style, window, cx)),
            ),
    )
}

fn settings_sidebar(
    root: &RootView,
    search_input: &Entity<InputState>,
    style: SettingsPanelStyle,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let groups = root
        .settings_page
        .visible_groups(&root.ui_text)
        .into_iter()
        .fold(
            div().flex().flex_col().gap_1().min_h_0(),
            |groups, group| {
                let group_id = group.id.as_str().to_string();
                let background = if group.selected {
                    theme.active_surface
                } else {
                    rgba(0x00000000)
                };
                let text = if group.selected {
                    theme.text
                } else {
                    theme.text_muted
                };

                groups.child(
                    div()
                        .id(SharedString::from(format!(
                            "settings-group-{}",
                            group.id.as_str()
                        )))
                        .flex()
                        .items_center()
                        .h_8()
                        .rounded_sm()
                        .px_3()
                        .bg(background)
                        .text_sm()
                        .text_color(text)
                        .hover(move |this| this.bg(theme.hover_surface))
                        .on_click(cx.listener(move |this, _, _window, cx| {
                            let _ = this.select_settings_group(&group_id);
                            cx.notify();
                        }))
                        .child(group.title),
                )
            },
        );

    div()
        .flex()
        .flex_col()
        .w(style.sidebar_width)
        .h_full()
        .min_h_0()
        .flex_none()
        .border_r_1()
        .border_color(theme.border)
        .bg(theme.app_background)
        .p_3()
        .gap_3()
        .child(
            div()
                .id(SharedString::from("settings-search"))
                .flex()
                .items_center()
                .h(style.search_height)
                .flex_none()
                .rounded_md()
                .border_1()
                .border_color(theme.border)
                .bg(theme.surface)
                .px_2()
                .child(
                    Input::new(search_input)
                        .prefix(IconName::Search)
                        .cleanable(true)
                        .appearance(false),
                ),
        )
        .child(
            div()
                .flex_1()
                .min_h_0()
                .child(groups.overflow_y_scrollbar()),
        )
}

fn settings_content(
    root: &mut RootView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let group = root.settings_page.selected_group;

    div()
        .flex()
        .flex_col()
        .flex_1()
        .min_w_0()
        .min_h_0()
        .bg(theme.surface)
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .justify_between()
                .border_b_1()
                .border_color(theme.border)
                .px_6()
                .py_4()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .child(
                            div()
                                .text_lg()
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(group.title(&root.ui_text)),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(group.description(&root.ui_text)),
                        ),
                )
                .child(settings_button(
                    "settings-close",
                    root.ui_text.get(UiTextKey::SettingsClose),
                    false,
                    theme,
                    cx,
                    cx.listener(|this, _, _window, cx| {
                        this.close_settings();
                        cx.notify();
                    }),
                )),
        )
        .child(
            div()
                .flex()
                .flex_col()
                .flex_1()
                .min_h_0()
                .overflow_y_scrollbar()
                .px_6()
                .child(settings_rows(root, group, style, window, cx)),
        )
}

fn settings_rows(
    root: &mut RootView,
    group: SettingsGroupId,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Div {
    match group {
        SettingsGroupId::General => settings_general_rows(root, style, window, cx),
        SettingsGroupId::Appearance => settings_appearance_rows(root, style, window, cx),
        SettingsGroupId::Languages => settings_language_rows(root, style, window, cx),
        SettingsGroupId::Terminal => settings_terminal_rows(root, style, window, cx),
        SettingsGroupId::DefaultLayout => settings_default_layout_rows(root, style, cx),
        SettingsGroupId::Keybindings => settings_keybinding_rows(root, style, cx),
    }
}

fn settings_general_rows(
    root: &mut RootView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let language_select = root.settings_language_select(window, cx);
    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguage),
            text.get(UiTextKey::SettingsLanguageDescription),
            settings_select_control(
                language_select,
                theme,
                false,
                text.get(UiTextKey::SettingsSelectLanguage),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsSystemNotifications),
            text.get(UiTextKey::SettingsSystemNotificationsDescription),
            settings_switch(
                "settings-notifications",
                root.system_notifications_enabled,
                theme,
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_system_notifications_enabled(*checked);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
}

fn settings_appearance_rows(
    root: &mut RootView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let ui_theme_select = root.settings_ui_theme_select(window, cx);
    let terminal_theme_select = root.settings_terminal_theme_select(window, cx);

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsUiTheme),
            text.get(UiTextKey::SettingsUiThemeDescription),
            settings_select_control(
                ui_theme_select,
                theme,
                true,
                text.get(UiTextKey::SettingsSearchTheme),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsTerminalTheme),
            text.get(UiTextKey::SettingsTerminalThemeDescription),
            settings_select_control(
                terminal_theme_select,
                theme,
                true,
                text.get(UiTextKey::SettingsSearchTheme),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsEditSettingsToml),
            text.get(UiTextKey::SettingsEditSettingsTomlDescription),
            settings_button(
                "settings-open-file",
                text.get(UiTextKey::SettingsShowPath),
                false,
                theme,
                cx,
                cx.listener(move |this, _, window, cx| {
                    this.show_settings_file_path_status();
                    this.flush_pending_status_notifications(window, cx);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsThemesDirectory),
            text.get(UiTextKey::SettingsThemesDirectoryDescription),
            settings_button(
                "settings-open-themes-dir",
                text.get(UiTextKey::SettingsShowPath),
                false,
                theme,
                cx,
                cx.listener(move |this, _, window, cx| {
                    this.show_themes_directory_status();
                    this.flush_pending_status_notifications(window, cx);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
}

fn settings_language_rows(
    root: &mut RootView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let default_language_select = root.settings_editor_language_select(window, cx);
    let supported_language_count = root.available_editor_language_names().len();
    let lsp_command = if root.editor_lsp_command().is_empty() {
        text.get(UiTextKey::SettingsUnbound).to_string()
    } else {
        root.editor_lsp_command().to_string()
    };

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguageDetection),
            text.get(UiTextKey::SettingsLanguageDetectionDescription),
            settings_switch(
                "settings-editor-auto-detect-language",
                root.editor_auto_detect_language(),
                theme,
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_editor_auto_detect_language(*checked);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsDefaultCodeLanguage),
            text.get(UiTextKey::SettingsDefaultCodeLanguageDescription),
            settings_select_control(
                default_language_select,
                theme,
                true,
                text.get(UiTextKey::SettingsSearchCodeLanguage),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsSupportedLanguages),
            text.get(UiTextKey::SettingsSupportedLanguagesDescription),
            settings_value(supported_language_count.to_string(), theme).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguageServer),
            text.get(UiTextKey::SettingsLanguageServerDescription),
            settings_switch(
                "settings-editor-lsp-enabled",
                root.editor_lsp_enabled(),
                theme,
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_editor_lsp_enabled(*checked);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLanguageServerCommand),
            text.get(UiTextKey::SettingsLanguageServerCommandDescription),
            settings_value(lsp_command, theme).into_any_element(),
        ))
}

fn settings_terminal_rows(
    root: &mut RootView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let shell_select = root.settings_shell_select(window, cx);
    let font_select = root.settings_font_family_select(window, cx);
    let font_size_input = root.settings_number_input(SettingsNumberField::FontSize, window, cx);
    let line_height_input = root.settings_number_input(SettingsNumberField::LineHeight, window, cx);
    let padding_input = root.settings_number_input(SettingsNumberField::Padding, window, cx);
    let scrollback_input = root.settings_number_input(SettingsNumberField::Scrollback, window, cx);

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsDefaultShell),
            text.get(UiTextKey::SettingsDefaultShellDescription),
            settings_select_control(
                shell_select,
                theme,
                false,
                text.get(UiTextKey::SettingsSelectShell),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsFontFamily),
            text.get(UiTextKey::SettingsFontFamilyDescription),
            settings_select_control(
                font_select,
                theme,
                true,
                text.get(UiTextKey::SettingsSearchFont),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsFontSize),
            text.get(UiTextKey::SettingsFontSizeDescription),
            settings_number_control(font_size_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsLineHeight),
            text.get(UiTextKey::SettingsLineHeightDescription),
            settings_number_control(line_height_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsPadding),
            text.get(UiTextKey::SettingsPaddingDescription),
            settings_number_control(padding_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsScrollback),
            text.get(UiTextKey::SettingsScrollbackDescription),
            settings_number_control(scrollback_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsScrollbar),
            text.get(UiTextKey::SettingsScrollbarDescription),
            settings_switch(
                "settings-show-scrollbar",
                root.terminal_show_scrollbar(),
                theme,
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_terminal_show_scrollbar(*checked);
                    this.sync_terminal_pane_configs(cx);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsClosePaneOnExit),
            text.get(UiTextKey::SettingsClosePaneOnExitDescription),
            settings_switch(
                "settings-close-on-exit",
                root.terminal_close_on_exit(),
                theme,
                cx.listener(|this, checked: &bool, _window, cx| {
                    let _ = this.set_terminal_close_on_exit(*checked);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
}

fn settings_default_layout_rows(
    root: &RootView,
    style: SettingsPanelStyle,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let path = root.default_layout_state.path().display().to_string();

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsDefaultLayoutPath),
            text.get(UiTextKey::SettingsDefaultLayoutPathDescription),
            settings_value(path, theme).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsEditDefaultLayout),
            text.get(UiTextKey::SettingsEditDefaultLayoutDescription),
            settings_command_button(
                "settings-default-layout-edit",
                text.get(UiTextKey::SettingsEdit),
                true,
                theme,
                CommandId::LayoutDefaultEdit,
                cx,
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsReloadDefaultLayout),
            text.get(UiTextKey::SettingsReloadDefaultLayoutDescription),
            settings_command_button(
                "settings-default-layout-reload",
                text.get(UiTextKey::SettingsOpen),
                true,
                theme,
                CommandId::LayoutDefaultReload,
                cx,
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsResetDefaultLayout),
            text.get(UiTextKey::SettingsResetDefaultLayoutDescription),
            settings_command_button(
                "settings-default-layout-reset",
                text.get(UiTextKey::SettingsReset),
                true,
                theme,
                CommandId::LayoutDefaultReset,
                cx,
            )
            .into_any_element(),
        ))
}

fn settings_keybinding_rows(
    root: &mut RootView,
    style: SettingsPanelStyle,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let text = root.ui_text;
    let diagnostics = if root.keybinding_warning_lines.is_empty() {
        text.get(UiTextKey::SettingsNoKeybindingConflicts)
            .to_string()
    } else {
        root.keybinding_warning_lines.join("; ")
    };

    let mut rows = div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsEditKeybindingsToml),
            text.get(UiTextKey::SettingsEditKeybindingsTomlDescription),
            settings_command_button(
                "settings-keybindings-open",
                text.get(UiTextKey::SettingsOpen),
                true,
                theme,
                CommandId::SettingsKeybindings,
                cx,
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            text.get(UiTextKey::SettingsKeybindingDiagnostics),
            text.get(UiTextKey::SettingsKeybindingDiagnosticsDescription),
            settings_value(diagnostics, theme).into_any_element(),
        ));

    for row in root.visible_keybinding_rows() {
        let command = row.command;
        let keys = row.display_keys();
        let title = row.title;
        let description = row.command_id;
        let title_text = if row.has_conflict {
            format!("{title} ({})", text.get(UiTextKey::SettingsConflict))
        } else {
            title.to_string()
        };

        rows = rows.child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap_4()
                .min_h(style.row_min_height)
                .border_b_1()
                .border_color(theme.border)
                .py_3()
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap_1()
                        .min_w_0()
                        .flex_1()
                        .child(
                            div()
                                .text_sm()
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(theme.text)
                                .child(title_text),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(description),
                        ),
                )
                .child(
                    div()
                        .flex()
                        .items_center()
                        .justify_end()
                        .gap_1()
                        .flex_none()
                        .child(settings_keybinding_value(
                            keys,
                            text.get(UiTextKey::SettingsUnbound),
                            theme,
                        ))
                        .child(settings_button(
                            format!("settings-keybinding-edit-{}", row.command_id),
                            text.get(UiTextKey::SettingsEdit),
                            false,
                            theme,
                            cx,
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.open_keybinding_edit_dialog(command);
                                cx.notify();
                            }),
                        ))
                        .child(settings_button(
                            format!("settings-keybinding-reset-{}", row.command_id),
                            text.get(UiTextKey::SettingsReset),
                            false,
                            theme,
                            cx,
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.reset_keybinding_command_keys(command);
                                cx.notify();
                            }),
                        ))
                        .child(settings_button(
                            format!("settings-keybinding-delete-{}", row.command_id),
                            text.get(UiTextKey::SettingsDelete),
                            false,
                            theme,
                            cx,
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.delete_keybinding_command_keys(command);
                                cx.notify();
                            }),
                        )),
                ),
        );
    }

    rows
}

fn setting_row(
    style: SettingsPanelStyle,
    theme: WorkbenchTheme,
    title: impl Into<String>,
    description: impl Into<String>,
    control: AnyElement,
) -> Div {
    workbench_settings_row(style.control_width, theme, title, description, control)
}

fn settings_select_control(
    select: Entity<SettingsStringSelectState>,
    theme: WorkbenchTheme,
    searchable: bool,
    search_placeholder: &'static str,
) -> Select<SearchableVec<String>> {
    let select_style = yttt_select_style(theme);
    Select::new(&select)
        .small()
        .menu_width(select_style.menu_width)
        .search_placeholder(search_placeholder)
        .appearance(true)
        .w(select_style.width)
        .h(select_style.height)
        .rounded(select_style.radius)
        .bg(select_style.background)
        .border_color(select_style.border)
        .text_color(select_style.text)
        .when(searchable, |select| select.cleanable(false))
}

fn settings_number_control(input: Entity<InputState>, style: SettingsPanelStyle) -> Div {
    div()
        .w(style.compact_control_width)
        .h(style.control_height)
        .child(
            NumberInput::new(&input)
                .small()
                .appearance(true)
                .w(style.compact_control_width)
                .h(style.control_height),
        )
}

fn settings_command_button(
    id: impl Into<String>,
    label: impl Into<String>,
    enabled: bool,
    theme: WorkbenchTheme,
    command: CommandId,
    cx: &mut Context<RootView>,
) -> Button {
    settings_button(
        id,
        label,
        false,
        theme,
        cx,
        cx.listener(move |this, _, window, cx| {
            if enabled {
                let _ = this.run_command(command);
                this.flush_pending_status_notifications(window, cx);
            }
            cx.notify();
        }),
    )
    .disabled(!enabled)
    .tab_stop(enabled)
}

fn settings_switch<H>(
    id: impl Into<String>,
    checked: bool,
    theme: WorkbenchTheme,
    on_change: H,
) -> Div
where
    H: Fn(&bool, &mut Window, &mut gpui::App) + 'static,
{
    workbench_switch(SharedString::from(id.into()), checked, theme, on_change)
}

fn settings_button<H>(
    id: impl Into<String>,
    label: impl Into<String>,
    selected: bool,
    theme: WorkbenchTheme,
    cx: &mut Context<RootView>,
    on_click: H,
) -> Button
where
    H: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    let variant = if selected {
        YtttButtonVariant::Primary
    } else {
        YtttButtonVariant::Secondary
    };
    yttt_button(
        SharedString::from(id.into()),
        SharedString::from(label.into()),
        variant,
        theme,
        cx,
    )
    .on_click(on_click)
}

fn settings_value(value: impl Into<String>, theme: WorkbenchTheme) -> Div {
    div()
        .max_w_64()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(theme.surface_elevated)
        .px_3()
        .py_1()
        .text_xs()
        .text_color(theme.text_muted)
        .child(value.into())
}

fn settings_keybinding_value(
    keybindings: Vec<String>,
    unbound_label: impl Into<String>,
    theme: WorkbenchTheme,
) -> Div {
    if keybindings.is_empty() {
        return div().child(settings_value(unbound_label, theme));
    }

    let mut value = div().flex().items_center().justify_end().gap_1().max_w_96();
    for keybinding in keybindings {
        value = value.child(workbench_keybinding_badge(keybinding, theme));
    }
    value
}

fn error_notification_overlay(item: ToastItem, theme: WorkbenchTheme) -> Div {
    div()
        .absolute()
        .top(px(48.0))
        .right(px(12.0))
        .child(workbench_inline_notification(item, theme))
}

fn push_component_notification(
    root: Entity<RootView>,
    event: NotificationEvent,
    window: &mut Window,
    cx: &mut Context<RootView>,
) {
    let item = toast_item_for_event(&event);
    let root_state = root.read(cx);
    let theme = root_state.theme_runtime.ui;
    let action_label = root_state
        .ui_text
        .get(UiTextKey::OpenNotificationTarget)
        .to_string();
    let focus_event = event.clone();
    window.push_notification(
        workbench_agent_notification(item, action_label, theme).on_click(move |_, _window, cx| {
            root.update(cx, |root, cx| {
                let _ = root.focus_notification_target(&focus_event);
                cx.notify();
            });
        }),
        cx,
    );
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
