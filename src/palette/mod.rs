use std::path::PathBuf;

use crate::{
    commands::{CommandId, CommandRegistry},
    model::{
        layout::{LayoutNode, PaneKind},
        workspace::{PaneProcessState, TabStartState, Workspace},
    },
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
    registry
        .commands()
        .iter()
        .map(|command| {
            let presentation = command.id.presentation();
            let availability = command.id.availability(context.has_selected_project);
            PaletteItem {
                id: command.id.as_str().to_string(),
                title: presentation.title.to_string(),
                subtitle: Some(presentation.description.to_string()),
                status: None,
                command: command.id,
                enabled: availability.enabled,
                disabled_reason: availability.disabled_reason.map(ToOwned::to_owned),
            }
        })
        .collect()
}

pub fn project_palette_items(
    workspace: &Workspace,
    recent_projects: &[RecentProject],
) -> Vec<PaletteItem> {
    let mut items: Vec<_> = workspace
        .opened_projects()
        .iter()
        .map(|project| PaletteItem {
            id: project.id.as_str().to_string(),
            title: project.layout.project.name.clone(),
            subtitle: Some(project.path.display().to_string()),
            status: Some("open".to_string()),
            command: CommandId::ProjectPalette,
            enabled: true,
            disabled_reason: None,
        })
        .collect();

    items.extend(recent_projects.iter().map(|project| PaletteItem {
        id: project.path.display().to_string(),
        title: project.title.clone(),
        subtitle: Some(project.path.display().to_string()),
        status: Some("recent".to_string()),
        command: CommandId::ProjectOpenRecent,
        enabled: true,
        disabled_reason: None,
    }));

    items
}

pub fn tab_palette_items(workspace: &Workspace) -> Option<Vec<PaletteItem>> {
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
                let status = tab_state.map(|state| match state.start_state {
                    TabStartState::Lazy => {
                        tab_status(tab.id == project.selected_tab_id, TabStartState::Lazy)
                    }
                    TabStartState::Started => {
                        tab_status(tab.id == project.selected_tab_id, TabStartState::Started)
                    }
                });

                PaletteItem {
                    id: tab.id.clone(),
                    title: tab.title.clone(),
                    subtitle: Some(pane_count_label(pane_count)),
                    status,
                    command: CommandId::TabPalette,
                    enabled: true,
                    disabled_reason: None,
                }
            })
            .collect(),
    )
}

pub fn pane_palette_items(workspace: &Workspace) -> Option<Vec<PaletteItem>> {
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
                let is_agent = pane_config
                    .map(|pane_config| pane_config.kind == PaneKind::Agent)
                    .unwrap_or(false);

                PaletteItem {
                    id: pane.pane_id.clone(),
                    title,
                    subtitle: Some(tab.title.clone()),
                    status: Some(pane_status(pane.process_state, is_active, is_agent)),
                    command: CommandId::PanePalette,
                    enabled: true,
                    disabled_reason: None,
                }
            })
            .collect(),
    )
}

fn pane_count_label(pane_count: usize) -> String {
    if pane_count == 1 {
        "1 pane".to_string()
    } else {
        format!("{pane_count} panes")
    }
}

fn tab_status(is_active: bool, state: TabStartState) -> String {
    let mut parts = Vec::new();
    if is_active {
        parts.push("active");
    }
    parts.push(match state {
        TabStartState::Lazy => "lazy",
        TabStartState::Started => "started",
    });
    parts.join(" · ")
}

fn pane_status(state: PaneProcessState, is_active: bool, is_agent: bool) -> String {
    let mut parts = Vec::new();
    if is_active {
        parts.push("active");
    }
    parts.push(process_status_label(state));
    if is_agent {
        parts.push("agent");
    }
    parts.join(" · ")
}

fn pane_count(layout: &LayoutNode) -> usize {
    match layout {
        LayoutNode::Pane(_) => 1,
        LayoutNode::Split(split) => pane_count(&split.left) + pane_count(&split.right),
    }
}

fn process_status_label(state: PaneProcessState) -> &'static str {
    match state {
        PaneProcessState::Idle => "idle",
        PaneProcessState::Running => "running",
        PaneProcessState::Exited => "exited",
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
