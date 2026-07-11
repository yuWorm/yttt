use crate::{
    model::{
        layout::PaneConfig,
        workspace::{AgentStatus, OpenedProject, PaneProcessState, PaneState},
    },
    runtime::agent::classify_agent,
};

pub fn agent_status_label(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Running => "agent running",
        AgentStatus::Completed => "agent completed",
        AgentStatus::Failed => "agent failed",
    }
}

pub fn project_agent_status(project: &OpenedProject) -> Option<AgentStatus> {
    project
        .layout
        .tabs
        .iter()
        .filter_map(|tab| tab_agent_status(project, &tab.id))
        .fold(None, merge_agent_status)
}

pub fn tab_agent_status(project: &OpenedProject, tab_id: &str) -> Option<AgentStatus> {
    let tab_config = project.layout.tabs.iter().find(|tab| tab.id == tab_id)?;
    let tab_state = project.tab_state(tab_id)?;

    tab_state
        .pane_states
        .iter()
        .filter_map(|pane_state| {
            let pane_config = tab_config.layout.find_pane(&pane_state.pane_id)?;
            pane_agent_status(pane_config, pane_state)
        })
        .fold(None, merge_agent_status)
}

pub fn pane_agent_status(pane_config: &PaneConfig, pane_state: &PaneState) -> Option<AgentStatus> {
    if !is_agent_pane(pane_config) {
        return None;
    }

    pane_state.agent_status.or(match pane_state.process_state {
        PaneProcessState::Running => Some(AgentStatus::Running),
        PaneProcessState::Idle | PaneProcessState::Exited => None,
    })
}

pub fn is_agent_pane(pane_config: &PaneConfig) -> bool {
    classify_agent(Some(pane_config.kind.clone()), &pane_config.command).is_agent()
}

fn merge_agent_status(current: Option<AgentStatus>, candidate: AgentStatus) -> Option<AgentStatus> {
    match current {
        Some(current) if agent_status_priority(current) >= agent_status_priority(candidate) => {
            Some(current)
        }
        _ => Some(candidate),
    }
}

fn agent_status_priority(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Running => 1,
        AgentStatus::Completed => 2,
        AgentStatus::Failed => 3,
    }
}
