use gpui::{Context, Window};
use yttt_agent_core::AgentProcessState;
use yttt_protocol::agent::AgentSnapshotUpdate;

use super::{WorkbenchView, helpers::terminal_pane_key};
use crate::{
    model::ids::ProjectId,
    runtime::{
        agent_hooks::AgentSnapshotEvent,
        agent_manager::{AgentPaneAddress, AgentPaneExitOutcome},
    },
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
        if self.terminal.host_agent_epoch.is_none() {
            self.terminal.host_agent_epoch = Some(host_epoch);
            return false;
        }
        if self
            .terminal
            .host_agent_epoch
            .is_some_and(|epoch| epoch >= host_epoch)
        {
            return false;
        }
        self.terminal.host_agent_epoch = Some(host_epoch);
        self.terminal.pending_host_agent_snapshots.clear();
        // Preserve resume metadata, but never present the previous Host's processes as live.
        let snapshots = self.agent_manager.retained_snapshots();
        for (address, snapshot) in &snapshots {
            if let Err(error) =
                self.record_agent_runtime_snapshot(address.clone(), snapshot.clone())
            {
                self.load_error = Some(error.to_string());
            }
        }
        self.agent_manager.reset_for_host_restore(snapshots);
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
            self.load_error = Some(format!(
                "Host Agent snapshot project mismatch for terminal {}",
                update.terminal_session_id.as_str()
            ));
            return true;
        }
        match self.agent_manager.apply_host_snapshot(address, update) {
            Some(AgentPaneExitOutcome::Snapshot { address, snapshot }) => {
                let result = if snapshot.process_state == AgentProcessState::Exited {
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
            Some(AgentPaneExitOutcome::ResumeFailed { address, snapshot }) => {
                if let Err(error) = self.record_agent_runtime_snapshot(address, snapshot) {
                    self.load_error = Some(error.to_string());
                } else {
                    self.load_error = Some(
                        "Agent session could not be resumed; the original session is retained. Retry or explicitly open a new session.".into(),
                    );
                }
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::*;
    use super::*;
    use yttt_agent_core::{AgentReducer, AgentSessionMetadata};
    use yttt_protocol::agent::AgentHookScope;

    #[gpui::test]
    fn new_host_epoch_marks_retained_agents_stale_without_losing_resume_state(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        cx.skip_drawing();
        let root_dir = tempfile::tempdir().unwrap();
        let mut workspace = Workspace::new();
        let project = workspace
            .open_project(
                ProjectDescriptor::new(
                    ProjectId::new("epoch-project"),
                    ProjectLocation::local(root_dir.path().to_path_buf()),
                ),
                dev_fixture_layout(),
            )
            .unwrap();
        let mut snapshot =
            AgentReducer::new(AgentInstanceId::random(), ProviderId::from_static("omp"), 1)
                .snapshot()
                .clone();
        snapshot.process_state = AgentProcessState::Running;
        snapshot.turn_state = AgentTurnState::Working;
        snapshot.session = Some(AgentSessionMetadata {
            session_id: Some("retained-session".into()),
            ..Default::default()
        });
        workspace
            .record_agent_snapshot(&project, "dev", "shell", snapshot.clone())
            .unwrap();
        let update = AgentSnapshotUpdate {
            scope: AgentHookScope {
                project_id: project.as_str().into(),
                tab_id: "dev".into(),
                pane_id: "shell".into(),
                generation: 1,
            },
            terminal_session_id: TerminalSessionId::new(terminal_pane_key(
                project.as_str(),
                "dev",
                "shell",
            )),
            host_epoch: 1,
            sequence: 20,
            snapshot,
        };
        let paths = AppConfigPaths::from_config_dir(root_dir.path().join("config"));
        let (view, cx) = cx.add_window_view(|_, _| {
            WorkbenchView::with_workspace_for_test_and_config_paths(workspace, paths)
        });
        view.update_in(cx, |view, window, cx| {
            let address = AgentPaneAddress::new(project.as_str(), "dev", "shell");
            view.agent_manager
                .apply_host_snapshot(address, update.clone())
                .unwrap();
            view.terminal.host_agent_epoch = Some(1);
            view.terminal
                .pending_host_agent_snapshots
                .insert(update.terminal_session_id.clone(), update.clone());
            assert!(view.reset_host_agent_epoch(2));
            assert!(view.terminal.pending_host_agent_snapshots.is_empty());
            let saved = view.workspace.opened_projects()[0]
                .tab_states
                .iter()
                .find(|tab| tab.tab_id == "dev")
                .unwrap()
                .pane_states
                .iter()
                .find(|pane| pane.pane_id == "shell")
                .unwrap()
                .agent_snapshot
                .as_ref()
                .unwrap();
            assert_eq!(saved.view_state(), AgentViewState::Stale);
            assert_eq!(
                saved.session.as_ref().unwrap().session_id.as_deref(),
                Some("retained-session")
            );
            assert!(!view.apply_host_agent_snapshot(update, window, cx));
            assert!(view.terminal.pending_host_agent_snapshots.is_empty());
            assert!(!view.reset_host_agent_epoch(2));
        });
    }
}
