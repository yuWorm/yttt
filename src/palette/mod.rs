use std::path::PathBuf;

use crate::{
    commands::{CommandId, CommandRegistry},
    model::{
        layout::LayoutNode,
        workspace::{AgentStatus, OpenedProject, PaneProcessState, TabStartState, Workspace},
    },
    ui::agent_status::{is_agent_pane, pane_agent_status, project_agent_status, tab_agent_status},
    ui::i18n::{UiText, UiTextKey},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaletteKind {
    Command,
    Project,
    Tab,
    Pane,
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
}

impl CommandPaletteContext {
    pub fn from_workspace(workspace: &Workspace) -> Self {
        Self {
            has_selected_project: workspace.selected_project_id().is_some(),
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
            let availability = command.id.availability(context.has_selected_project);
            PaletteItem {
                id: command.id.as_str().to_string(),
                title: ui_text.get(command_title_key(command.id)).to_string(),
                subtitle: Some(ui_text.get(command_description_key(command.id)).to_string()),
                status: None,
                keybinding: None,
                command: command.id,
                enabled: availability.enabled,
                disabled_reason: command_disabled_reason_key(
                    command.id,
                    context.has_selected_project,
                )
                .map(|key| ui_text.get(key).to_string()),
            }
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
    let mut items: Vec<_> = workspace
        .opened_projects()
        .iter()
        .map(|project| PaletteItem {
            id: project.id.as_str().to_string(),
            title: project.layout.project.name.clone(),
            subtitle: Some(project.path.display().to_string()),
            status: Some(open_project_status(project, ui_text)),
            keybinding: None,
            command: CommandId::ProjectPalette,
            enabled: true,
            disabled_reason: None,
        })
        .collect();

    items.extend(recent_projects.iter().map(|project| PaletteItem {
        id: project.path.display().to_string(),
        title: project.title.clone(),
        subtitle: Some(project.path.display().to_string()),
        status: Some(ui_text.get(UiTextKey::PaletteStatusRecent).to_string()),
        keybinding: None,
        command: CommandId::ProjectOpenRecent,
        enabled: true,
        disabled_reason: None,
    }));

    items
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

    Some(
        project
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

                PaletteItem {
                    id: tab.id.clone(),
                    title: tab.title.clone(),
                    subtitle: Some(pane_count_label(pane_count, ui_text)),
                    status,
                    keybinding: None,
                    command: CommandId::TabPalette,
                    enabled: true,
                    disabled_reason: None,
                }
            })
            .collect(),
    )
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

fn command_title_key(command_id: CommandId) -> UiTextKey {
    match command_id {
        CommandId::ProjectOpen => UiTextKey::CommandProjectOpenTitle,
        CommandId::ProjectOpenRecent => UiTextKey::CommandProjectOpenRecentTitle,
        CommandId::ProjectClose => UiTextKey::CommandProjectCloseTitle,
        CommandId::ProjectPalette => UiTextKey::CommandProjectPaletteTitle,
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
        CommandId::LayoutSaveCurrent => UiTextKey::CommandLayoutSaveCurrentTitle,
        CommandId::LayoutExportProjectConfig => UiTextKey::CommandLayoutExportProjectConfigTitle,
        CommandId::LayoutOpenFile => UiTextKey::CommandLayoutOpenFileTitle,
        CommandId::CommandPaletteOpen => UiTextKey::CommandPaletteOpenTitle,
        CommandId::SettingsOpen => UiTextKey::CommandSettingsOpenTitle,
        CommandId::SettingsKeybindings => UiTextKey::CommandSettingsKeybindingsTitle,
        CommandId::SettingsNotifications => UiTextKey::CommandSettingsNotificationsTitle,
    }
}

fn command_description_key(command_id: CommandId) -> UiTextKey {
    match command_id {
        CommandId::ProjectOpen => UiTextKey::CommandProjectOpenDescription,
        CommandId::ProjectOpenRecent => UiTextKey::CommandProjectOpenRecentDescription,
        CommandId::ProjectClose => UiTextKey::CommandProjectCloseDescription,
        CommandId::ProjectPalette => UiTextKey::CommandProjectPaletteDescription,
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
        CommandId::LayoutSaveCurrent => UiTextKey::CommandLayoutSaveCurrentDescription,
        CommandId::LayoutExportProjectConfig => {
            UiTextKey::CommandLayoutExportProjectConfigDescription
        }
        CommandId::LayoutOpenFile => UiTextKey::CommandLayoutOpenFileDescription,
        CommandId::CommandPaletteOpen => UiTextKey::CommandPaletteOpenDescription,
        CommandId::SettingsOpen => UiTextKey::CommandSettingsOpenDescription,
        CommandId::SettingsKeybindings => UiTextKey::CommandSettingsKeybindingsDescription,
        CommandId::SettingsNotifications => UiTextKey::CommandSettingsNotificationsDescription,
    }
}

fn command_disabled_reason_key(
    command_id: CommandId,
    has_selected_project: bool,
) -> Option<UiTextKey> {
    match command_id {
        CommandId::ProjectOpen | CommandId::ProjectOpenRecent => None,
        CommandId::CommandPaletteOpen
        | CommandId::ProjectPalette
        | CommandId::SettingsOpen
        | CommandId::SettingsKeybindings
        | CommandId::SettingsNotifications => None,
        _ if has_selected_project => None,
        _ => Some(UiTextKey::CommandDisabledOpenProjectFirst),
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
