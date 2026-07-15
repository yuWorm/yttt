use std::path::PathBuf;

use crate::{
    commands::{ActiveSurface, CommandContext, CommandId, CommandRegistry},
    model::{
        ids::ProjectId,
        layout::LayoutNode,
        workspace::{AgentStatus, OpenedProject, PaneProcessState, TabStartState, Workspace},
    },
    ui::{
        editor::{DocumentId, WorkItemId},
        i18n::{UiText, UiTextKey},
        terminal::status::{
            is_agent_pane, pane_agent_status, project_agent_status, tab_agent_status,
        },
    },
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKind {
    Command,
    NewTabCommand,
    Project,
    OpenedProject,
    RecentProject,
    Tab,
    Pane,
    GitBranch,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub status: Option<String>,
    pub keybinding: Option<String>,
    pub command: CommandId,
    pub enabled: bool,
    pub disabled_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecentProject {
    pub title: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TabPaletteSnapshot {
    id: WorkItemId,
    title: String,
    subtitle: Option<String>,
    status: Option<String>,
}

impl TabPaletteSnapshot {
    pub fn terminal(
        tab_id: impl Into<String>,
        title: impl Into<String>,
        subtitle: Option<String>,
        status: Option<String>,
    ) -> Self {
        Self {
            id: WorkItemId::Terminal(tab_id.into()),
            title: title.into(),
            subtitle,
            status,
        }
    }

    pub fn file(document_id: DocumentId, relative_path: PathBuf, status: Option<String>) -> Self {
        let title = relative_path
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| relative_path.to_string_lossy().into_owned());
        Self {
            id: WorkItemId::File(document_id),
            title,
            subtitle: Some(relative_path.to_string_lossy().into_owned()),
            status,
        }
    }

    pub fn id(&self) -> &WorkItemId {
        &self.id
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivePalette {
    pub kind: PaletteKind,
    pub query: String,
    pub selected_index: usize,
}

impl ActivePalette {
    pub fn new(kind: PaletteKind) -> Self {
        Self {
            kind,
            query: String::new(),
            selected_index: 0,
        }
    }

    pub fn filtered_items<'a>(&self, items: &'a [PaletteItem]) -> Vec<&'a PaletteItem> {
        let query = self.query.trim().to_lowercase();
        if query.is_empty() {
            return items.iter().collect();
        }

        items
            .iter()
            .filter(|item| item_matches_query(item, &query))
            .collect()
    }

    pub fn selected_item<'a>(&self, items: &'a [PaletteItem]) -> Option<&'a PaletteItem> {
        let filtered_items = self.filtered_items(items);
        filtered_items
            .get(
                self.selected_index
                    .min(filtered_items.len().saturating_sub(1)),
            )
            .copied()
    }

    pub fn select_next(&mut self, items: &[PaletteItem]) {
        let item_count = self.filtered_items(items).len();
        if item_count == 0 {
            self.selected_index = 0;
            return;
        }

        self.selected_index = (self.selected_index + 1) % item_count;
    }

    pub fn select_prev(&mut self, items: &[PaletteItem]) {
        let item_count = self.filtered_items(items).len();
        if item_count == 0 {
            self.selected_index = 0;
            return;
        }

        self.selected_index = if self.selected_index == 0 {
            item_count - 1
        } else {
            self.selected_index - 1
        };
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CommandPaletteContext {
    pub has_selected_project: bool,
    pub active_surface: ActiveSurface,
}

impl CommandPaletteContext {
    pub fn from_workspace(workspace: &Workspace) -> Self {
        let has_selected_project = workspace.selected_project_id().is_some();
        Self::from_command_context(CommandContext {
            has_selected_project,
            active_surface: if has_selected_project {
                ActiveSurface::Terminal
            } else {
                ActiveSurface::None
            },
        })
    }

    pub fn from_command_context(context: CommandContext) -> Self {
        Self {
            has_selected_project: context.has_selected_project,
            active_surface: context.active_surface,
        }
    }

    pub fn command_context(self) -> CommandContext {
        CommandContext {
            has_selected_project: self.has_selected_project,
            active_surface: self.active_surface,
        }
    }
}

pub fn command_palette_items(
    registry: &CommandRegistry,
    context: CommandPaletteContext,
) -> Vec<PaletteItem> {
    command_palette_items_with_text(registry, context, &UiText::english())
}

pub fn command_palette_items_with_text(
    registry: &CommandRegistry,
    context: CommandPaletteContext,
    ui_text: &UiText,
) -> Vec<PaletteItem> {
    registry
        .commands()
        .iter()
        .map(|command| {
            let availability = command
                .id
                .availability_for_context(context.command_context());
            PaletteItem {
                id: command.id.as_str().to_string(),
                title: command_title_with_text(command.id, ui_text).to_string(),
                subtitle: Some(command_description_with_text(command.id, ui_text).to_string()),
                status: None,
                keybinding: None,
                command: command.id,
                enabled: availability.enabled,
                disabled_reason: command_disabled_reason_key(availability.disabled_reason)
                    .map(|key| ui_text.get(key).to_string()),
            }
        })
        .collect()
}

pub fn new_tab_command_palette_items(commands: &[String]) -> Vec<PaletteItem> {
    commands
        .iter()
        .filter_map(|command| {
            let command = command.trim();
            (!command.is_empty()).then(|| PaletteItem {
                id: command.to_string(),
                title: command.to_string(),
                subtitle: None,
                status: None,
                keybinding: None,
                command: CommandId::TabNew,
                enabled: true,
                disabled_reason: None,
            })
        })
        .collect()
}

pub fn project_palette_items(
    workspace: &Workspace,
    recent_projects: &[RecentProject],
) -> Vec<PaletteItem> {
    project_palette_items_with_text(workspace, recent_projects, &UiText::english())
}

pub fn project_palette_items_with_text(
    workspace: &Workspace,
    recent_projects: &[RecentProject],
    ui_text: &UiText,
) -> Vec<PaletteItem> {
    let mut items = opened_project_palette_items_with_text(workspace, ui_text);
    items.extend(recent_project_palette_items_with_text(
        workspace,
        recent_projects,
        ui_text,
    ));
    items
}

pub fn opened_project_palette_items(workspace: &Workspace) -> Vec<PaletteItem> {
    opened_project_palette_items_with_text(workspace, &UiText::english())
}

pub fn opened_project_palette_items_with_text(
    workspace: &Workspace,
    ui_text: &UiText,
) -> Vec<PaletteItem> {
    workspace
        .opened_projects()
        .iter()
        .map(|project| PaletteItem {
            id: project.id.as_str().to_string(),
            title: project.layout.project.name.clone(),
            subtitle: Some(project.path.display().to_string()),
            status: Some(open_project_status(project, ui_text)),
            keybinding: None,
            command: CommandId::ProjectOpenedPalette,
            enabled: true,
            disabled_reason: None,
        })
        .collect()
}

pub fn recent_project_palette_items(
    workspace: &Workspace,
    recent_projects: &[RecentProject],
) -> Vec<PaletteItem> {
    recent_project_palette_items_with_text(workspace, recent_projects, &UiText::english())
}

pub fn recent_project_palette_items_with_text(
    workspace: &Workspace,
    recent_projects: &[RecentProject],
    ui_text: &UiText,
) -> Vec<PaletteItem> {
    recent_projects
        .iter()
        .filter(|recent| {
            workspace
                .opened_projects()
                .iter()
                .all(|opened| opened.path.as_path() != recent.path.as_path())
        })
        .map(|project| PaletteItem {
            id: project.path.display().to_string(),
            title: project.title.clone(),
            subtitle: Some(project.path.display().to_string()),
            status: Some(ui_text.get(UiTextKey::PaletteStatusRecent).to_string()),
            keybinding: None,
            command: CommandId::ProjectOpenRecent,
            enabled: true,
            disabled_reason: None,
        })
        .collect()
}

pub fn tab_palette_items(workspace: &Workspace) -> Option<Vec<PaletteItem>> {
    tab_palette_items_with_text(workspace, &UiText::english())
}

pub fn tab_palette_items_with_text(
    workspace: &Workspace,
    ui_text: &UiText,
) -> Option<Vec<PaletteItem>> {
    let selected_project_id = workspace.selected_project_id()?;
    let project = workspace.project(selected_project_id)?;

    let snapshots = project
        .layout
        .tabs
        .iter()
        .map(|tab| {
            let tab_state = project.tab_state(&tab.id);
            let pane_count = tab_state
                .map(|state| state.pane_states.len())
                .unwrap_or_else(|| pane_count(&tab.layout));
            let agent_status = tab_agent_status(project, &tab.id);
            let status = tab_state.map(|state| {
                tab_status(
                    tab.id == project.selected_tab_id,
                    state.start_state,
                    agent_status,
                    ui_text,
                )
            });

            TabPaletteSnapshot::terminal(
                tab.id.clone(),
                tab.title.clone(),
                Some(pane_count_label(pane_count, ui_text)),
                status,
            )
        })
        .collect::<Vec<_>>();

    Some(tab_palette_items_from_snapshots(&snapshots, false))
}

pub fn unified_tab_palette_items(snapshots: &[TabPaletteSnapshot]) -> Vec<PaletteItem> {
    tab_palette_items_from_snapshots(snapshots, true)
}

pub fn decode_tab_palette_item_id(id: &str, project_id: &ProjectId) -> Option<WorkItemId> {
    if let Some(tab_id) = id.strip_prefix("terminal:") {
        return (!tab_id.is_empty()).then(|| WorkItemId::Terminal(tab_id.to_string()));
    }
    let canonical_path = id.strip_prefix("file:")?;
    (!canonical_path.is_empty()).then(|| {
        WorkItemId::File(DocumentId {
            project_id: project_id.clone(),
            canonical_path: PathBuf::from(canonical_path),
        })
    })
}

fn tab_palette_items_from_snapshots(
    snapshots: &[TabPaletteSnapshot],
    prefix_ids: bool,
) -> Vec<PaletteItem> {
    snapshots
        .iter()
        .map(|snapshot| PaletteItem {
            id: tab_palette_item_id(&snapshot.id, prefix_ids),
            title: snapshot.title.clone(),
            subtitle: snapshot.subtitle.clone(),
            status: snapshot.status.clone(),
            keybinding: None,
            command: CommandId::TabPalette,
            enabled: true,
            disabled_reason: None,
        })
        .collect()
}

fn tab_palette_item_id(id: &WorkItemId, prefix_ids: bool) -> String {
    match id {
        WorkItemId::Terminal(tab_id) if prefix_ids => format!("terminal:{tab_id}"),
        WorkItemId::Terminal(tab_id) => tab_id.clone(),
        WorkItemId::File(document_id) => {
            format!("file:{}", document_id.canonical_path.to_string_lossy())
        }
    }
}

pub fn pane_palette_items(workspace: &Workspace) -> Option<Vec<PaletteItem>> {
    pane_palette_items_with_text(workspace, &UiText::english())
}

pub fn pane_palette_items_with_text(
    workspace: &Workspace,
    ui_text: &UiText,
) -> Option<Vec<PaletteItem>> {
    let selected_project_id = workspace.selected_project_id()?;
    let project = workspace.project(selected_project_id)?;
    let tab = project
        .layout
        .tabs
        .iter()
        .find(|tab| tab.id == project.selected_tab_id)?;
    let tab_state = project.tab_state(&tab.id)?;

    Some(
        tab_state
            .pane_states
            .iter()
            .map(|pane| {
                let pane_config = tab.layout.find_pane(&pane.pane_id);
                let title = pane_config
                    .map(|pane_config| pane_config.title.clone())
                    .unwrap_or_else(|| pane.pane_id.clone());
                let is_active = tab_state.focused_pane_id.as_deref() == Some(&pane.pane_id);
                let is_agent = pane_config.map(is_agent_pane).unwrap_or(false);
                let agent_status =
                    pane_config.and_then(|pane_config| pane_agent_status(pane_config, pane));

                PaletteItem {
                    id: pane.pane_id.clone(),
                    title,
                    subtitle: Some(tab.title.clone()),
                    status: Some(pane_status(
                        pane.process_state,
                        is_active,
                        is_agent,
                        agent_status,
                        ui_text,
                    )),
                    keybinding: None,
                    command: CommandId::PanePalette,
                    enabled: true,
                    disabled_reason: None,
                }
            })
            .collect(),
    )
}

pub fn command_title_with_text(command_id: CommandId, ui_text: &UiText) -> &'static str {
    ui_text.get(command_title_key(command_id))
}

pub fn command_description_with_text(command_id: CommandId, ui_text: &UiText) -> &'static str {
    ui_text.get(command_description_key(command_id))
}

fn command_title_key(command_id: CommandId) -> UiTextKey {
    match command_id {
        CommandId::ProjectCreate => UiTextKey::CommandProjectCreateTitle,
        CommandId::ProjectOpen => UiTextKey::CommandProjectOpenTitle,
        CommandId::ProjectOpenRecent => UiTextKey::CommandProjectOpenRecentTitle,
        CommandId::ProjectClose => UiTextKey::CommandProjectCloseTitle,
        CommandId::ProjectPalette => UiTextKey::CommandProjectPaletteTitle,
        CommandId::ProjectOpenedPalette => UiTextKey::CommandProjectOpenedPaletteTitle,
        CommandId::ProjectPanelToggle => UiTextKey::CommandProjectPanelToggleTitle,
        CommandId::ProjectPanelRefresh => UiTextKey::CommandProjectPanelRefreshTitle,
        CommandId::GitBranchSwitch => UiTextKey::CommandGitBranchSwitchTitle,
        CommandId::GitDiffOpen => UiTextKey::CommandGitDiffOpenTitle,
        CommandId::FileSave => UiTextKey::CommandFileSaveTitle,
        CommandId::TabNew => UiTextKey::CommandTabNewTitle,
        CommandId::TabClose => UiTextKey::CommandTabCloseTitle,
        CommandId::TabRename => UiTextKey::CommandTabRenameTitle,
        CommandId::TabNext => UiTextKey::CommandTabNextTitle,
        CommandId::TabPrev => UiTextKey::CommandTabPrevTitle,
        CommandId::TabPalette => UiTextKey::CommandTabPaletteTitle,
        CommandId::PaneSplitHorizontal => UiTextKey::CommandPaneSplitHorizontalTitle,
        CommandId::PaneSplitVertical => UiTextKey::CommandPaneSplitVerticalTitle,
        CommandId::PaneClose => UiTextKey::CommandPaneCloseTitle,
        CommandId::PaneFocusLeft => UiTextKey::CommandPaneFocusLeftTitle,
        CommandId::PaneFocusRight => UiTextKey::CommandPaneFocusRightTitle,
        CommandId::PaneFocusUp => UiTextKey::CommandPaneFocusUpTitle,
        CommandId::PaneFocusDown => UiTextKey::CommandPaneFocusDownTitle,
        CommandId::PaneResizeLeft => UiTextKey::CommandPaneResizeLeftTitle,
        CommandId::PaneResizeRight => UiTextKey::CommandPaneResizeRightTitle,
        CommandId::PaneResizeUp => UiTextKey::CommandPaneResizeUpTitle,
        CommandId::PaneResizeDown => UiTextKey::CommandPaneResizeDownTitle,
        CommandId::PaneRename => UiTextKey::CommandPaneRenameTitle,
        CommandId::PanePalette => UiTextKey::CommandPanePaletteTitle,
        CommandId::LayoutDefaultEdit => UiTextKey::CommandLayoutDefaultEditTitle,
        CommandId::LayoutDefaultReset => UiTextKey::CommandLayoutDefaultResetTitle,
        CommandId::LayoutDefaultReload => UiTextKey::CommandLayoutDefaultReloadTitle,
        CommandId::LayoutProjectEdit => UiTextKey::CommandLayoutProjectEditTitle,
        CommandId::LayoutSaveCurrent => UiTextKey::CommandLayoutSaveCurrentTitle,
        CommandId::LayoutExportProjectConfig => UiTextKey::CommandLayoutExportProjectConfigTitle,
        CommandId::LayoutResetLocalOverride => UiTextKey::CommandLayoutResetLocalOverrideTitle,
        CommandId::LayoutOpenFile => UiTextKey::CommandLayoutOpenFileTitle,
        CommandId::CommandPaletteOpen => UiTextKey::CommandPaletteOpenTitle,
        CommandId::SettingsOpen => UiTextKey::CommandSettingsOpenTitle,
        CommandId::SettingsKeybindings => UiTextKey::CommandSettingsKeybindingsTitle,
        CommandId::SettingsNotifications => UiTextKey::CommandSettingsNotificationsTitle,
    }
}

fn command_description_key(command_id: CommandId) -> UiTextKey {
    match command_id {
        CommandId::ProjectCreate => UiTextKey::CommandProjectCreateDescription,
        CommandId::ProjectOpen => UiTextKey::CommandProjectOpenDescription,
        CommandId::ProjectOpenRecent => UiTextKey::CommandProjectOpenRecentDescription,
        CommandId::ProjectClose => UiTextKey::CommandProjectCloseDescription,
        CommandId::ProjectPalette => UiTextKey::CommandProjectPaletteDescription,
        CommandId::ProjectOpenedPalette => UiTextKey::CommandProjectOpenedPaletteDescription,
        CommandId::ProjectPanelToggle => UiTextKey::CommandProjectPanelToggleDescription,
        CommandId::ProjectPanelRefresh => UiTextKey::CommandProjectPanelRefreshDescription,
        CommandId::GitBranchSwitch => UiTextKey::CommandGitBranchSwitchDescription,
        CommandId::GitDiffOpen => UiTextKey::CommandGitDiffOpenDescription,
        CommandId::FileSave => UiTextKey::CommandFileSaveDescription,
        CommandId::TabNew => UiTextKey::CommandTabNewDescription,
        CommandId::TabClose => UiTextKey::CommandTabCloseDescription,
        CommandId::TabRename => UiTextKey::CommandTabRenameDescription,
        CommandId::TabNext => UiTextKey::CommandTabNextDescription,
        CommandId::TabPrev => UiTextKey::CommandTabPrevDescription,
        CommandId::TabPalette => UiTextKey::CommandTabPaletteDescription,
        CommandId::PaneSplitHorizontal => UiTextKey::CommandPaneSplitHorizontalDescription,
        CommandId::PaneSplitVertical => UiTextKey::CommandPaneSplitVerticalDescription,
        CommandId::PaneClose => UiTextKey::CommandPaneCloseDescription,
        CommandId::PaneFocusLeft => UiTextKey::CommandPaneFocusLeftDescription,
        CommandId::PaneFocusRight => UiTextKey::CommandPaneFocusRightDescription,
        CommandId::PaneFocusUp => UiTextKey::CommandPaneFocusUpDescription,
        CommandId::PaneFocusDown => UiTextKey::CommandPaneFocusDownDescription,
        CommandId::PaneResizeLeft => UiTextKey::CommandPaneResizeLeftDescription,
        CommandId::PaneResizeRight => UiTextKey::CommandPaneResizeRightDescription,
        CommandId::PaneResizeUp => UiTextKey::CommandPaneResizeUpDescription,
        CommandId::PaneResizeDown => UiTextKey::CommandPaneResizeDownDescription,
        CommandId::PaneRename => UiTextKey::CommandPaneRenameDescription,
        CommandId::PanePalette => UiTextKey::CommandPanePaletteDescription,
        CommandId::LayoutDefaultEdit => UiTextKey::CommandLayoutDefaultEditDescription,
        CommandId::LayoutDefaultReset => UiTextKey::CommandLayoutDefaultResetDescription,
        CommandId::LayoutDefaultReload => UiTextKey::CommandLayoutDefaultReloadDescription,
        CommandId::LayoutProjectEdit => UiTextKey::CommandLayoutProjectEditDescription,
        CommandId::LayoutSaveCurrent => UiTextKey::CommandLayoutSaveCurrentDescription,
        CommandId::LayoutExportProjectConfig => {
            UiTextKey::CommandLayoutExportProjectConfigDescription
        }
        CommandId::LayoutResetLocalOverride => {
            UiTextKey::CommandLayoutResetLocalOverrideDescription
        }
        CommandId::LayoutOpenFile => UiTextKey::CommandLayoutOpenFileDescription,
        CommandId::CommandPaletteOpen => UiTextKey::CommandPaletteOpenDescription,
        CommandId::SettingsOpen => UiTextKey::CommandSettingsOpenDescription,
        CommandId::SettingsKeybindings => UiTextKey::CommandSettingsKeybindingsDescription,
        CommandId::SettingsNotifications => UiTextKey::CommandSettingsNotificationsDescription,
    }
}

fn command_disabled_reason_key(reason: Option<&str>) -> Option<UiTextKey> {
    match reason {
        None => None,
        Some("Open a project first") => Some(UiTextKey::CommandDisabledOpenProjectFirst),
        Some("Focus a project file first") => Some(UiTextKey::CommandDisabledFocusProjectFileFirst),
        Some("Open a terminal or file first") => Some(UiTextKey::CommandDisabledOpenWorkItemFirst),
        Some("Switch to a terminal tab first") => {
            Some(UiTextKey::CommandDisabledSwitchTerminalFirst)
        }
        Some(_) => Some(UiTextKey::CommandUnavailable),
    }
}

fn open_project_status(project: &OpenedProject, ui_text: &UiText) -> String {
    let mut parts = vec![ui_text.get(UiTextKey::PaletteStatusOpen)];
    if let Some(agent_status) = project_agent_status(project) {
        parts.push(agent_status_label(agent_status, ui_text));
    }
    parts.join(" · ")
}

fn pane_count_label(pane_count: usize, ui_text: &UiText) -> String {
    let unit = if pane_count == 1 {
        ui_text.get(UiTextKey::PaletteStatusPaneSingular)
    } else {
        ui_text.get(UiTextKey::PaletteStatusPanePlural)
    };
    format!("{pane_count} {unit}")
}

fn tab_status(
    is_active: bool,
    state: TabStartState,
    agent_status: Option<AgentStatus>,
    ui_text: &UiText,
) -> String {
    let mut parts = Vec::new();
    if is_active {
        parts.push(ui_text.get(UiTextKey::PaletteStatusActive));
    }
    parts.push(match state {
        TabStartState::Lazy => ui_text.get(UiTextKey::PaletteStatusLazy),
        TabStartState::Started => ui_text.get(UiTextKey::PaletteStatusStarted),
    });
    if let Some(agent_status) = agent_status {
        parts.push(agent_status_label(agent_status, ui_text));
    }
    parts.join(" · ")
}

fn pane_status(
    state: PaneProcessState,
    is_active: bool,
    is_agent: bool,
    agent_status: Option<AgentStatus>,
    ui_text: &UiText,
) -> String {
    let mut parts = Vec::new();
    if is_active {
        parts.push(ui_text.get(UiTextKey::PaletteStatusActive));
    }
    parts.push(process_status_label(state, ui_text));
    if let Some(agent_status) = agent_status {
        parts.push(agent_status_label(agent_status, ui_text));
    } else if is_agent {
        parts.push(ui_text.get(UiTextKey::PaletteStatusAgent));
    }
    parts.join(" · ")
}

fn pane_count(layout: &LayoutNode) -> usize {
    match layout {
        LayoutNode::Pane(_) => 1,
        LayoutNode::Split(split) => pane_count(&split.left) + pane_count(&split.right),
    }
}

fn process_status_label(state: PaneProcessState, ui_text: &UiText) -> &'static str {
    match state {
        PaneProcessState::Idle => ui_text.get(UiTextKey::PaletteStatusIdle),
        PaneProcessState::Running => ui_text.get(UiTextKey::PaletteStatusRunning),
        PaneProcessState::Exited => ui_text.get(UiTextKey::PaletteStatusExited),
    }
}

fn agent_status_label(status: AgentStatus, ui_text: &UiText) -> &'static str {
    match status {
        AgentStatus::Running => ui_text.get(UiTextKey::PaletteStatusAgentRunning),
        AgentStatus::Completed => ui_text.get(UiTextKey::PaletteStatusAgentCompleted),
        AgentStatus::Failed => ui_text.get(UiTextKey::PaletteStatusAgentFailed),
    }
}

fn item_matches_query(item: &PaletteItem, query: &str) -> bool {
    item.id.to_lowercase().contains(query)
        || item.title.to_lowercase().contains(query)
        || item
            .subtitle
            .as_deref()
            .map(|subtitle| subtitle.to_lowercase().contains(query))
            .unwrap_or(false)
        || item
            .status
            .as_deref()
            .map(|status| status.to_lowercase().contains(query))
            .unwrap_or(false)
}
