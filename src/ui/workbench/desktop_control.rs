use super::*;
use crate::model::layout::PaneKind;
use yttt_protocol::desktop_control::*;

fn control_error(code: DesktopControlErrorCode, message: impl Into<String>) -> DesktopControlError {
    DesktopControlError::new(code, message)
}

fn failed(error: impl std::fmt::Display) -> DesktopControlError {
    control_error(DesktopControlErrorCode::Failed, error.to_string())
}

impl WorkbenchView {
    pub(crate) fn control_window(&self, id: String) -> ControlWindow {
        ControlWindow {
            id,
            selected_project: self
                .workspace
                .selected_project_id()
                .map(ToString::to_string),
            writable: self.shared_mutation_allowed(),
            loading: self.workspace_is_loading(),
        }
    }

    pub(crate) fn control_matches(&self, request: &DesktopControlRequest) -> bool {
        request
            .project
            .as_ref()
            .is_none_or(|id| self.workspace.project(&ProjectId::new(id)).is_some())
    }

    pub(crate) fn control_list(
        &self,
        window_id: &str,
        request: &DesktopControlRequest,
        cx: &App,
    ) -> DesktopControlResponse {
        if matches!(request.command, DesktopControlCommand::Windows) {
            return DesktopControlResponse::Windows(vec![self.control_window(window_id.into())]);
        }
        let mut projects = Vec::new();
        let mut tabs = Vec::new();
        let mut panes = Vec::new();
        let host_agents = self
            .terminal
            .host_runtime
            .as_ref()
            .filter(|_| {
                matches!(
                    request.command,
                    DesktopControlCommand::Panes | DesktopControlCommand::Agents
                )
            })
            .map(|runtime| runtime.agent_snapshots())
            .unwrap_or_default();
        for project in self.workspace.opened_projects() {
            if request
                .project
                .as_ref()
                .is_some_and(|id| id != project.id.as_str())
            {
                continue;
            }
            projects.push(ControlProject {
                window: window_id.into(),
                id: project.id.to_string(),
                name: project.layout.project.name.clone(),
                path: project.location.display_path(),
                selected: self.workspace.selected_project_id() == Some(&project.id),
            });
            if matches!(request.command, DesktopControlCommand::Projects) {
                continue;
            }
            for tab in &project.layout.tabs {
                if request.tab.as_ref().is_some_and(|id| id != &tab.id) {
                    continue;
                }
                let Some(state) = project.tab_state(&tab.id) else {
                    continue;
                };
                let target = ControlTarget {
                    window: window_id.into(),
                    project: project.id.to_string(),
                    tab: tab.id.clone(),
                    pane: None,
                };
                tabs.push(ControlTab {
                    target: target.clone(),
                    title: tab.title.clone(),
                    selected: project.selected_tab_id == tab.id,
                    panes: state.pane_states.len(),
                });
                if matches!(request.command, DesktopControlCommand::Tabs) {
                    continue;
                }
                for pane in &state.pane_states {
                    if request.pane.as_ref().is_some_and(|id| id != &pane.pane_id) {
                        continue;
                    }
                    let Some(config) = tab.layout.find_pane(&pane.pane_id) else {
                        continue;
                    };
                    let session_id = terminal_pane_key(project.id.as_str(), &tab.id, &pane.pane_id);
                    let agent = host_agents
                        .iter()
                        .find(|update| update.terminal_session_id.as_str() == session_id)
                        .map(|update| update.snapshot.clone())
                        .or_else(|| pane.agent_snapshot.clone());
                    if matches!(request.command, DesktopControlCommand::Agents)
                        && agent.is_none()
                        && config.kind != PaneKind::Agent
                    {
                        continue;
                    }
                    let runtime_state = self
                        .terminal
                        .terminal_panes
                        .get(&session_id)
                        .map(|view| view.read(cx).control_state());
                    panes.push(ControlPane {
                        target: ControlTarget {
                            pane: Some(pane.pane_id.clone()),
                            ..target.clone()
                        },
                        title: config.title.clone(),
                        command: config.command.clone(),
                        session_id,
                        state: runtime_state.unwrap_or_else(|| {
                            match pane.process_state {
                                crate::model::workspace::PaneProcessState::Idle => "idle",
                                crate::model::workspace::PaneProcessState::Restoring => "restoring",
                                crate::model::workspace::PaneProcessState::Running => "running",
                                crate::model::workspace::PaneProcessState::Exited => "exited",
                            }
                            .into()
                        }),
                        focused: state.focused_pane_id.as_ref() == Some(&pane.pane_id),
                        agent,
                    });
                }
            }
        }
        match request.command {
            DesktopControlCommand::Windows => {
                DesktopControlResponse::Windows(vec![self.control_window(window_id.into())])
            }
            DesktopControlCommand::Projects => DesktopControlResponse::Projects(projects),
            DesktopControlCommand::Tabs => DesktopControlResponse::Tabs(tabs),
            DesktopControlCommand::Agents => DesktopControlResponse::Agents(panes),
            _ => DesktopControlResponse::Panes(panes),
        }
    }

    fn control_target(
        &self,
        request: &DesktopControlRequest,
        window_id: &str,
    ) -> Result<ControlTarget, DesktopControlError> {
        let project_id = request.project.as_ref().ok_or_else(|| {
            control_error(
                DesktopControlErrorCode::InvalidRequest,
                "--project is required",
            )
        })?;
        let project = self
            .workspace
            .project(&ProjectId::new(project_id))
            .ok_or_else(|| {
                control_error(
                    DesktopControlErrorCode::NotFound,
                    "Project is not open in this window",
                )
            })?;
        if let Some(tab_id) = &request.tab {
            let tab = project.layout.tab(tab_id).ok_or_else(|| {
                control_error(
                    DesktopControlErrorCode::NotFound,
                    "Terminal tab does not exist",
                )
            })?;
            if let Some(pane_id) = &request.pane
                && tab.layout.find_pane(pane_id).is_none()
            {
                return Err(control_error(
                    DesktopControlErrorCode::NotFound,
                    "Pane does not exist in the specified tab",
                ));
            }
        }
        Ok(ControlTarget {
            window: window_id.into(),
            project: project_id.clone(),
            tab: request.tab.clone().unwrap_or_default(),
            pane: request.pane.clone(),
        })
    }

    pub(crate) fn handle_control(
        &mut self,
        window_id: &str,
        request: DesktopControlRequest,
        reply: flume::Sender<DesktopControlResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let result = self.apply_control(window_id, &request, &reply, window, cx);
        if let Some(result) = result {
            let _ = reply.send(result);
        }
    }

    fn apply_control(
        &mut self,
        window_id: &str,
        request: &DesktopControlRequest,
        reply: &flume::Sender<DesktopControlResult>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Option<DesktopControlResult> {
        let result = (|| {
            request.validate()?;
            let mut target = self.control_target(request, window_id)?;
            if matches!(request.command, DesktopControlCommand::Read) {
                let key = terminal_pane_key(
                    &target.project,
                    &target.tab,
                    target.pane.as_deref().unwrap_or_default(),
                );
                let viewport = self
                    .terminal
                    .host_runtime
                    .as_ref()
                    .and_then(|runtime| runtime.terminal_snapshot(&TerminalSessionId::new(key)))
                    .ok_or_else(|| {
                        control_error(
                            DesktopControlErrorCode::NotReady,
                            "No live terminal viewport is available",
                        )
                    })?;
                let text = viewport
                    .rows
                    .iter()
                    .map(|row| {
                        let mut line = String::new();
                        let mut column = 0;
                        for span in &row.spans {
                            line.extend(std::iter::repeat_n(
                                ' ',
                                usize::from(span.start_column.saturating_sub(column)),
                            ));
                            line.push_str(&span.text);
                            column = span.start_column.saturating_add(span.width);
                        }
                        line.trim_end().to_string()
                    })
                    .collect::<Vec<_>>()
                    .join("\n");
                return Ok(Some(DesktopControlResponse::Text { target, text }));
            }
            if self.workspace_is_loading() {
                return Err(control_error(
                    DesktopControlErrorCode::NotReady,
                    "Workspace is still loading",
                ));
            }
            if !self.shared_mutation_allowed() {
                return Err(control_error(
                    DesktopControlErrorCode::PermissionDenied,
                    "This desktop does not hold workspace control",
                ));
            }
            if let DesktopControlCommand::Send {
                text,
                enter,
                raw,
                agent_only,
            } = &request.command
            {
                if *agent_only {
                    let mut query = request.clone();
                    query.command = DesktopControlCommand::Agents;
                    let DesktopControlResponse::Agents(agents) =
                        self.control_list(window_id, &query, cx)
                    else {
                        unreachable!()
                    };
                    let snapshot = agents.first().and_then(|pane| pane.agent.as_ref())
                        .ok_or_else(|| control_error(DesktopControlErrorCode::NotReady, "Agent has not reported a ready state; use panes send for explicit terminal interaction"))?;
                    if snapshot.process_state != yttt_agent_core::AgentProcessState::Running
                        || !matches!(
                            snapshot.turn_state,
                            yttt_agent_core::AgentTurnState::Idle
                                | yttt_agent_core::AgentTurnState::Completed
                                | yttt_agent_core::AgentTurnState::Failed
                                | yttt_agent_core::AgentTurnState::Interrupted
                        )
                    {
                        return Err(control_error(
                            DesktopControlErrorCode::Busy,
                            "Agent is not ready for a new instruction; inspect agents list",
                        ));
                    }
                }
                let key = terminal_pane_key(
                    &target.project,
                    &target.tab,
                    target.pane.as_deref().unwrap_or_default(),
                );
                let pane = self.terminal.terminal_panes.get(&key).ok_or_else(|| {
                    control_error(
                        DesktopControlErrorCode::NotReady,
                        "Terminal has not started",
                    )
                })?;
                let (response, bytes) = pane
                    .read(cx)
                    .control_input(text, *enter, *raw, cx)
                    .map_err(|message| control_error(DesktopControlErrorCode::NotReady, message))?;
                let reply = reply.clone();
                cx.spawn(async move |_, _| {
                    let result = match response.recv_async().await {
                        Ok(Ok(yttt_protocol::Response::TerminalInputAccepted { .. })) => {
                            Ok(DesktopControlResponse::InputAccepted { target, bytes })
                        }
                        Ok(Err(error)) => Err(control_error(
                            DesktopControlErrorCode::OutcomeUnknown,
                            error.to_string(),
                        )),
                        _ => Err(control_error(
                            DesktopControlErrorCode::OutcomeUnknown,
                            "Host input acknowledgement was lost; input will not be replayed",
                        )),
                    };
                    let _ = reply.send(result);
                })
                .detach();
                return Ok(None);
            }
            if !matches!(
                self.foreground_input_owner_kind(),
                InputOwnerKind::Workspace | InputOwnerKind::Editor
            ) {
                return Err(control_error(
                    DesktopControlErrorCode::Busy,
                    "Close the active dialog or palette before changing the workspace layout",
                ));
            }
            if let DesktopControlCommand::CreateAgent { provider, .. } = &request.command
                && crate::config::default_layout::BuiltinAgent::from_id(provider).is_none()
            {
                return Err(control_error(
                    DesktopControlErrorCode::InvalidRequest,
                    "Supported providers: codex, claude, grok, groky, opencode, pi, omp",
                ));
            }
            if matches!(request.command, DesktopControlCommand::Close) {
                let project = self
                    .workspace
                    .project(&ProjectId::new(&target.project))
                    .expect("validated target");
                let closing_tab = target.pane.is_none()
                    || project
                        .tab_state(&target.tab)
                        .is_some_and(|tab| tab.pane_states.len() == 1);
                if closing_tab && project.layout.tabs.len() <= 1 {
                    return Err(control_error(
                        DesktopControlErrorCode::InvalidRequest,
                        "Cannot close the last terminal tab; create another tab first",
                    ));
                }
            }
            self.select_project(&ProjectId::new(&target.project))
                .map_err(failed)?;
            if !target.tab.is_empty() {
                self.select_work_item(WorkItemId::Terminal(target.tab.clone()))
                    .map_err(failed)?;
            }
            if let Some(pane) = &target.pane {
                self.workspace.focus_pane(pane).map_err(failed)?;
            }
            let response = match &request.command {
                DesktopControlCommand::CreateShell { command, title } => {
                    target.tab = self
                        .workspace
                        .create_shell_tab_with_id(uuid::Uuid::new_v4().to_string(), command.clone())
                        .map_err(failed)?;
                    target.pane = Some("shell".into());
                    if let Some(title) = title {
                        self.workspace.rename_selected_tab(title).map_err(failed)?;
                    }
                    self.select_work_item(WorkItemId::Terminal(target.tab.clone()))
                        .map_err(failed)?;
                    DesktopControlResponse::Created {
                        state: self.control_start_pane(&target, window, cx),
                        target,
                    }
                }
                DesktopControlCommand::CreateAgent {
                    provider,
                    args,
                    title,
                } => {
                    let agent = crate::config::default_layout::BuiltinAgent::from_id(provider)
                        .expect("validated provider");
                    let program = if provider == "groky" {
                        "groky"
                    } else {
                        agent.command()
                    };
                    target.tab = self
                        .workspace
                        .create_agent_tab_with_id(
                            uuid::Uuid::new_v4().to_string(),
                            title.as_deref().unwrap_or(agent.display_name()),
                            program,
                            args.clone(),
                        )
                        .map_err(failed)?;
                    target.pane = Some("agent".into());
                    self.select_work_item(WorkItemId::Terminal(target.tab.clone()))
                        .map_err(failed)?;
                    DesktopControlResponse::Created {
                        state: self.control_start_pane(&target, window, cx),
                        target,
                    }
                }
                DesktopControlCommand::Split { direction, command } => {
                    target.pane = Some(
                        self.workspace
                            .split_focused_pane_with_id(
                                match direction {
                                    ControlSplitDirection::Horizontal => SplitDirection::Horizontal,
                                    ControlSplitDirection::Vertical => SplitDirection::Vertical,
                                },
                                uuid::Uuid::new_v4().to_string(),
                                command.clone(),
                            )
                            .map_err(failed)?,
                    );
                    self.reconcile_active_terminal_with_workspace()
                        .map_err(failed)?;
                    DesktopControlResponse::Created {
                        state: self.control_start_pane(&target, window, cx),
                        target,
                    }
                }
                DesktopControlCommand::Focus => {
                    self.queue_selected_terminal_focus();
                    DesktopControlResponse::Updated { target }
                }
                DesktopControlCommand::Rename { title } => {
                    if target.pane.is_some() {
                        self.workspace.rename_focused_pane(title).map_err(failed)?;
                    } else {
                        self.workspace.rename_selected_tab(title).map_err(failed)?;
                    }
                    DesktopControlResponse::Updated { target }
                }
                DesktopControlCommand::Resize { direction, percent } => {
                    self.workspace
                        .resize_focused_split(
                            match direction {
                                ControlResizeDirection::Left => {
                                    crate::model::split_tree::ResizeDirection::Left
                                }
                                ControlResizeDirection::Right => {
                                    crate::model::split_tree::ResizeDirection::Right
                                }
                                ControlResizeDirection::Up => {
                                    crate::model::split_tree::ResizeDirection::Up
                                }
                                ControlResizeDirection::Down => {
                                    crate::model::split_tree::ResizeDirection::Down
                                }
                            },
                            f32::from(*percent) / 100.0,
                        )
                        .map_err(failed)?;
                    DesktopControlResponse::Updated { target }
                }
                DesktopControlCommand::Close => {
                    let only_pane = self
                        .workspace
                        .project(&ProjectId::new(&target.project))
                        .and_then(|project| project.tab_state(&target.tab))
                        .is_some_and(|tab| tab.pane_states.len() == 1);
                    if target.pane.is_none() || only_pane {
                        self.close_active_work_item().map_err(failed)?;
                    } else {
                        self.run_command(CommandId::PaneClose).map_err(failed)?;
                        let pane = target.pane.as_deref().expect("validated pane");
                        let key = terminal_pane_key(&target.project, &target.tab, pane);
                        self.terminal.terminal_panes.remove(&key);
                        self.terminal.terminal_pane_subscriptions.remove(&key);
                        self.agent_manager.forget_pane(&AgentPaneAddress::new(
                            &target.project,
                            &target.tab,
                            pane,
                        ));
                    }
                    DesktopControlResponse::Closed { target }
                }
                _ => {
                    return Err(control_error(
                        DesktopControlErrorCode::InvalidRequest,
                        "Unsupported mutation",
                    ));
                }
            };
            cx.notify();
            Ok(Some(response))
        })();
        match result {
            Ok(Some(response)) => Some(Ok(response)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }

    fn control_start_pane(
        &mut self,
        target: &ControlTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> String {
        let Some(context) = self
            .visible_terminal_pane_contexts()
            .into_iter()
            .find(|context| Some(&context.pane.id) == target.pane.as_ref())
        else {
            return "pending".into();
        };
        if self.agent_launch_waits_for_initialization(&context) {
            return "initializing".into();
        }
        self.ensure_terminal_pane(context, window, cx)
            .map(|pane| pane.read(cx).control_state())
            .unwrap_or_else(|| "pending".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(project: &str, command: DesktopControlCommand) -> DesktopControlRequest {
        DesktopControlRequest {
            window: None,
            project: Some(project.into()),
            tab: None,
            pane: None,
            command,
        }
    }

    #[gpui::test]
    fn cli_control_creates_splits_renames_and_closes_the_explicit_target(
        cx: &mut gpui::TestAppContext,
    ) {
        cx.update(gpui_component::init);
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let (root, cx) =
            cx.add_window_view(|_, _| WorkbenchView::with_config_paths_for_test(paths));
        root.update_in(cx, |root, _, _| {
            root.open_project_path(temp.path()).unwrap()
        });
        let project = root.read_with(cx, |root, _| {
            root.workspace.selected_project_id().unwrap().to_string()
        });
        let send = |request: DesktopControlRequest, cx: &mut gpui::VisualTestContext| {
            let (reply, result) = flume::bounded(1);
            root.update_in(cx, |root, window, cx| {
                root.handle_control("1", request, reply, window, cx)
            });
            result.try_recv().expect("synchronous layout result")
        };
        let created = send(
            request(
                &project,
                DesktopControlCommand::CreateShell {
                    command: "printf hello".into(),
                    title: Some("CLI shell".into()),
                },
            ),
            cx,
        )
        .unwrap();
        let DesktopControlResponse::Created { target, .. } = created else {
            panic!("expected creation")
        };
        assert!(uuid::Uuid::parse_str(&target.tab).is_ok());
        let mut split = request(
            &project,
            DesktopControlCommand::Split {
                direction: ControlSplitDirection::Vertical,
                command: "printf split".into(),
            },
        );
        split.tab = Some(target.tab.clone());
        split.pane = target.pane.clone();
        let DesktopControlResponse::Created { target: second, .. } = send(split, cx).unwrap()
        else {
            panic!("expected split")
        };
        assert!(uuid::Uuid::parse_str(second.pane.as_deref().unwrap()).is_ok());
        let mut rename = request(
            &project,
            DesktopControlCommand::Rename {
                title: "Worker".into(),
            },
        );
        rename.tab = Some(second.tab.clone());
        rename.pane = second.pane.clone();
        send(rename.clone(), cx).unwrap();
        root.read_with(cx, |root, app| {
            let mut query = request(&project, DesktopControlCommand::Panes);
            query.tab = Some(target.tab.clone());
            let DesktopControlResponse::Panes(panes) = root.control_list("1", &query, app) else {
                unreachable!()
            };
            assert_eq!(panes.len(), 2);
            assert!(
                panes
                    .iter()
                    .any(|pane| pane.title == "Worker" && pane.command == "printf split")
            );
            assert!(
                root.project
                    .project_editor_runtime
                    .workspace()
                    .session(&ProjectId::new(&project))
                    .unwrap()
                    .active_work_item()
                    .is_some_and(|item| item == &WorkItemId::Terminal(target.tab.clone()))
            );
        });
        rename.command = DesktopControlCommand::Close;
        send(rename, cx).unwrap();
        root.read_with(cx, |root, _| {
            let project = root.workspace.project(&ProjectId::new(&project)).unwrap();
            assert_eq!(project.tab_state(&target.tab).unwrap().pane_states.len(), 1);
            assert!(
                project
                    .layout
                    .tab(&target.tab)
                    .unwrap()
                    .layout
                    .find_pane(second.pane.as_deref().unwrap())
                    .is_none()
            );
        });
        let mut close = request(&project, DesktopControlCommand::Close);
        close.tab = Some(target.tab.clone());
        send(close, cx).unwrap();
        root.read_with(cx, |root, _| {
            assert!(
                root.workspace
                    .project(&ProjectId::new(&project))
                    .unwrap()
                    .layout
                    .tab(&target.tab)
                    .is_none()
            )
        });
    }

    #[gpui::test]
    fn cli_control_rejects_missing_target_before_changing_selection(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let (root, cx) =
            cx.add_window_view(|_, _| WorkbenchView::with_config_paths_for_test(paths));
        root.update_in(cx, |root, _, _| {
            root.open_project_path(temp.path()).unwrap()
        });
        let before = root.read_with(cx, |root, _| root.workspace.persisted_state());
        let (reply, response) = flume::bounded(1);
        let mut invalid = request(
            before.selected_project_id.as_ref().unwrap().as_str(),
            DesktopControlCommand::Close,
        );
        invalid.tab = Some("missing".into());
        root.update_in(cx, |root, window, cx| {
            root.handle_control("1", invalid, reply, window, cx)
        });
        assert_eq!(
            response.try_recv().unwrap().unwrap_err().code,
            DesktopControlErrorCode::NotFound
        );
        root.read_with(cx, |root, _| {
            assert_eq!(root.workspace.persisted_state(), before)
        });
    }
    #[cfg(unix)]
    #[gpui::test]
    fn cli_control_observer_can_list_but_cannot_mutate(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let host = crate::ui::terminal::pane::recovery_tests::RecoveryHost::start();
        let _owner = host.client("cli-owner");
        let observer = host.client("cli-observer");
        assert!(!observer.shared_editing_enabled());
        let paths = AppConfigPaths::from_config_dir(host.root.path().join("view-config"));
        let (root, cx) =
            cx.add_window_view(|_, _| WorkbenchView::with_config_paths_for_test(paths));
        root.update_in(cx, |root, _, _| {
            root.open_project_path(host.root.path()).unwrap()
        });
        let before = root.read_with(cx, |root, _| root.workspace.persisted_state());
        root.update_in(cx, |root, _, _| {
            root.terminal.host_runtime = Some(observer.clone())
        });
        let project = before.selected_project_id.as_ref().unwrap().as_str();
        root.read_with(cx, |root, app| {
            let DesktopControlResponse::Projects(projects) =
                root.control_list("1", &request(project, DesktopControlCommand::Projects), app)
            else {
                unreachable!()
            };
            assert_eq!(projects.len(), 1);
        });
        let (reply, response) = flume::bounded(1);
        root.update_in(cx, |root, window, cx| {
            root.handle_control(
                "1",
                request(
                    project,
                    DesktopControlCommand::CreateShell {
                        command: "must not run".into(),
                        title: None,
                    },
                ),
                reply,
                window,
                cx,
            )
        });
        assert_eq!(
            response.try_recv().unwrap().unwrap_err().code,
            DesktopControlErrorCode::PermissionDenied
        );
        root.read_with(cx, |root, _| {
            assert_eq!(root.workspace.persisted_state(), before)
        });
    }
    #[gpui::test]
    fn cli_control_preserves_a_pending_rename_dialog(cx: &mut gpui::TestAppContext) {
        cx.update(gpui_component::init);
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let (root, cx) =
            cx.add_window_view(|_, _| WorkbenchView::with_config_paths_for_test(paths));
        root.update_in(cx, |root, _, _| {
            root.open_project_path(temp.path()).unwrap();
            root.run_command(CommandId::TabRename).unwrap();
        });
        let before = root.read_with(cx, |root, _| root.workspace.persisted_state());
        let project = before.selected_project_id.as_ref().unwrap().as_str();
        let (reply, result) = flume::bounded(1);
        root.update_in(cx, |root, window, cx| {
            root.handle_control(
                "1",
                request(
                    project,
                    DesktopControlCommand::CreateShell {
                        command: String::new(),
                        title: None,
                    },
                ),
                reply,
                window,
                cx,
            )
        });
        assert_eq!(
            result.try_recv().unwrap().unwrap_err().code,
            DesktopControlErrorCode::Busy
        );
        root.read_with(cx, |root, _| {
            assert_eq!(root.workspace.persisted_state(), before);
            assert!(root.overlays.pending_tab_rename.is_some());
        });
    }
}
