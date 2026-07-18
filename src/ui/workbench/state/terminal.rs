use std::collections::HashMap;

use gpui::{Entity, Subscription};

use crate::{
    model::ids::ProjectId,
    ui::{interaction::input_owner::TerminalInputGate, terminal::pane::TerminalPaneView},
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in super::super) struct TerminalPaneTarget {
    pub(in super::super) project_id: ProjectId,
    pub(in super::super) tab_id: String,
    pub(in super::super) pane_id: String,
}

pub(in super::super) struct TerminalControllerState {
    pub(in super::super) start_processes: bool,
    pub(in super::super) terminal_input_gate: TerminalInputGate,
    pub(in super::super) pending_terminal_focus: Option<TerminalPaneTarget>,
    pub(in super::super) terminal_panes: HashMap<String, Entity<TerminalPaneView>>,
    pub(in super::super) terminal_pane_subscriptions: HashMap<String, Subscription>,
}

impl Default for TerminalControllerState {
    fn default() -> Self {
        Self {
            start_processes: true,
            terminal_input_gate: TerminalInputGate::default(),
            pending_terminal_focus: None,
            terminal_panes: HashMap::new(),
            terminal_pane_subscriptions: HashMap::new(),
        }
    }
}
