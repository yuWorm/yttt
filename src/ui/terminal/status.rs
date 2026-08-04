use yttt_agent_core::AgentViewState;

use crate::{
    model::{
        layout::PaneConfig,
        workspace::{OpenedProject, PaneProcessState, PaneState},
    },
    runtime::agent::classify_agent,
};

pub fn agent_status_label(status: AgentViewState) -> &'static str {
    status.label()
}

pub fn project_agent_status(project: &OpenedProject) -> Option<AgentViewState> {
    project
        .layout
        .tabs
        .iter()
        .filter_map(|tab| tab_agent_status(project, &tab.id))
        .fold(None, merge_agent_status)
}

pub fn tab_agent_status(project: &OpenedProject, tab_id: &str) -> Option<AgentViewState> {
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

pub fn pane_agent_status(
    pane_config: &PaneConfig,
    pane_state: &PaneState,
) -> Option<AgentViewState> {
    if let Some(snapshot) = &pane_state.agent_snapshot {
        return Some(snapshot.view_state());
    }
    if !is_agent_pane(pane_config) {
        return None;
    }

    match pane_state.process_state {
        PaneProcessState::Running => Some(AgentViewState::Working),
        PaneProcessState::Idle | PaneProcessState::Exited => None,
    }
}

pub fn is_agent_pane(pane_config: &PaneConfig) -> bool {
    classify_agent(Some(pane_config.kind.clone()), &pane_config.command).is_agent()
}

fn merge_agent_status(
    current: Option<AgentViewState>,
    candidate: AgentViewState,
) -> Option<AgentViewState> {
    match current {
        Some(current) if current.priority() >= candidate.priority() => Some(current),
        _ => Some(candidate),
    }
}
