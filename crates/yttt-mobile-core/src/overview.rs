//! The home screen's data: projects, their tabs, the terminals the Host runs
//! and every Agent's state, without boards. Native code counts and sorts it.

use yttt_agent_core::{AgentViewState, WaitingReason};
use yttt_core::model::host_workspace::HostWorkspaceState;
use yttt_protocol::agent::AgentSnapshotUpdate;

#[derive(Clone, Debug, Default, PartialEq, uniffi::Record)]
pub struct Overview {
    pub projects: Vec<ProjectSummary>,
    pub agents: Vec<AgentSummary>,
    /// Terminals the Host runs, from its resource catalog.
    pub terminals: u32,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct ProjectSummary {
    pub project_id: String,
    pub name: String,
    pub tabs: Vec<TabSummary>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct TabSummary {
    pub tab_id: String,
    pub title: String,
    pub panes: u32,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct AgentSummary {
    /// `{project}:{tab}:{pane}`, the terminal to open for this Agent.
    pub session_id: String,
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
    pub provider: String,
    pub state: AgentState,
    pub waiting: Option<WaitingKind>,
    pub waiting_message: Option<String>,
    pub task: Option<String>,
    pub current_action: Option<String>,
    /// When the Agent entered `state`, in Unix milliseconds.
    pub state_started_at_ms: u64,
    pub updated_at_ms: u64,
}

/// What an Agent is doing, as the desktop shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum AgentState {
    Starting,
    Idle,
    Working,
    Waiting,
    Completed,
    Failed,
    Interrupted,
    Stale,
}

/// What a waiting Agent waits for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum WaitingKind {
    Approval,
    UserInput,
    External,
}

impl From<AgentViewState> for AgentState {
    fn from(state: AgentViewState) -> Self {
        match state {
            AgentViewState::Starting => Self::Starting,
            AgentViewState::Idle => Self::Idle,
            AgentViewState::Working => Self::Working,
            AgentViewState::Waiting => Self::Waiting,
            AgentViewState::Completed => Self::Completed,
            AgentViewState::Failed => Self::Failed,
            AgentViewState::Interrupted => Self::Interrupted,
            AgentViewState::Stale => Self::Stale,
        }
    }
}

impl From<WaitingReason> for WaitingKind {
    fn from(reason: WaitingReason) -> Self {
        match reason {
            WaitingReason::Approval => Self::Approval,
            WaitingReason::UserInput => Self::UserInput,
            WaitingReason::External => Self::External,
        }
    }
}

/// The overview from the open workspace's `layout` (none before it opened),
/// the Agent snapshots and the number of terminals, in the Host's order.
pub fn build(
    layout: Option<&HostWorkspaceState>,
    agents: &[AgentSnapshotUpdate],
    terminals: usize,
) -> Overview {
    let projects = layout
        .map(|layout| {
            layout
                .workspace
                .opened_projects
                .iter()
                .map(|project| ProjectSummary {
                    project_id: project.id.to_string(),
                    name: project.layout.project.name.clone(),
                    tabs: project
                        .layout
                        .tabs
                        .iter()
                        .map(|tab| TabSummary {
                            tab_id: tab.id.clone(),
                            title: tab.title.clone(),
                            panes: project
                                .tab_state(&tab.id)
                                .map_or(0, |state| state.pane_states.len() as u32),
                        })
                        .collect(),
                })
                .collect()
        })
        .unwrap_or_default();
    let agents = agents
        .iter()
        .map(|update| {
            let snapshot = &update.snapshot;
            let state = AgentState::from(snapshot.view_state());
            let (project_id, tab_id, pane_id) = locate(layout, update);
            AgentSummary {
                session_id: update.terminal_session_id.to_string(),
                project_id,
                tab_id,
                pane_id,
                provider: snapshot.provider_id.as_str().to_string(),
                state,
                waiting: (state == AgentState::Waiting)
                    .then_some(snapshot.waiting_reason)
                    .flatten()
                    .map(WaitingKind::from),
                waiting_message: snapshot.waiting_message.clone(),
                task: snapshot.task.as_ref().map(|task| task.title.clone()),
                current_action: snapshot.current_action.as_ref().map(|action| {
                    match &action.detail {
                        Some(detail) => format!("{} {detail}", action.name),
                        None => action.name.clone(),
                    }
                }),
                state_started_at_ms: snapshot.state_started_at,
                updated_at_ms: snapshot.updated_at,
            }
        })
        .collect();
    Overview {
        projects,
        agents,
        terminals: terminals as u32,
    }
}

/// The project, tab and pane an Agent runs in. The Host names a pane's
/// terminal `{project}:{tab}:{pane}`, as the desktop keys its panes; the
/// hook scope carries the project but not always the tab and pane (for a
/// pane the Host starts, both are the whole terminal name).
fn locate(
    layout: Option<&HostWorkspaceState>,
    update: &AgentSnapshotUpdate,
) -> (String, String, String) {
    let session = update.terminal_session_id.as_str();
    let in_layout = layout.and_then(|layout| {
        layout.workspace.opened_projects.iter().find_map(|project| {
            project.tab_states.iter().find_map(|tab| {
                tab.pane_states.iter().find_map(|pane| {
                    (session == format!("{}:{}:{}", project.id, tab.tab_id, pane.pane_id)).then(
                        || {
                            (
                                project.id.to_string(),
                                tab.tab_id.clone(),
                                pane.pane_id.clone(),
                            )
                        },
                    )
                })
            })
        })
    });
    let scope = &update.scope;
    in_layout
        .or_else(|| {
            let rest = session.strip_prefix(&format!("{}:", scope.project_id))?;
            let (tab, pane) = rest.split_once(':')?;
            Some((scope.project_id.clone(), tab.to_string(), pane.to_string()))
        })
        .unwrap_or_else(|| {
            (
                scope.project_id.clone(),
                scope.tab_id.clone(),
                scope.pane_id.clone(),
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_agent_core::{
        AgentAction, AgentInstanceId, AgentProcessState, AgentReducer, AgentTask, AgentTaskSource,
        AgentTurnState, ProviderId,
    };
    use yttt_core::model::{
        editor::ProjectEditorWorkspaceSnapshot,
        ids::{ProjectId, ProjectInstanceId, TerminalSessionId},
        layout::ProjectLayout,
        project::ProjectLocation,
        workspace::{
            OpenedProject, PaneProcessState, PaneState, TabStartState, TabState, WorkspaceState,
        },
    };
    use yttt_protocol::agent::AgentHookScope;

    fn layout() -> HostWorkspaceState {
        let project: ProjectLayout = toml::from_str(
            r#"
            [project]
            name = "Website"

            [[tabs]]
            id = "dev"
            title = "Dev server"
            layout = { type = "pane", id = "server", title = "Server", command = "npm run dev" }

            [[tabs]]
            id = "agents"
            title = "Agents"
            layout = { type = "pane", id = "claude", title = "Claude", command = "claude" }
            "#,
        )
        .unwrap();
        let pane = |id: &str| PaneState {
            pane_id: id.to_string(),
            process_state: PaneProcessState::Running,
            agent_snapshot: None,
        };
        HostWorkspaceState {
            revision: 1,
            workspace: WorkspaceState {
                opened_projects: vec![OpenedProject {
                    id: ProjectId::new("web"),
                    instance_id: ProjectInstanceId::random(),
                    location: ProjectLocation::local("/tmp/web"),
                    layout: project,
                    selected_tab_id: "dev".to_string(),
                    tab_states: vec![
                        TabState {
                            tab_id: "dev".to_string(),
                            start_state: TabStartState::Started,
                            focused_pane_id: Some("server".to_string()),
                            pane_states: vec![pane("server"), pane("logs")],
                        },
                        TabState {
                            tab_id: "agents".to_string(),
                            start_state: TabStartState::Started,
                            focused_pane_id: Some("claude".to_string()),
                            pane_states: vec![pane("claude")],
                        },
                    ],
                }],
                selected_project_id: Some(ProjectId::new("web")),
            },
            editor: ProjectEditorWorkspaceSnapshot::default(),
        }
    }

    fn waiting_agent(project: &str, tab: &str, pane: &str) -> AgentSnapshotUpdate {
        let mut snapshot = AgentReducer::new(
            AgentInstanceId::new("agent-1").unwrap(),
            ProviderId::new("claude").unwrap(),
            1_000,
        )
        .snapshot()
        .clone();
        snapshot.process_state = AgentProcessState::Running;
        snapshot.turn_state = AgentTurnState::Waiting;
        snapshot.waiting_reason = Some(WaitingReason::Approval);
        snapshot.waiting_message = Some("Run npm install?".to_string());
        snapshot.task = AgentTask::new("Add a login page", AgentTaskSource::UserPromptHook);
        snapshot.current_action = Some(AgentAction {
            id: None,
            name: "Bash".to_string(),
            detail: Some("npm install".to_string()),
        });
        snapshot.state_started_at = 2_000;
        snapshot.updated_at = 3_000;
        AgentSnapshotUpdate {
            // As the Host scopes a pane it starts: the tab and pane are the
            // whole terminal name.
            scope: AgentHookScope {
                project_id: project.to_string(),
                tab_id: format!("{project}:{tab}:{pane}"),
                pane_id: format!("{project}:{tab}:{pane}"),
                generation: 1,
            },
            terminal_session_id: TerminalSessionId::new(format!("{project}:{tab}:{pane}")),
            host_epoch: 1,
            sequence: 1,
            snapshot,
            board_omitted: true,
        }
    }

    #[test]
    fn projects_and_tabs_come_from_the_layout() {
        let overview = build(Some(&layout()), &[], 3);
        assert_eq!(overview.terminals, 3);
        assert_eq!(
            overview.projects,
            vec![ProjectSummary {
                project_id: "web".to_string(),
                name: "Website".to_string(),
                tabs: vec![
                    TabSummary {
                        tab_id: "dev".to_string(),
                        title: "Dev server".to_string(),
                        panes: 2,
                    },
                    TabSummary {
                        tab_id: "agents".to_string(),
                        title: "Agents".to_string(),
                        panes: 1,
                    },
                ],
            }]
        );
        assert!(overview.agents.is_empty());
    }

    #[test]
    fn an_agent_carries_its_state_task_and_where_it_runs() {
        let overview = build(
            Some(&layout()),
            &[waiting_agent("web", "agents", "claude")],
            1,
        );
        assert_eq!(
            overview.agents,
            vec![AgentSummary {
                session_id: "web:agents:claude".to_string(),
                project_id: "web".to_string(),
                tab_id: "agents".to_string(),
                pane_id: "claude".to_string(),
                provider: "claude".to_string(),
                state: AgentState::Waiting,
                waiting: Some(WaitingKind::Approval),
                waiting_message: Some("Run npm install?".to_string()),
                task: Some("Add a login page".to_string()),
                current_action: Some("Bash npm install".to_string()),
                state_started_at_ms: 2_000,
                updated_at_ms: 3_000,
            }]
        );
    }

    #[test]
    fn a_waiting_reason_counts_only_while_waiting() {
        let mut agent = waiting_agent("web", "agents", "claude");
        agent.snapshot.turn_state = AgentTurnState::Working;
        let overview = build(Some(&layout()), &[agent], 1);
        assert_eq!(overview.agents[0].state, AgentState::Working);
        assert_eq!(overview.agents[0].waiting, None);
    }

    #[test]
    fn agents_outside_the_open_workspace_or_before_it_opened_are_listed() {
        let elsewhere = waiting_agent("other", "tab", "pane");
        let overview = build(Some(&layout()), std::slice::from_ref(&elsewhere), 1);
        let agent = &overview.agents[0];
        assert_eq!(
            (
                agent.project_id.as_str(),
                agent.tab_id.as_str(),
                agent.pane_id.as_str()
            ),
            ("other", "tab", "pane")
        );
        let overview = build(None, &[elsewhere], 0);
        assert!(overview.projects.is_empty());
        assert_eq!(overview.agents.len(), 1);
    }
}
