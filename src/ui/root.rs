use gpui::{
    AnyElement, ClickEvent, Context, Div, Entity, FocusHandle, Focusable as _, FontWeight,
    InteractiveElement as _, IntoElement, KeyDownEvent, Keystroke, MouseButton, MouseDownEvent,
    MouseMoveEvent, MouseUpEvent, ParentElement as _, PathPromptOptions, Pixels, Point, Render,
    ScrollHandle, SharedString, Stateful, Subscription, Window, div, prelude::*, px, relative,
    rgba,
};
use gpui_component::{
    IconName, IndexPath, Root as ComponentRoot, Sizable as _, Theme as ComponentTheme,
    WindowExt as _,
    alert::Alert,
    button::{Button, ButtonCustomVariant, ButtonVariants as _},
    input::{Input, InputEvent, InputState, NumberInput, NumberInputEvent, StepAction},
    notification::{Notification, NotificationType},
    scroll::ScrollableElement as _,
    select::{SearchableVec, Select, SelectEvent, SelectState},
};
use yttt_terminal::input::keystroke_to_bytes;

mod dialogs;
mod helpers;
use dialogs::*;
use helpers::*;

use std::{
    collections::{HashMap, HashSet},
    fs,
    path::{Path, PathBuf},
    rc::Rc,
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
        CommandDispatchError, CommandId, CommandRegistry, default_registry,
        dispatch_workspace_command,
    },
    config::{
        keybindings::{
            KeybindingLoadWarning, KeybindingsLoadError, ensure_keybindings_file, load_keybindings,
        },
        layout_loader::{
            LayoutSource, ProjectOpenError, RecentProjectsConfig, export_project_layout,
            load_recent_projects, open_project_config, save_local_layout,
        },
        paths::AppConfigPaths,
        settings::{
            AppSettings, LanguageSetting, SettingsLoadWarning, SettingsSaveError,
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
        command_palette_items, pane_palette_items, project_palette_items, tab_palette_items,
    },
    runtime::{
        git_status::{ProjectGitStatus, read_project_git_status},
        notification::{
            NoopSystemNotifier, NotificationEvent, NotificationKind, maybe_notify_system,
        },
    },
    ui::{
        actions::{
            LayoutExportProjectConfig, LayoutOpenFile, LayoutSaveCurrent, OpenCommandPalette,
            OpenPanePalette, OpenProject, OpenProjectPalette, OpenTabPalette, PaletteCancel,
            PaletteConfirm, PaletteSelectNext, PaletteSelectPrev, PaneClose, PaneFocusDown,
            PaneFocusLeft, PaneFocusRight, PaneFocusUp, PaneRename, PaneResizeDown, PaneResizeLeft,
            PaneResizeRight, PaneResizeUp, PaneSplitHorizontal, PaneSplitVertical, ProjectClose,
            SettingsKeybindings, SettingsNotifications, SettingsOpen, TabClose, TabNew, TabNext,
            TabPrev, TabRename, UiKeybindingSpec, WORKSPACE_CONTEXT, runtime_command_for_keystroke,
            ui_keybinding_specs_from_config,
        },
        components::{ActionEmphasis, workbench_action_button},
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
        keybindings_editor::{KeybindingEditError, KeybindingRow, KeybindingsEditorState},
        overlay::capture_overlay_input,
        palette::palette_overlay,
        palette_surface::palette_input_placeholder,
        primitives::{
            button::{YtttButtonVariant, yttt_button_style},
            dialog::yttt_dialog_style,
            input::{YtttInputKind, yttt_input_style},
        },
        settings::{SettingsGroupId, SettingsPageState, SettingsPanelStyle, settings_panel_style},
        sidebar::project_sidebar,
        split_view::{pointer_resize_for_drag_delta, split_child_basis},
        tabs::project_tabs,
        terminal_pane::{
            TerminalPaneContext, TerminalPaneEvent, TerminalPaneExitedEvent, TerminalPaneView,
        },
        theme::{ThemeRuntime, WorkbenchTheme},
        titlebar::{TitlebarInfo, compact_path_for_titlebar, workbench_titlebar},
        toast::{ToastQueue, ToastTone, toast_item_for_event},
    },
};

pub use crate::ui::overlay::overlay_input_capture_policy;

pub struct RootView {
    workspace: Workspace,
    config_paths: AppConfigPaths,
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
    settings_font_family_select: Option<Entity<SettingsStringSelectState>>,
    settings_font_family_select_subscription: Option<Subscription>,
    settings_number_inputs: HashMap<SettingsNumberField, Entity<InputState>>,
    settings_number_input_subscriptions: HashMap<SettingsNumberField, Vec<Subscription>>,
    layout_toml_editor: Option<LayoutTomlEditorState>,
    layout_toml_input: Option<Entity<InputState>>,
    layout_toml_input_subscription: Option<Subscription>,
    layout_toml_input_needs_focus: bool,
    palette_scroll_handle: ScrollHandle,
    sidebar_collapsed: bool,
    active_split_resize_drag: Option<ActiveSplitResizeDrag>,
    pending_terminal_focus_pane_id: Option<String>,
    terminal_panes: HashMap<String, Entity<TerminalPaneView>>,
    terminal_pane_subscriptions: HashMap<String, Subscription>,
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

#[derive(Clone, Debug, PartialEq, Eq)]
struct LayoutTomlEditorState {
    path: PathBuf,
    value: String,
    error: Option<String>,
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
        let system_notifications_enabled = app_settings.notifications.system;

        Self {
            workspace,
            config_paths,
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
            active_split_resize_drag: None,
            pending_terminal_focus_pane_id: None,
            terminal_panes: HashMap::new(),
            terminal_pane_subscriptions: HashMap::new(),
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

    pub fn sidebar_is_collapsed(&self) -> bool {
        self.sidebar_collapsed
    }

    pub fn toggle_sidebar(&mut self) {
        self.sidebar_collapsed = !self.sidebar_collapsed;
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
                .unwrap_or("Command is unavailable");
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
                    self.workspace.select_project(&project_id)?;
                    self.refresh_selected_project_git_status();
                    self.queue_selected_terminal_focus();
                } else if item.command == CommandId::ProjectOpenRecent {
                    self.open_project_path(PathBuf::from(&item.id))?;
                }
            }
            PaletteKind::Tab => {
                self.workspace.select_tab(&item.id)?;
                self.queue_selected_terminal_focus();
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

        self.workspace.select_project(&project_id)?;
        if let Err(error) = self.workspace.select_tab(&event.tab_id) {
            return self.fail_workspace_error(error);
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
        self.workspace.select_tab(tab_id)?;
        if click_count >= 2 {
            self.run_command(CommandId::TabRename)?;
        } else {
            self.queue_selected_terminal_focus();
        }
        self.load_error = None;
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

    pub fn visible_error_message(&self) -> Option<&str> {
        self.load_error.as_deref()
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

    pub fn set_settings_search_query(&mut self, query: impl Into<String>) {
        self.settings_page.search_query = query.into();
        let selected_group_visible = self
            .settings_page
            .visible_groups()
            .iter()
            .any(|group| group.id == self.settings_page.selected_group);
        if !selected_group_visible
            && let Some(first_group) = self.settings_page.visible_groups().first()
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
            .visible_groups()
            .into_iter()
            .map(|group| group.title)
            .collect()
    }

    pub fn selected_settings_group_title(&self) -> Option<&'static str> {
        Some(self.settings_page.selected_group.title())
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
            .map(|editor| editor.path.as_path())
    }

    pub fn layout_toml_editor_value(&self) -> Option<&str> {
        self.layout_toml_editor
            .as_ref()
            .map(|editor| editor.value.as_str())
    }

    pub fn visible_layout_toml_editor_error(&self) -> Option<&str> {
        self.layout_toml_editor
            .as_ref()
            .and_then(|editor| editor.error.as_deref())
    }

    pub fn open_layout_toml_editor(&mut self) -> Result<(), RootViewError> {
        let path = self.ensure_layout_toml_edit_file()?;
        let value = fs::read_to_string(&path).map_err(|source| {
            RootViewError::LayoutTomlEditor(format!(
                "failed to read layout TOML at {}: {source}",
                path.display()
            ))
        })?;

        self.layout_toml_editor = Some(LayoutTomlEditorState {
            path,
            value,
            error: None,
        });
        self.reset_layout_toml_input();
        self.layout_toml_input_needs_focus = true;
        self.load_error = None;
        self.sync_input_owner_state();
        Ok(())
    }

    pub fn set_layout_toml_editor_value(&mut self, value: impl Into<String>) {
        if let Some(editor) = &mut self.layout_toml_editor {
            editor.value = value.into();
            editor.error = None;
            self.reset_layout_toml_input();
        }
    }

    pub fn save_layout_toml_editor(&mut self) -> Result<(), RootViewError> {
        let Some(editor) = self.layout_toml_editor.clone() else {
            return Ok(());
        };

        let layout = match toml::from_str::<ProjectLayout>(&editor.value) {
            Ok(layout) => layout,
            Err(error) => {
                self.set_layout_toml_editor_error(format!("failed to parse layout TOML: {error}"));
                return Ok(());
            }
        };
        if let Err(error) = layout.validate() {
            self.set_layout_toml_editor_error(format!("invalid layout TOML: {error}"));
            return Ok(());
        }

        if let Some(parent) = editor.path.parent() {
            fs::create_dir_all(parent).map_err(|source| {
                RootViewError::LayoutTomlEditor(format!(
                    "failed to create layout directory {}: {source}",
                    parent.display()
                ))
            })?;
        }
        fs::write(&editor.path, editor.value).map_err(|source| {
            RootViewError::LayoutTomlEditor(format!(
                "failed to write layout TOML at {}: {source}",
                editor.path.display()
            ))
        })?;

        let selected_project_id = self.workspace.selected_project_id().cloned();
        self.workspace.replace_selected_project_layout(layout)?;
        if let Some(project_id) = selected_project_id {
            self.remove_terminal_panes_for_project(project_id.as_str());
        }
        self.layout_toml_editor = None;
        self.reset_layout_toml_input();
        self.queue_selected_terminal_focus();
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
            "System notifications: enabled"
        } else {
            "System notifications: disabled"
        }
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
        self.queue_selected_terminal_focus();
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

    pub fn workspace_arrow_keydown_command(
        key: &str,
        platform: bool,
        control: bool,
        alt: bool,
        shift: bool,
    ) -> Option<CommandId> {
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
        match command_id {
            CommandId::CommandPaletteOpen => {
                self.open_palette(PaletteKind::Command);
                Ok(())
            }
            CommandId::ProjectPalette => {
                self.open_palette(PaletteKind::Project);
                Ok(())
            }
            CommandId::TabPalette => {
                self.open_palette(PaletteKind::Tab);
                Ok(())
            }
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
                self.load_error = Some(format!("Keybindings file: {}", path.display()));
                Ok(())
            }
            CommandId::SettingsNotifications => {
                self.set_system_notifications_enabled(!self.system_notifications_enabled)?;
                self.load_error = Some(self.visible_notification_settings_message().to_string());
                Ok(())
            }
            CommandId::TabNew => {
                let shell = self.resolved_terminal_shell();
                let _tab_id = self.workspace.create_shell_tab_with_command(shell)?;
                self.queue_selected_terminal_focus();
                Ok(())
            }
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
            CommandId::LayoutOpenFile => {
                let (project_path, _layout) = self.selected_project_layout_snapshot()?;
                let project_layout_file = self.config_paths.project_layout_file(&project_path);
                let local_layout_file = self.config_paths.local_layout_file(&project_path);
                if project_layout_file.exists() {
                    self.last_opened_layout_file = Some(project_layout_file);
                    self.load_error = None;
                } else if local_layout_file.exists() {
                    self.last_opened_layout_file = Some(local_layout_file);
                    self.load_error = None;
                } else {
                    self.load_error = Some(format!(
                        "Layout file does not exist: {}",
                        project_layout_file.display()
                    ));
                }
                Ok(())
            }
            _ => {
                dispatch_workspace_command(&mut self.workspace, command_id)?;
                if should_focus_terminal_after_command(command_id) {
                    self.queue_selected_terminal_focus();
                }
                Ok(())
            }
        }
    }

    pub fn has_pending_project_close(&self) -> bool {
        self.pending_close_project_id.is_some()
    }

    pub fn visible_close_project_dialog_text(&self) -> Option<String> {
        self.pending_close_project_id.as_ref().map(|_| {
            format!(
                "{}\n{}",
                self.ui_text.get(UiTextKey::CloseProjectTitle),
                self.ui_text.get(UiTextKey::CloseProjectBody)
            )
        })
    }

    pub fn visible_close_project_dialog_actions(&self) -> Vec<String> {
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

    pub fn confirm_pending_project_close(&mut self) -> Result<(), RootViewError> {
        let project_id = self
            .pending_close_project_id
            .clone()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let closed = self.workspace.confirm_close_project(&project_id)?;
        self.pending_close_project_id = None;
        self.layout_source_messages.remove(&closed.project_id);
        self.project_git_statuses.remove(&closed.project_id);
        self.remove_terminal_panes_for_project(closed.project_id.as_str());
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
        match open_project_config(&self.config_paths, project_path.as_ref()) {
            Ok(opened) => {
                let source_message = layout_source_message(&opened.layout_source);
                let opened_path = opened.path.clone();
                let project_id = self.workspace.open_project(opened.path, opened.layout)?;
                self.refresh_project_git_status(&project_id, &opened_path);
                self.queue_selected_terminal_focus();
                self.layout_source_messages
                    .insert(project_id, source_message);
                self.recent_projects = recent_projects_for_palette(opened.recent_projects);
                self.load_error = None;
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
            PaletteKind::Command => command_palette_items(
                &self.command_registry,
                CommandPaletteContext::from_workspace(&self.workspace),
            ),
            PaletteKind::Project => project_palette_items(&self.workspace, &self.recent_projects),
            PaletteKind::Tab => tab_palette_items(&self.workspace).unwrap_or_default(),
            PaletteKind::Pane => pane_palette_items(&self.workspace).unwrap_or_default(),
        }
    }

    fn request_close_selected_project(&mut self) -> Result<CloseProjectDecision, RootViewError> {
        let project_id = self
            .workspace
            .selected_project_id()
            .cloned()
            .ok_or(WorkspaceError::NoSelectedProject)?;
        let decision = self.workspace.request_close_project(&project_id)?;
        match &decision {
            CloseProjectDecision::Closed(closed) => {
                self.pending_close_project_id = None;
                self.layout_source_messages.remove(&closed.project_id);
                self.project_git_statuses.remove(&closed.project_id);
                self.remove_terminal_panes_for_project(closed.project_id.as_str());
            }
            CloseProjectDecision::NeedsConfirmation { project_id, .. } => {
                self.pending_close_project_id = Some(project_id.clone());
            }
        }
        self.sync_input_owner_state();

        Ok(decision)
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

    fn ensure_layout_toml_edit_file(&self) -> Result<PathBuf, RootViewError> {
        let (project_path, layout) = self.selected_project_layout_snapshot()?;
        let project_layout_file = self.config_paths.project_layout_file(&project_path);
        if project_layout_file.exists() {
            return Ok(project_layout_file);
        }

        let local_layout_file = self.config_paths.local_layout_file(&project_path);
        if local_layout_file.exists() {
            return Ok(local_layout_file);
        }

        save_local_layout(&self.config_paths, &project_path, &layout).map_err(RootViewError::from)
    }

    fn set_layout_toml_editor_error(&mut self, error: String) {
        if let Some(editor) = &mut self.layout_toml_editor {
            editor.error = Some(error);
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
            InputOwnerRegistration::blocking(
                InputOwnerKind::Editor,
                InputScopeId::new("editor.layout_toml"),
            )
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
            let placeholder = palette_input_placeholder(active_palette.kind);
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
                    .placeholder("Search settings...")
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
        let editor = self.layout_toml_editor.as_ref()?;

        let input = if let Some(input) = &self.layout_toml_input {
            input.clone()
        } else {
            let value = editor.value.clone();
            let input = cx.new(|cx| {
                InputState::new(window, cx)
                    .placeholder("Edit layout TOML...")
                    .default_value(value)
                    .code_editor("toml")
                    .rows(24)
                    .soft_wrap(false)
            });
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

    pub fn visible_split_handle_style(_direction: SplitDirection) -> SplitHandleStyle {
        let theme = WorkbenchTheme::dark();
        SplitHandleStyle {
            visible_line_width: theme.split_line_width,
            hit_area_width: theme.split_hit_area_width,
        }
    }

    fn begin_split_resize_drag(&mut self, direction: SplitDirection, position: Point<Pixels>) {
        self.active_split_resize_drag = Some(ActiveSplitResizeDrag {
            direction,
            last_position: position,
        });
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

    fn on_split_resize_mouse_move(
        &mut self,
        event: &MouseMoveEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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

    fn on_split_resize_mouse_up(
        &mut self,
        _event: &MouseUpEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.active_split_resize_drag.take().is_some() {
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn on_palette_input_event(
        &mut self,
        input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
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
                if let Some(editor) = &mut self.layout_toml_editor {
                    editor.value = input.read(cx).value().to_string();
                    editor.error = None;
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
        if let Some(project_path) = std::env::var_os("YTTT_OPEN_PROJECT") {
            let _ = self.open_project_path(PathBuf::from(project_path));
        } else {
            self.prompt_for_project_directory(cx);
        }
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
        _window: &mut Window,
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
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutSaveCurrent, cx);
    }

    fn on_layout_export_project_config(
        &mut self,
        _: &LayoutExportProjectConfig,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutExportProjectConfig, cx);
    }

    fn on_layout_open_file(
        &mut self,
        _: &LayoutOpenFile,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::LayoutOpenFile, cx);
    }

    fn on_settings_keybindings(
        &mut self,
        _: &SettingsKeybindings,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::SettingsKeybindings, cx);
    }

    fn on_settings_open(&mut self, _: &SettingsOpen, _window: &mut Window, cx: &mut Context<Self>) {
        self.dispatch_command_action(CommandId::SettingsOpen, cx);
    }

    fn on_settings_notifications(
        &mut self,
        _: &SettingsNotifications,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::SettingsNotifications, cx);
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
            if let Some(command_id) = Self::workspace_arrow_keydown_command(
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
        self.sync_input_owner_state();
        let focus_handle = self.root_focus_handle(cx);

        let body = if self.workspace.opened_projects().is_empty() {
            empty_workspace(cx, &self.ui_text, &self.theme_runtime.ui)
        } else {
            let split_view = self.active_terminal_split_view(window, cx);

            div()
                .flex()
                .flex_1()
                .relative()
                .bg(self.theme_runtime.ui.app_background)
                .text_color(self.theme_runtime.ui.text)
                .child(project_sidebar(
                    &self.workspace,
                    self.theme_runtime.ui,
                    self.sidebar_collapsed,
                    cx.listener(|this, _, _window, cx| {
                        this.toggle_sidebar();
                        cx.notify();
                    }),
                    |project_id| {
                        let project_id = ProjectId::new(project_id);
                        cx.listener(move |this, _, _window, cx| {
                            let _ = this.workspace.select_project(&project_id);
                            this.refresh_selected_project_git_status();
                            cx.notify();
                        })
                    },
                ))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_1()
                        .child(project_tabs(
                            &self.workspace,
                            self.theme_runtime.ui,
                            |tab_id| {
                                cx.listener(move |this, event: &ClickEvent, _window, cx| {
                                    let _ =
                                        this.handle_project_tab_click(&tab_id, event.click_count());
                                    cx.notify();
                                })
                            },
                            |tab_id| {
                                cx.listener(move |this, _, _window, cx| {
                                    let _ = this.workspace.select_tab(&tab_id);
                                    let _ = this.run_command(CommandId::TabClose);
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
                        ))
                        .child(split_view),
                )
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
                        cx.listener(move |this, _, _window, cx| {
                            if let Some(active_palette) = &mut this.active_palette {
                                active_palette.selected_index = selected_index;
                            }
                            let _ = this.confirm_palette_selection();
                            cx.notify();
                        })
                    },
                ));
            }
        }
        if let Some(load_error) = &self.load_error {
            root = root.child(error_banner(load_error));
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
        if self.pending_close_project_id.is_some() {
            root = root.child(close_project_dialog(
                cx,
                &self.ui_text,
                self.theme_runtime.ui,
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
            .on_mouse_move(cx.listener(Self::on_split_resize_mouse_move))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(Self::on_split_resize_mouse_up),
            )
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
            .on_action(cx.listener(Self::on_layout_save_current))
            .on_action(cx.listener(Self::on_layout_export_project_config))
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
    let Some(editor) = root.layout_toml_editor.as_ref() else {
        return div();
    };
    let path = editor.path.display().to_string();
    let error = editor.error.clone();

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
                                            .child("Edit layout TOML"),
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
                                    .bg(theme.terminal_background)
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
                    .w(style.width)
                    .h(style.height)
                    .max_w(style.max_width)
                    .max_h(style.max_height)
                    .rounded_md()
                    .border_1()
                    .border_color(theme.border_strong)
                    .bg(theme.surface)
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
    let groups = root.settings_page.visible_groups().into_iter().fold(
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
                                .child(group.title()),
                        )
                        .child(
                            div()
                                .text_xs()
                                .text_color(theme.text_subtle)
                                .child(group.description()),
                        ),
                )
                .child(settings_button(
                    "settings-close",
                    "Close",
                    false,
                    theme,
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
        SettingsGroupId::Terminal => settings_terminal_rows(root, style, window, cx),
        SettingsGroupId::ProjectLayout => settings_project_layout_rows(root, style, cx),
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
    let language_select = root.settings_language_select(window, cx);
    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            "Language",
            "Application display language.",
            settings_select_control(language_select, style, false, "Select language")
                .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "System notifications",
            "Notify when agent terminal tasks complete or fail.",
            settings_button(
                "settings-notifications",
                if root.system_notifications_enabled {
                    "On"
                } else {
                    "Off"
                },
                root.system_notifications_enabled,
                theme,
                cx.listener(|this, _, _window, cx| {
                    let _ = this.run_command(CommandId::SettingsNotifications);
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
    let settings_file = root.config_paths.settings_file().display().to_string();
    let themes_dir = root.config_paths.themes_dir().display().to_string();
    let ui_theme_select = root.settings_ui_theme_select(window, cx);
    let terminal_theme_select = root.settings_terminal_theme_select(window, cx);

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            "UI theme",
            "Theme used for YTTT chrome, panels, and controls.",
            settings_select_control(ui_theme_select, style, true, "Search theme...")
                .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Terminal theme",
            "Optional terminal colors override.",
            settings_select_control(terminal_theme_select, style, true, "Search theme...")
                .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Edit settings TOML",
            "Open the app settings file for advanced edits.",
            settings_button(
                "settings-open-file",
                "Show Path",
                false,
                theme,
                cx.listener(move |this, _, _window, cx| {
                    this.load_error = Some(format!("Settings file: {settings_file}"));
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Themes directory",
            "Open the folder containing user theme TOML files.",
            settings_button(
                "settings-open-themes-dir",
                "Show Path",
                false,
                theme,
                cx.listener(move |this, _, _window, cx| {
                    this.load_error = Some(format!("Themes directory: {themes_dir}"));
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
}

fn settings_terminal_rows(
    root: &mut RootView,
    style: SettingsPanelStyle,
    window: &mut Window,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
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
            "Default shell",
            "Shell command used when creating new terminal tabs.",
            settings_select_control(shell_select, style, false, "Select shell").into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Font family",
            "Terminal font family.",
            settings_select_control(font_select, style, true, "Search font...").into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Font size",
            "Terminal font size in pixels.",
            settings_number_control(font_size_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Line height",
            "Terminal line height multiplier.",
            settings_number_control(line_height_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Padding",
            "Terminal pane inner padding.",
            settings_number_control(padding_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Scrollback",
            "Number of terminal lines kept in memory.",
            settings_number_control(scrollback_input, style).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Scrollbar",
            "Show a thin scrollback indicator in terminal panes.",
            settings_button(
                "settings-show-scrollbar",
                if root.terminal_show_scrollbar() {
                    "On"
                } else {
                    "Off"
                },
                root.terminal_show_scrollbar(),
                theme,
                cx.listener(|this, _, _window, cx| {
                    let next = !this.terminal_show_scrollbar();
                    let _ = this.set_terminal_show_scrollbar(next);
                    this.sync_terminal_pane_configs(cx);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Close pane on exit",
            "Automatically close terminal panes when their process exits.",
            settings_button(
                "settings-close-on-exit",
                if root.terminal_close_on_exit() {
                    "On"
                } else {
                    "Off"
                },
                root.terminal_close_on_exit(),
                theme,
                cx.listener(|this, _, _window, cx| {
                    let next = !this.terminal_close_on_exit();
                    let _ = this.set_terminal_close_on_exit(next);
                    cx.notify();
                }),
            )
            .into_any_element(),
        ))
}

fn settings_project_layout_rows(
    root: &RootView,
    style: SettingsPanelStyle,
    cx: &mut Context<RootView>,
) -> Div {
    let theme = root.theme_runtime.ui;
    let has_project = root.workspace.selected_project_id().is_some();
    let layout_source = root
        .visible_layout_source_message()
        .unwrap_or("Open a project first")
        .to_string();

    div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            "Layout source",
            "Current project layout source.",
            settings_value(layout_source, theme).into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Save current layout",
            "Save current layout as an app-local override.",
            settings_command_button(
                "settings-layout-save",
                "Save",
                has_project,
                theme,
                CommandId::LayoutSaveCurrent,
                cx,
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Export project layout",
            "Write current layout into the project config.",
            settings_command_button(
                "settings-layout-export",
                "Export",
                has_project,
                theme,
                CommandId::LayoutExportProjectConfig,
                cx,
            )
            .into_any_element(),
        ))
        .child(setting_row(
            style,
            theme,
            "Edit layout TOML",
            "Edit the selected project layout file.",
            settings_button(
                "settings-layout-edit",
                "Open",
                false,
                theme,
                cx.listener(move |this, _, _window, cx| {
                    if has_project {
                        let _ = this.open_layout_toml_editor();
                    }
                    cx.notify();
                }),
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
    let diagnostics = if root.keybinding_warning_lines.is_empty() {
        "No keybinding conflicts".to_string()
    } else {
        root.keybinding_warning_lines.join("; ")
    };

    let mut rows = div()
        .flex()
        .flex_col()
        .child(setting_row(
            style,
            theme,
            "Edit keybindings TOML",
            "Open the user keybindings file.",
            settings_command_button(
                "settings-keybindings-open",
                "Open",
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
            "Keybinding diagnostics",
            "Show invalid commands and shortcut conflicts.",
            settings_value(diagnostics, theme).into_any_element(),
        ));

    for row in root.visible_keybinding_rows() {
        let command = row.command;
        let keys = if row.keys.is_empty() {
            "Unbound".to_string()
        } else {
            row.keys.join(", ")
        };
        let title = row.title;
        let description = row.command_id;
        let title_text = if row.has_conflict {
            format!("{title} (conflict)")
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
                        .child(settings_value(keys, theme))
                        .child(settings_button(
                            format!("settings-keybinding-edit-{}", row.command_id),
                            "Edit",
                            false,
                            theme,
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.open_keybinding_edit_dialog(command);
                                cx.notify();
                            }),
                        ))
                        .child(settings_button(
                            format!("settings-keybinding-reset-{}", row.command_id),
                            "Reset",
                            false,
                            theme,
                            cx.listener(move |this, _, _window, cx| {
                                let _ = this.reset_keybinding_command_keys(command);
                                cx.notify();
                            }),
                        ))
                        .child(settings_button(
                            format!("settings-keybinding-delete-{}", row.command_id),
                            "Delete",
                            false,
                            theme,
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
    let title = title.into();
    let description = description.into();
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap_6()
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
                        .child(title),
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
                .justify_end()
                .items_center()
                .w(style.control_width)
                .flex_none()
                .child(control),
        )
}

fn settings_select_control(
    select: Entity<SettingsStringSelectState>,
    style: SettingsPanelStyle,
    searchable: bool,
    search_placeholder: &'static str,
) -> Select<SearchableVec<String>> {
    Select::new(&select)
        .small()
        .menu_width(style.select_menu_width)
        .search_placeholder(search_placeholder)
        .appearance(true)
        .w(style.control_width)
        .h(style.control_height)
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
) -> Stateful<Div> {
    settings_button(
        id,
        label,
        false,
        theme,
        cx.listener(move |this, _, _window, cx| {
            if enabled {
                let _ = this.run_command(command);
            }
            cx.notify();
        }),
    )
}

fn settings_button<H>(
    _id: impl Into<String>,
    label: impl Into<String>,
    selected: bool,
    theme: WorkbenchTheme,
    on_click: H,
) -> Stateful<Div>
where
    H: Fn(&ClickEvent, &mut Window, &mut gpui::App) + 'static,
{
    let id: String = _id.into();
    let label: String = label.into();
    let background = if selected {
        theme.active_surface
    } else {
        theme.surface_elevated
    };
    div()
        .id(SharedString::from(id))
        .flex()
        .items_center()
        .justify_center()
        .h_7()
        .rounded_sm()
        .border_1()
        .border_color(theme.border)
        .bg(background)
        .px_3()
        .text_xs()
        .text_color(theme.text)
        .hover(move |this| this.bg(theme.hover_surface))
        .on_click(on_click)
        .child(label)
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

fn error_banner(message: &str) -> Div {
    div()
        .absolute()
        .top_4()
        .left_4()
        .right_4()
        .child(Alert::error("root-error-banner", message.to_string()).banner())
}

fn push_component_notification(
    root: Entity<RootView>,
    event: NotificationEvent,
    window: &mut Window,
    cx: &mut Context<RootView>,
) {
    let item = toast_item_for_event(&event);
    let notification_type = match item.tone {
        ToastTone::Success => NotificationType::Success,
        ToastTone::Error => NotificationType::Error,
    };

    let focus_event = event.clone();
    window.push_notification(
        Notification::new()
            .title(item.title)
            .message(item.context)
            .with_type(notification_type)
            .on_click(move |_, _window, cx| {
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
