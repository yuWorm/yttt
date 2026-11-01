use gpui::{Context, Window};
use yttt_agent_core::{AgentProcessState, AgentSnapshot, AgentViewState};
use yttt_protocol::agent::AgentSnapshotUpdate;

use super::{UiTextKey, WorkbenchView, helpers::terminal_pane_key};
use crate::{
    model::ids::{ProjectId, TerminalSessionId},
    runtime::{agent_hooks::AgentSnapshotEvent, agent_manager::AgentPaneAddress},
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
                while let Some(event) = client.recv().await {
                    if this
                        .update_in(cx, |view, window, cx| {
                            let changed = match event {
                                AgentSnapshotEvent::Update(update) => {
                                    view.apply_host_agent_snapshot(*update, window, cx)
                                }
                                AgentSnapshotEvent::Cleared(terminal_session_id) => {
                                    view.clear_host_agent_state(&terminal_session_id, cx)
                                }
                                AgentSnapshotEvent::Reset { host_epoch } => {
                                    view.reset_host_agent_epoch(host_epoch)
                                }
                            };
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
        if changed {
            cx.notify();
        }
    }

    fn reset_host_agent_epoch(&mut self, host_epoch: u64) -> bool {
        if self
            .terminal
            .host_agent_epoch
            .is_some_and(|epoch| epoch >= host_epoch)
        {
            return false;
        }
        let first = self.terminal.host_agent_epoch.is_none();
        self.terminal.host_agent_epoch = Some(host_epoch);
        if first {
            return false;
        }
        // The new Host run restarts the Agents and reports their snapshots
        // from the start; its layout carries the saved sessions.
        self.terminal.pending_host_agent_snapshots.clear();
        self.agent_manager.reset_host_sequences();
        false
    }

    /// Forgets the pane's Agent state and the session title it gave the pane.
    fn clear_host_agent_state(
        &mut self,
        terminal_session_id: &TerminalSessionId,
        cx: &mut Context<Self>,
    ) -> bool {
        self.terminal
            .pending_host_agent_snapshots
            .remove(terminal_session_id);
        let held = self.workspace.opened_projects().iter().find_map(|project| {
            project.tab_states.iter().find_map(|tab| {
                tab.pane_states.iter().find_map(|pane| {
                    let snapshot = pane.agent_snapshot.as_ref()?;
                    (terminal_pane_key(project.id.as_str(), &tab.tab_id, &pane.pane_id)
                        == terminal_session_id.as_str())
                    .then(|| {
                        (
                            AgentPaneAddress::new(project.id.as_str(), &tab.tab_id, &pane.pane_id),
                            snapshot.provider_id.clone(),
                        )
                    })
                })
            })
        });
        let Some((address, provider_id)) = held else {
            return false;
        };
        self.update_terminal_agent_title(&address, provider_id.as_str(), None, cx);
        if let Err(error) = self.workspace.clear_agent_snapshot(
            &ProjectId::new(&address.project_id),
            &address.tab_id,
            &address.pane_id,
        ) {
            self.load_error = Some(error.to_string());
        }
        true
    }

    fn expects_host_agent_snapshot(&self, terminal_session_id: &str) -> bool {
        self.workspace.opened_projects().iter().any(|project| {
            project.tab_states.iter().any(|tab| {
                tab.pane_states.iter().any(|pane| {
                    terminal_pane_key(project.id.as_str(), &tab.tab_id, &pane.pane_id)
                        == terminal_session_id
                })
            })
        })
    }

    pub(super) fn apply_host_agent_snapshot(
        &mut self,
        update: AgentSnapshotUpdate,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.terminal.host_runtime.as_ref().is_some_and(|runtime| {
            matches!(
                runtime.state(),
                yttt_client_core::ConnectionState::Ready { host_epoch, .. }
                    if host_epoch != update.host_epoch
            )
        }) || self
            .terminal
            .host_agent_epoch
            .is_some_and(|epoch| update.host_epoch < epoch)
        {
            return false;
        }
        self.reset_host_agent_epoch(update.host_epoch);
        let Some(pane) = self
            .terminal
            .terminal_panes
            .get(update.terminal_session_id.as_str())
        else {
            if self.expects_host_agent_snapshot(update.terminal_session_id.as_str()) {
                self.terminal
                    .pending_host_agent_snapshots
                    .insert(update.terminal_session_id.clone(), update);
            }
            return false;
        };
        let address: AgentPaneAddress = pane.read(cx).agent_pane_address();
        if address.project_id != update.scope.project_id {
            self.load_error = Some(
                self.ui_text
                    .get(UiTextKey::WorkbenchI18nAgentSnapshotProjectMismatch)
                    .replace("{terminal}", update.terminal_session_id.as_str()),
            );
            return true;
        }
        if !self.agent_manager.accept_host_snapshot(&address, &update) {
            return false;
        }
        // The Host merges the same way into the layout it broadcasts next.
        let mut snapshot = update.snapshot;
        let previous = self.pane_agent_snapshot(&address);
        if let Some(previous) = previous {
            snapshot.board.merge_retained(&previous.board);
        }
        let resume_failed = snapshot.session.is_none()
            && snapshot.view_state() == AgentViewState::Failed
            && previous.is_some_and(|previous| previous.session.is_some());
        if snapshot.session.is_none() {
            snapshot.session = previous.and_then(|previous| previous.session.clone());
        }
        // An ended Agent stays on its pane as history; ending is not announced.
        let result = if snapshot.process_state == AgentProcessState::Exited {
            self.record_agent_runtime_snapshot(address, snapshot)
        } else {
            self.record_agent_event_snapshot(address, snapshot, window, cx)
        };
        if let Err(error) = result {
            self.load_error = Some(format!(
                "{}: {error}",
                self.ui_text
                    .get(UiTextKey::WorkbenchI18nAgentSnapshotApplyFailed)
            ));
        } else if resume_failed {
            self.load_error = Some(
                self.ui_text
                    .get(UiTextKey::WorkbenchI18nAgentSessionResumeFailedRetained)
                    .to_string(),
            );
        }
        true
    }

    fn pane_agent_snapshot(&self, address: &AgentPaneAddress) -> Option<&AgentSnapshot> {
        self.workspace
            .project(&ProjectId::new(&address.project_id))?
            .tab_state(&address.tab_id)?
            .pane_states
            .iter()
            .find(|pane| pane.pane_id == address.pane_id)?
            .agent_snapshot
            .as_ref()
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use yttt_agent_core::AgentReducer;

    #[gpui::test]
    fn a_cleared_agent_state_leaves_its_pane(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        cx.skip_drawing();
        let root_dir = tempfile::tempdir().unwrap();
        let mut workspace = Workspace::new();
        let project = workspace
            .open_project(
                ProjectDescriptor::new(
                    ProjectId::new("cleared-project"),
                    ProjectLocation::local(root_dir.path().to_path_buf()),
                ),
                dev_fixture_layout(),
            )
            .unwrap();
        let snapshot = AgentReducer::new(
            AgentInstanceId::random(),
            ProviderId::from_static("claude"),
            1,
        )
        .snapshot()
        .clone();
        workspace
            .record_agent_snapshot(&project, "dev", "shell", snapshot)
            .unwrap();
        let paths = AppConfigPaths::from_config_dir(root_dir.path().join("config"));
        let (view, cx) = cx.add_window_view(|_, _| {
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths)
        });
        view.update(cx, |view, cx| {
            let session =
                TerminalSessionId::new(terminal_pane_key(project.as_str(), "dev", "shell"));
            assert!(view.clear_host_agent_state(&session, cx));
            assert!(!view.clear_host_agent_state(&session, cx));
            let pane = view.workspace.opened_projects()[0]
                .tab_states
                .iter()
                .find(|tab| tab.tab_id == "dev")
                .unwrap()
                .pane_states
                .iter()
                .find(|pane| pane.pane_id == "shell")
                .unwrap();
            assert!(pane.agent_snapshot.is_none());
        });
    }
}
