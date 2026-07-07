use std::path::PathBuf;

use crate::{
    commands::{CommandId, CommandRegistry},
    model::{
        layout::LayoutNode,
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

pub fn command_palette_items(registry: &CommandRegistry) -> Vec<PaletteItem> {
    registry
        .commands()
        .iter()
        .map(|command| PaletteItem {
            id: command.id.as_str().to_string(),
            title: command.title.to_string(),
            subtitle: None,
            status: None,
            command: command.id,
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
        })
        .collect();

    items.extend(recent_projects.iter().map(|project| PaletteItem {
        id: project.path.display().to_string(),
        title: project.title.clone(),
        subtitle: Some(project.path.display().to_string()),
        status: Some("recent".to_string()),
        command: CommandId::ProjectOpenRecent,
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
                    TabStartState::Lazy => "lazy".to_string(),
                    TabStartState::Started => "started".to_string(),
                });

                PaletteItem {
                    id: tab.id.clone(),
                    title: tab.title.clone(),
                    subtitle: Some(format!("{pane_count} panes")),
                    status,
                    command: CommandId::TabPalette,
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
            .map(|pane| PaletteItem {
                id: pane.pane_id.clone(),
                title: tab
                    .layout
                    .find_pane(&pane.pane_id)
                    .map(|pane_config| pane_config.title.clone())
                    .unwrap_or_else(|| pane.pane_id.clone()),
                subtitle: Some(tab.title.clone()),
                status: Some(process_status_label(pane.process_state).to_string()),
                command: CommandId::PanePalette,
            })
            .collect(),
    )
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
