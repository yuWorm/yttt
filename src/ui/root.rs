use gpui::{
    AnyElement, ClickEvent, Context, Div, Entity, FocusHandle, InteractiveElement as _,
    IntoElement, KeyDownEvent, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent,
    PathPromptOptions, Pixels, Point, Render, ScrollHandle, Subscription, Window, div, prelude::*,
    relative, rgb, rgba,
};
use gpui_component::{
    Root as ComponentRoot, WindowExt as _,
    alert::Alert,
    button::{Button, ButtonVariants as _},
    input::{Input, InputEvent, InputState},
    notification::{Notification, NotificationType},
};

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};

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
            SettingsKeybindings, SettingsNotifications, TabClose, TabNew, TabNext, TabPrev,
            TabRename, WORKSPACE_CONTEXT,
        },
        components::{ActionEmphasis, workbench_action_button},
        i18n::{UiText, UiTextKey},
        palette::palette_overlay,
        palette_surface::palette_input_placeholder,
        sidebar::project_sidebar,
        split_view::{pointer_resize_for_drag_delta, split_child_basis},
        tabs::project_tabs,
        terminal_pane::{
            TerminalPaneContext, TerminalPaneEvent, TerminalPaneExitedEvent, TerminalPaneView,
        },
        theme::WorkbenchTheme,
        titlebar::{TitlebarInfo, compact_path_for_titlebar, workbench_titlebar},
        toast::{ToastQueue, ToastTone, toast_item_for_event},
    },
};

pub struct RootView {
    workspace: Workspace,
    config_paths: AppConfigPaths,
    active_palette: Option<ActivePalette>,
    command_registry: CommandRegistry,
    recent_projects: Vec<RecentProject>,
    load_error: Option<String>,
    layout_source_messages: HashMap<ProjectId, String>,
    keybinding_warning_lines: Vec<String>,
    last_opened_layout_file: Option<PathBuf>,
    last_opened_keybindings_file: Option<PathBuf>,
    pending_close_project_id: Option<ProjectId>,
    pending_tab_rename: Option<PendingTabRename>,
    focus_handle: Option<FocusHandle>,
    palette_input: Option<Entity<InputState>>,
    palette_input_subscription: Option<Subscription>,
    palette_input_needs_focus: bool,
    tab_rename_input: Option<Entity<InputState>>,
    tab_rename_input_subscription: Option<Subscription>,
    tab_rename_input_needs_focus: bool,
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
    theme: WorkbenchTheme,
}

const EMPTY_WORKSPACE_ACTIONS: [UiTextKey; 3] = [
    UiTextKey::OpenDirectory,
    UiTextKey::OpenRecent,
    UiTextKey::CommandPalette,
];

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

        Self {
            workspace,
            config_paths,
            active_palette: None,
            command_registry,
            recent_projects,
            load_error,
            layout_source_messages: HashMap::new(),
            keybinding_warning_lines,
            last_opened_layout_file: None,
            last_opened_keybindings_file: None,
            pending_close_project_id: None,
            pending_tab_rename: None,
            focus_handle: None,
            palette_input: None,
            palette_input_subscription: None,
            palette_input_needs_focus: false,
            tab_rename_input: None,
            tab_rename_input_subscription: None,
            tab_rename_input_needs_focus: false,
            palette_scroll_handle: ScrollHandle::new(),
            sidebar_collapsed: false,
            active_split_resize_drag: None,
            pending_terminal_focus_pane_id: None,
            terminal_panes: HashMap::new(),
            terminal_pane_subscriptions: HashMap::new(),
            project_git_statuses: HashMap::new(),
            toast_queue: ToastQueue::default(),
            system_notifier: NoopSystemNotifier,
            system_notifications_enabled: false,
            ui_text: UiText::english(),
            theme: WorkbenchTheme::dark(),
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

    pub fn active_palette(&self) -> Option<&ActivePalette> {
        self.active_palette.as_ref()
    }

    pub fn open_palette(&mut self, kind: PaletteKind) {
        self.active_palette = Some(ActivePalette::new(kind));
        self.reset_palette_input();
        self.palette_scroll_handle = ScrollHandle::new();
        self.palette_input_needs_focus = true;
    }

    pub fn close_palette(&mut self) {
        self.active_palette = None;
        self.reset_palette_input();
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
                Ok(())
            }
            Err(error) => self.fail_workspace_error(error),
        }
    }

    pub fn cancel_tab_rename_dialog(&mut self) {
        self.clear_tab_rename_dialog();
        self.queue_selected_terminal_focus();
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
                self.system_notifications_enabled = !self.system_notifications_enabled;
                self.load_error = Some(self.visible_notification_settings_message().to_string());
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
        Ok(())
    }

    pub fn cancel_pending_project_close(&mut self) {
        self.pending_close_project_id = None;
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
        Ok(())
    }

    fn clear_tab_rename_dialog(&mut self) {
        self.pending_tab_rename = None;
        self.reset_tab_rename_input();
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

    fn active_terminal_split_view(&mut self, window: &mut Window, cx: &mut Context<Self>) -> Div {
        self.prune_terminal_panes();

        let Some((project_id, project_path, project_title, tab_id, tab_title, layout)) =
            self.selected_tab_layout_clone()
        else {
            return project_empty_terminal_state(cx, &self.ui_text, &self.theme);
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
            .bg(self.theme.terminal_background)
            .text_color(self.theme.text)
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
                    .child(Self::split_resize_handle(split.direction, cx))
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
            };
            let pane_view = cx.new(|cx| TerminalPaneView::new(context, cx));
            let subscription = cx.subscribe_in(&pane_view, window, Self::on_terminal_pane_event);
            self.terminal_pane_subscriptions
                .insert(key.clone(), subscription);
            self.terminal_panes.insert(key, pane_view.clone());
            pane_view
        };

        let pane_id = input.pane.id.clone();
        if self.pending_terminal_focus_pane_id.as_deref() == Some(&pane_id) {
            if pane_view.update(cx, |pane, cx| pane.focus_terminal(window, cx)) {
                self.pending_terminal_focus_pane_id = None;
            }
        }

        let border_color = if input.is_focused {
            self.theme.focused_pane_border
        } else {
            rgba(0x00000000)
        };
        let mut wrapper = div()
            .flex()
            .flex_1()
            .border_1()
            .border_color(border_color)
            .bg(self.theme.terminal_background);
        wrapper
            .interactivity()
            .on_click(cx.listener(move |this, _, _window, cx| {
                let _ = this.focus_visible_terminal_pane(&pane_id);
                cx.notify();
            }));
        wrapper.child(pane_view)
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

    fn split_resize_handle(direction: SplitDirection, cx: &mut Context<Self>) -> AnyElement {
        let style = Self::visible_split_handle_style(direction);
        let theme = WorkbenchTheme::dark();
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

    fn on_settings_notifications(
        &mut self,
        _: &SettingsNotifications,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.dispatch_command_action(CommandId::SettingsNotifications, cx);
    }

    fn dispatch_command_action(&mut self, command_id: CommandId, cx: &mut Context<Self>) {
        if self.active_palette.is_some() || self.pending_tab_rename.is_some() {
            cx.propagate();
            return;
        }

        let _ = self.run_command(command_id);
        cx.notify();
    }

    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
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

impl Default for RootView {
    fn default() -> Self {
        Self::new()
    }
}

impl Render for RootView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let focus_handle = self.root_focus_handle(cx);

        let body = if self.workspace.opened_projects().is_empty() {
            empty_workspace(cx, &self.ui_text, &self.theme)
        } else {
            let split_view = self.active_terminal_split_view(window, cx);

            div()
                .flex()
                .flex_1()
                .relative()
                .bg(self.theme.app_background)
                .text_color(self.theme.text)
                .child(project_sidebar(
                    &self.workspace,
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
            .bg(self.theme.app_background)
            .text_color(self.theme.text)
            .child(workbench_titlebar(self.visible_titlebar_info(), self.theme))
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
                    self.theme,
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
        if self.pending_tab_rename.is_some() {
            if let Some(input) = self.tab_rename_input(window, cx) {
                root = root.child(tab_rename_dialog(cx, &self.ui_text, &input, self.theme));
            }
        }
        if self.pending_close_project_id.is_some() {
            root = root.child(close_project_dialog(cx, &self.ui_text));
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

        if !focus_handle.contains_focused(window, cx) {
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

fn terminal_pane_key(project_id: &str, tab_id: &str, pane_id: &str) -> String {
    format!("{project_id}:{tab_id}:{pane_id}")
}

fn collect_terminal_pane_keys(
    project_id: &str,
    tab_id: &str,
    layout: &LayoutNode,
    keys: &mut HashSet<String>,
) {
    match layout {
        LayoutNode::Pane(pane) => {
            keys.insert(terminal_pane_key(project_id, tab_id, &pane.id));
        }
        LayoutNode::Split(split) => {
            collect_terminal_pane_keys(project_id, tab_id, &split.left, keys);
            collect_terminal_pane_keys(project_id, tab_id, &split.right, keys);
        }
    }
}

fn opens_palette_command(command_id: CommandId) -> bool {
    matches!(
        command_id,
        CommandId::CommandPaletteOpen
            | CommandId::ProjectPalette
            | CommandId::TabPalette
            | CommandId::PanePalette
    )
}

fn collect_terminal_pane_contexts(
    project_id: &str,
    project_path: &Path,
    project_title: &str,
    tab_id: &str,
    tab_title: &str,
    layout: &LayoutNode,
    focused_pane_id: Option<&str>,
    contexts: &mut Vec<TerminalPaneContext>,
) {
    match layout {
        LayoutNode::Pane(pane) => contexts.push(TerminalPaneContext {
            project_id: project_id.to_string(),
            project_path: project_path.to_path_buf(),
            project_title: project_title.to_string(),
            tab_id: tab_id.to_string(),
            tab_title: tab_title.to_string(),
            pane: pane.clone(),
            is_focused: focused_pane_id == Some(pane.id.as_str()),
        }),
        LayoutNode::Split(split) => {
            collect_terminal_pane_contexts(
                project_id,
                project_path,
                project_title,
                tab_id,
                tab_title,
                &split.left,
                focused_pane_id,
                contexts,
            );
            collect_terminal_pane_contexts(
                project_id,
                project_path,
                project_title,
                tab_id,
                tab_title,
                &split.right,
                focused_pane_id,
                contexts,
            );
        }
    }
}

fn recent_projects_for_palette(config: RecentProjectsConfig) -> Vec<RecentProject> {
    config
        .projects
        .into_iter()
        .map(|project| RecentProject {
            title: project.title,
            path: project.path,
        })
        .collect()
}

fn should_focus_terminal_after_command(command_id: CommandId) -> bool {
    matches!(
        command_id,
        CommandId::TabNew
            | CommandId::TabClose
            | CommandId::TabNext
            | CommandId::TabPrev
            | CommandId::PaneSplitVertical
            | CommandId::PaneSplitHorizontal
            | CommandId::PaneClose
            | CommandId::PaneFocusLeft
            | CommandId::PaneFocusRight
            | CommandId::PaneFocusUp
            | CommandId::PaneFocusDown
    )
}

fn layout_source_message(source: &LayoutSource) -> String {
    let source_name = match source {
        LayoutSource::ProjectConfig(_) => "project config",
        LayoutSource::ProjectConfigWithAppOverride { .. } => "project config + app-local override",
        LayoutSource::AppLocalConfig(_) => "app-local layout",
        LayoutSource::CreatedAppLocalDefault(_) => "created app-local default",
    };

    format!("Layout source: {source_name}")
}

fn load_keybindings_messages(
    paths: &AppConfigPaths,
    registry: &CommandRegistry,
) -> (Option<String>, Vec<String>) {
    match load_keybindings(paths, registry) {
        Ok(loaded) if loaded.warnings.is_empty() => (None, Vec::new()),
        Ok(loaded) => {
            let lines = format_keybinding_warning_lines(&loaded.warnings);
            (Some(format!("Keybindings: {}", lines.join("; "))), lines)
        }
        Err(error) => (Some(error.to_string()), Vec::new()),
    }
}

fn format_keybinding_warning_lines(warnings: &[KeybindingLoadWarning]) -> Vec<String> {
    warnings
        .iter()
        .map(|warning| match warning {
            KeybindingLoadWarning::Conflict(conflict) => {
                format!("Conflicting keybinding: {}", conflict.keys)
            }
            KeybindingLoadWarning::InvalidCommand(command) => {
                format!("Invalid command id: {command}")
            }
        })
        .collect()
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

fn tab_rename_dialog(
    cx: &mut Context<RootView>,
    ui_text: &UiText,
    input: &Entity<InputState>,
    theme: WorkbenchTheme,
) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .items_start()
        .justify_center()
        .pt_16()
        .bg(rgba(0x00000066))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w_96()
                .rounded_md()
                .border_1()
                .border_color(theme.border_strong)
                .bg(theme.surface)
                .p_4()
                .text_color(theme.text)
                .shadow_lg()
                .child(
                    div()
                        .text_sm()
                        .font_weight(gpui::FontWeight::SEMIBOLD)
                        .child(ui_text.get(UiTextKey::RenameTabTitle)),
                )
                .child(Input::new(input).cleanable(false).appearance(false))
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(ui_text.get(UiTextKey::RenameTabHint)),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-tab-rename")
                                .label(ui_text.get(UiTextKey::Cancel))
                                .outline()
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    this.cancel_tab_rename_dialog();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-tab-rename")
                                .label(ui_text.get(UiTextKey::RenameTabAction))
                                .primary()
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    let _ = this.confirm_tab_rename_dialog_from_input(cx);
                                    cx.notify();
                                })),
                        ),
                ),
        )
}

fn close_project_dialog(cx: &mut Context<RootView>, ui_text: &UiText) -> Div {
    div()
        .absolute()
        .top_0()
        .left_0()
        .right_0()
        .bottom_0()
        .flex()
        .items_center()
        .justify_center()
        .bg(rgba(0x00000099))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_3()
                .w_96()
                .rounded_md()
                .border_1()
                .border_color(rgb(0x3a3a3a))
                .bg(rgb(0x151515))
                .p_4()
                .text_color(rgb(0xf5f5f5))
                .child(
                    div()
                        .text_lg()
                        .child(ui_text.get(UiTextKey::CloseProjectTitle)),
                )
                .child(
                    Alert::warning(
                        "close-project-warning",
                        ui_text.get(UiTextKey::CloseProjectBody),
                    )
                    .banner(),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(rgb(0x737373))
                        .child("Enter to close, Escape to cancel"),
                )
                .child(
                    div()
                        .flex()
                        .justify_end()
                        .gap_2()
                        .child(
                            Button::new("cancel-close-project")
                                .label(ui_text.get(UiTextKey::Cancel))
                                .outline()
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    this.cancel_pending_project_close();
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("confirm-close-project")
                                .label(ui_text.get(UiTextKey::CloseProjectAction))
                                .danger()
                                .on_click(cx.listener(|this, _, _window, cx| {
                                    let _ = this.confirm_pending_project_close();
                                    cx.notify();
                                })),
                        ),
                ),
        )
}

fn empty_workspace(cx: &mut Context<RootView>, ui_text: &UiText, theme: &WorkbenchTheme) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_5()
        .flex_1()
        .w_full()
        .relative()
        .justify_center()
        .items_center()
        .bg(theme.app_background)
        .text_color(theme.text)
        .child(div().text_xl().child(ui_text.get(UiTextKey::AppName)))
        .child(
            div()
                .flex()
                .flex_col()
                .gap_2()
                .items_center()
                .text_center()
                .child(
                    div()
                        .text_sm()
                        .text_color(theme.text_muted)
                        .child(ui_text.get(UiTextKey::EmptySubtitle)),
                )
                .child(
                    div()
                        .text_xs()
                        .text_color(theme.text_subtle)
                        .child(ui_text.get(UiTextKey::EmptySidebarNote)),
                ),
        )
        .child(
            div()
                .flex()
                .flex_wrap()
                .gap_2()
                .justify_center()
                .child(
                    workbench_action_button(
                        "empty-open-directory",
                        ui_text.get(UiTextKey::OpenDirectory),
                        "secondary-o",
                        ActionEmphasis::Primary,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_open_project(&OpenProject, window, cx);
                    })),
                )
                .child(
                    workbench_action_button(
                        "empty-open-recent",
                        ui_text.get(UiTextKey::OpenRecent),
                        "secondary-shift-o",
                        ActionEmphasis::Secondary,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_open_project_palette(&OpenProjectPalette, window, cx);
                    })),
                )
                .child(
                    workbench_action_button(
                        "empty-command-palette",
                        ui_text.get(UiTextKey::CommandPalette),
                        "secondary-p",
                        ActionEmphasis::Secondary,
                    )
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.on_open_command_palette(&OpenCommandPalette, window, cx);
                    })),
                ),
        )
}

fn project_empty_terminal_state(
    cx: &mut Context<RootView>,
    ui_text: &UiText,
    theme: &WorkbenchTheme,
) -> Div {
    div()
        .flex()
        .flex_col()
        .gap_3()
        .flex_1()
        .w_full()
        .justify_center()
        .items_center()
        .bg(theme.terminal_background)
        .text_color(theme.text)
        .child(
            div()
                .text_sm()
                .text_color(theme.text_muted)
                .child(ui_text.get(UiTextKey::NoTerminalTabs)),
        )
        .child(
            workbench_action_button(
                "project-empty-new-tab",
                ui_text.get(UiTextKey::NewTab),
                "secondary-t",
                ActionEmphasis::Primary,
            )
            .on_click(cx.listener(|this, _, _window, cx| {
                let _ = this.run_command(CommandId::TabNew);
                cx.notify();
            })),
        )
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
