use std::collections::HashMap;

use gpui::{Entity, Subscription};

use crate::ui::{interaction::input_owner::TerminalInputGate, terminal::pane::TerminalPaneView};

pub(in super::super) struct TerminalControllerState {
    pub(in super::super) start_processes: bool,
    pub(in super::super) terminal_input_gate: TerminalInputGate,
    pub(in super::super) pending_terminal_focus_pane_id: Option<String>,
    pub(in super::super) terminal_panes: HashMap<String, Entity<TerminalPaneView>>,
    pub(in super::super) terminal_pane_subscriptions: HashMap<String, Subscription>,
}

impl Default for TerminalControllerState {
    fn default() -> Self {
        Self {
            start_processes: true,
            terminal_input_gate: TerminalInputGate::default(),
            pending_terminal_focus_pane_id: None,
            terminal_panes: HashMap::new(),
            terminal_pane_subscriptions: HashMap::new(),
        }
    }
}
