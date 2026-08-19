use gpui::{Context, Window};
use yttt_agent_core::{AgentProcessState, AgentViewState};
use yttt_protocol::agent::AgentSnapshotUpdate;

use super::{WorkbenchView, helpers::terminal_pane_key};
use crate::{
    model::ids::ProjectId,
    runtime::agent_manager::{AgentPaneAddress, AgentPaneExitOutcome},
};

impl WorkbenchView {
    pub(super) fn sync_agent_process_monitoring(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.terminal.start_processes {
            return;
        }
        self.apply_pending_host_agent_snapshots(window, cx);
        if self.terminal.agent_process_monitor_task.is_some() {
            return;
        }
        let Some(client) = self.agent_manager.snapshot_client() else {
            return;
        };

        self.terminal.agent_process_monitor_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                while let Some(update) = client.recv().await {
                    if this
                        .update_in(cx, |view, window, cx| {
                            let mut changed = view.apply_host_agent_snapshot(update, window, cx);
                            if let Some(error) = view.agent_manager.take_error() {
                                view.load_error = Some(error);
                                changed = true;
                            }
                            if changed {
                                cx.notify();
                            }
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            }));
    }

    fn apply_pending_host_agent_snapshots(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let pending = std::mem::take(&mut self.terminal.pending_host_agent_snapshots);
        let mut changed = false;
        for update in pending.into_values() {
            changed |= self.apply_host_agent_snapshot(update, window, cx);
        }
        if let Some(error) = self.agent_manager.take_error() {
            self.load_error = Some(error);
            changed = true;
        }
        if changed {
            cx.notify();
        }
    }

    pub(super) fn apply_host_agent_snapshot(
        &mut self,
        update: AgentSnapshotUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        let Some(pane) = self
            .terminal
            .terminal_panes
            .get(update.terminal_session_id.as_str())
        else {
            self.terminal
                .pending_host_agent_snapshots
                .insert(update.terminal_session_id.clone(), update);
            return false;
        };
        let address: AgentPaneAddress = pane.read(cx).agent_pane_address();
        if address.project_id != update.scope.project_id {
            self.load_error = Some(format!(
                "Host Agent snapshot project mismatch for terminal {}",
                update.terminal_session_id.as_str()
            ));
            return true;
        }
        match self.agent_manager.apply_host_snapshot(address, update) {
            Some(AgentPaneExitOutcome::Snapshot { address, snapshot }) => {
                let result = if snapshot.process_state == AgentProcessState::Exited
                    && snapshot.view_state() == AgentViewState::Failed
                {
                    self.workspace.clear_agent_snapshot(
                        &ProjectId::new(&address.project_id),
                        &address.tab_id,
                        &address.pane_id,
                    )
                } else {
                    self.record_agent_event_snapshot(address, snapshot, window, cx)
                };
                if let Err(error) = result {
                    self.load_error = Some(error.to_string());
                }
                true
            }
            Some(AgentPaneExitOutcome::ResumeFailed { address }) => {
                if let Err(error) = self.workspace.clear_agent_snapshot(
                    &ProjectId::new(&address.project_id),
                    &address.tab_id,
                    &address.pane_id,
                ) {
                    self.load_error = Some(error.to_string());
                }
                let key = terminal_pane_key(&address.project_id, &address.tab_id, &address.pane_id);
                self.terminal.terminal_panes.remove(&key);
                self.terminal.terminal_pane_subscriptions.remove(&key);
                true
            }
            None => false,
        }
    }
}
