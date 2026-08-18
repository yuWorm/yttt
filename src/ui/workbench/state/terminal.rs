use std::{
    collections::{BTreeMap, HashMap},
    sync::{Arc, RwLock},
};

use gpui::{Entity, Subscription, Task};

use crate::{
    host_runtime::DesktopHostRuntime,
    model::ids::{ProjectId, TerminalSessionId},
    ui::{interaction::input_owner::TerminalInputGate, terminal::pane::TerminalPaneView},
};
use yttt_protocol::agent::AgentSnapshotUpdate;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct TerminalPaneTarget {
    pub(in super::super) project_id: ProjectId,
    pub(in super::super) tab_id: String,
    pub(in super::super) pane_id: String,
}

pub(in super::super) struct TerminalControllerState {
    pub(in super::super) start_processes: bool,
    pub(in super::super) terminal_input_gate: TerminalInputGate,
    pub(in super::super) environment: Arc<RwLock<BTreeMap<String, String>>>,
    pub(in super::super) pending_terminal_focus: Option<TerminalPaneTarget>,
    pub(in super::super) terminal_panes: HashMap<String, Entity<TerminalPaneView>>,
    pub(in super::super) terminal_pane_subscriptions: HashMap<String, Subscription>,
    pub(in super::super) host_runtime: Option<Arc<DesktopHostRuntime>>,
    pub(in super::super) agent_process_monitor_task: Option<Task<()>>,
    pub(in super::super) pending_host_agent_snapshots:
        HashMap<TerminalSessionId, AgentSnapshotUpdate>,
}

impl TerminalControllerState {
    pub(in super::super) fn new(environment: BTreeMap<String, String>) -> Self {
        Self {
            start_processes: true,
            terminal_input_gate: TerminalInputGate::default(),
            environment: Arc::new(RwLock::new(environment)),
            pending_terminal_focus: None,
            terminal_panes: HashMap::new(),
            terminal_pane_subscriptions: HashMap::new(),
            host_runtime: None,
            agent_process_monitor_task: None,
            pending_host_agent_snapshots: HashMap::new(),
        }
    }
}

impl Default for TerminalControllerState {
    fn default() -> Self {
        Self::new(BTreeMap::new())
    }
}
