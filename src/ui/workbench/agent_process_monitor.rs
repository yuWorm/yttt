use std::time::Duration;

use gpui::{Context, Window};
use yttt_agent_core::{AgentProcessState, AgentViewState};

use super::{WorkbenchView, helpers::terminal_pane_key};
use crate::{model::ids::ProjectId, runtime::agent_manager::AgentPaneExitOutcome};

const AGENT_SNAPSHOT_DRAIN_INTERVAL: Duration = Duration::from_millis(50);

impl WorkbenchView {
    pub(super) fn sync_agent_process_monitoring(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.terminal.agent_process_monitor_task.is_some() || !self.terminal.start_processes {
            return;
        }

        self.terminal.agent_process_monitor_task =
            Some(cx.spawn_in(window, async move |this, cx| {
                loop {
                    if this
                        .update_in(cx, |view, window, cx| {
                            view.apply_host_agent_snapshots(window, cx);
                        })
                        .is_err()
                    {
                        break;
                    }
                    cx.background_executor()
                        .timer(AGENT_SNAPSHOT_DRAIN_INTERVAL)
                        .await;
                }
            }));
    }

    fn apply_host_agent_snapshots(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let updates = self.agent_manager.drain_host_snapshots();
        let mut changed = false;
        for update in updates {
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
        update: yttt_protocol::agent::AgentSnapshotUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        match self.agent_manager.apply_host_snapshot(update) {
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
