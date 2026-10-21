use std::time::{Duration, SystemTime, UNIX_EPOCH};
use yttt_agent_providers::session_management::SessionOperation;

use super::*;

impl WorkbenchView {
    pub(super) fn primary_agent(&self) -> BuiltinAgent {
        self.app_settings
            .agent
            .primary
            .or_else(|| self.default_layout_state.template().primary_agent())
            .unwrap_or_default()
    }

    pub fn agent_sessions_enabled(&self) -> bool {
        self.app_settings.agent.sessions_enabled
    }

    pub fn agent_session_agents(&self) -> Vec<BuiltinAgent> {
        let primary = self.primary_agent();
        let mut agents = Vec::with_capacity(BuiltinAgent::ALL.len());
        agents.push(primary);
        agents.extend(BuiltinAgent::ALL.into_iter().filter(|agent| {
            *agent != primary
                && self
                    .app_settings
                    .agent
                    .additional_session_agents
                    .contains(agent)
        }));
        agents
    }

    pub fn agent_session_agent_enabled(&self, agent: BuiltinAgent) -> bool {
        agent == self.primary_agent()
            || self
                .app_settings
                .agent
                .additional_session_agents
                .contains(&agent)
    }

    pub fn set_agent_session_agent_enabled(
        &mut self,
        agent: BuiltinAgent,
        enabled: bool,
    ) -> Result<(), WorkbenchError> {
        let primary = self.primary_agent();
        self.app_settings
            .agent
            .additional_session_agents
            .retain(|candidate| *candidate != primary && *candidate != agent);
        if enabled && agent != primary {
            self.app_settings
                .agent
                .additional_session_agents
                .push(agent);
        }
        self.app_settings
            .agent
            .additional_session_agents
            .sort_by_key(|agent| {
                BuiltinAgent::ALL
                    .iter()
                    .position(|candidate| candidate == agent)
                    .unwrap_or(usize::MAX)
            });
        if !self.persist_app_settings(false)? {
            return Ok(());
        }
        if self.agent_sessions_enabled() {
            self.refresh_agent_sessions();
        }
        Ok(())
    }

    pub fn set_agent_sessions_enabled(&mut self, enabled: bool) -> Result<(), WorkbenchError> {
        self.app_settings.agent.sessions_enabled = enabled;
        if !self.persist_app_settings(false)? {
            return Ok(());
        }
        if !enabled {
            self.agent_sessions.clear();
            if self.project.active_panel_page == ProjectPanelPage::AgentSessions {
                self.project.active_panel_page = ProjectPanelPage::Files;
            }
        }
        Ok(())
    }

    pub(super) fn ensure_agent_session_scan_requested(&mut self) {
        if !self.agent_sessions_enabled() {
            return;
        }
        let Some(project_id) = self.workspace.selected_project_id().cloned() else {
            return;
        };
        let Some(project) = self.workspace.project(&project_id) else {
            return;
        };
        if project.location.local_path().is_none() {
            return;
        }
        let key = AgentSessionScanKey {
            project_id,
            agents: self.agent_session_agents(),
        };
        if self.agent_sessions.key.as_ref() != Some(&key) {
            self.request_agent_session_scan(key);
        }
    }

    pub(super) fn ensure_agent_session_refresh(&mut self, cx: &mut Context<Self>) {
        if !self.agent_sessions_enabled()
            || self.project.active_panel_page != ProjectPanelPage::AgentSessions
        {
            self.agent_sessions.refresh_task = None;
            return;
        }
        if self.agent_sessions.loading
            || self.agent_sessions.mutation.is_some()
            || self.agent_sessions.key.is_none()
            || self.agent_sessions.refresh_task.is_some()
        {
            return;
        }
        self.agent_sessions.refresh_task = Some(cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs(30))
                .await;
            let _ = this.update(cx, |root, cx| {
                root.agent_sessions.refresh_task = None;
                if !root.command_center_is_active()
                    && root.project.active_panel_page == ProjectPanelPage::AgentSessions
                {
                    root.refresh_agent_sessions();
                    cx.notify();
                }
            });
        }));
    }

    pub(super) fn refresh_agent_sessions(&mut self) {
        if !self.agent_sessions_enabled() {
            return;
        }
        let Some(project_id) = self.workspace.selected_project_id().cloned() else {
            return;
        };
        let Some(project) = self.workspace.project(&project_id) else {
            return;
        };
        if project.location.local_path().is_none() {
            return;
        }
        self.request_agent_session_scan(AgentSessionScanKey {
            project_id,
            agents: self.agent_session_agents(),
        });
    }

    fn request_agent_session_scan(&mut self, key: AgentSessionScanKey) {
        let changed_project_or_providers = self.agent_sessions.key.as_ref() != Some(&key);
        if !changed_project_or_providers && self.agent_sessions.loading {
            return;
        }
        self.agent_sessions.refresh_task = None;
        self.agent_sessions.generation = self.agent_sessions.generation.wrapping_add(1);
        self.agent_sessions.pending_scan = true;
        self.agent_sessions.loading = true;
        if changed_project_or_providers {
            self.agent_sessions.mutation_error = None;
            self.agent_sessions.expanded_providers.clear();
            if key.agents.len() > 1
                && let Some(primary_agent) = key.agents.first()
            {
                self.agent_sessions
                    .expanded_providers
                    .insert(primary_agent.id());
            }
            self.agent_sessions.sessions = Arc::new(Vec::new());
        }
        self.agent_sessions.key = Some(key);
        self.agent_sessions.error = None;
    }

    pub(super) fn flush_pending_agent_session_scan(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.agent_sessions.pending_scan || self.agent_sessions.mutation.is_some() {
            return;
        }
        self.agent_sessions.pending_scan = false;
        let Some(key) = self.agent_sessions.key.clone() else {
            return;
        };
        let Some(project_path) = self
            .workspace
            .project(&key.project_id)
            .and_then(|project| project.location.local_path())
            .cloned()
        else {
            self.agent_sessions.loading = false;
            return;
        };
        let generation = self.agent_sessions.generation;
        let agents = key.agents.clone();
        let agent_session_access = self.config_paths.agent_session_access().clone();
        let remote_host = self
            .terminal
            .host_runtime
            .clone()
            .filter(|runtime| runtime.is_remote());
        let task = cx.background_spawn(async move {
            match remote_host {
                Some(runtime) => crate::runtime::agent_sessions::scan_remote_agent_sessions(
                    &runtime,
                    &agents,
                    &project_path,
                ),
                None => scan_agent_sessions(&agents, &project_path, &agent_session_access),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, _window, cx| {
                if root.agent_sessions.generation != generation
                    || root.agent_sessions.key.as_ref() != Some(&key)
                    || !root.agent_sessions_enabled()
                {
                    return;
                }
                root.agent_sessions.loading = false;
                match result {
                    Ok(scan) => {
                        root.agent_sessions.sessions = Arc::new(scan.sessions);
                        root.agent_sessions.error =
                            (!scan.warnings.is_empty()).then(|| scan.warnings.join("\n"));
                    }
                    Err(error) => {
                        root.agent_sessions.error = Some(format!(
                            "{}: {error}",
                            root.ui_text
                                .get(UiTextKey::WorkbenchI18nAgentSessionsLoadError)
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }

    pub(super) fn agent_sessions_search_input(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<InputState> {
        if let Some(input) = &self.agent_sessions.search_input {
            return input.clone();
        }

        let input = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder(self.ui_text.get(UiTextKey::AgentSessionsSearchPlaceholder))
                .clean_on_escape()
        });
        let subscription =
            cx.subscribe_in(&input, window, Self::on_agent_sessions_search_input_event);
        self.agent_sessions.search_input = Some(input.clone());
        self.agent_sessions.search_input_subscription = Some(subscription);
        input
    }

    fn on_agent_sessions_search_input_event(
        &mut self,
        _input: &Entity<InputState>,
        event: &InputEvent,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if matches!(event, InputEvent::Change) {
            cx.notify();
        }
    }

    pub(super) fn open_agent_session_management(
        &mut self,
        provider: BuiltinAgent,
        session_id: &str,
        delete: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_sessions.mutation.is_some() || !self.require_shared_mutation_control() {
            return;
        }
        let Some(key) = self.agent_sessions.key.clone() else {
            return;
        };
        let Some(session) = self
            .agent_sessions
            .sessions
            .iter()
            .find(|session| session.provider == provider && session.id == session_id)
        else {
            return;
        };
        let name = self.agent_session_title(session);
        let input = (!delete)
            .then(|| cx.new(|cx| InputState::new(window, cx).default_value(session.title.clone())));
        let session_id = session_id.to_string();
        let identity = format!("{} · {}", provider.display_name(), session_id);
        let title = self
            .ui_text
            .get(if delete {
                UiTextKey::AgentSessionsDelete
            } else {
                UiTextKey::AgentSessionsRename
            })
            .to_string();
        let description = self
            .ui_text
            .get(if delete && provider == BuiltinAgent::Grok {
                UiTextKey::AgentSessionsGrokDeleteDescription
            } else if delete {
                UiTextKey::AgentSessionsDeleteDescription
            } else {
                UiTextKey::AgentSessionsRenameDescription
            })
            .to_string();
        let cancel = self.ui_text.get(UiTextKey::Cancel).to_string();
        let appearance = self.theme_runtime();
        let theme = appearance.ui;
        let style = appearance.style;
        let workbench = cx.weak_entity();
        window.open_alert_dialog(cx, move |alert, _, cx| {
            let input = input.clone();
            let workbench = workbench.clone();
            let key = key.clone();
            let session_id = session_id.clone();
            let body = div()
                .flex()
                .flex_col()
                .gap(style.spacing.md)
                .child(div().font_weight(FontWeight::SEMIBOLD).child(name.clone()))
                .child(div().text_xs().child(identity.clone()))
                .child(description.clone())
                .when_some(input.as_ref(), |body, input| {
                    body.child(yttt_dialog_input(input, theme, style))
                });
            alert.title(title.clone()).description(body).footer(
                DialogFooter::new()
                    .child(
                        yttt_button(
                            "agent-session-mutation-cancel",
                            cancel.clone(),
                            YtttButtonVariant::Secondary,
                            theme,
                            style,
                            cx,
                        )
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        yttt_button(
                            "agent-session-mutation-confirm",
                            title.clone(),
                            if delete {
                                YtttButtonVariant::Danger
                            } else {
                                YtttButtonVariant::Primary
                            },
                            theme,
                            style,
                            cx,
                        )
                        .debug_selector(|| "agent-session-mutation-confirm".to_string())
                        .on_click(move |_, window, cx| {
                            let operation = match &input {
                                Some(input) => SessionOperation::Rename {
                                    title: input.read(cx).value().to_string(),
                                },
                                None => SessionOperation::Delete,
                            };
                            let _ = workbench.update(cx, |root, cx| {
                                root.start_agent_session_mutation(
                                    key.clone(),
                                    provider,
                                    session_id.clone(),
                                    operation,
                                    window,
                                    cx,
                                );
                            });
                            window.close_dialog(cx);
                        }),
                    ),
            )
        });
    }

    fn start_agent_session_mutation(
        &mut self,
        key: AgentSessionScanKey,
        provider: BuiltinAgent,
        session_id: String,
        operation: SessionOperation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.agent_sessions.mutation.is_some() || !self.require_shared_mutation_control() {
            return;
        }
        if self.agent_sessions.key.as_ref() != Some(&key)
            || self.workspace.selected_project_id() != Some(&key.project_id)
        {
            self.load_error = Some(
                self.ui_text
                    .get(UiTextKey::AgentSessionsSelectionChanged)
                    .into(),
            );
            cx.notify();
            return;
        }
        let Some(project_path) = self
            .workspace
            .project(&key.project_id)
            .and_then(|project| project.location.local_path())
            .cloned()
        else {
            return;
        };
        // Fence any read started before this write; do not refresh until it settles.
        self.agent_sessions.generation = self.agent_sessions.generation.wrapping_add(1);
        self.agent_sessions.pending_scan = false;
        self.agent_sessions.loading = false;
        self.agent_sessions.refresh_task = None;
        self.agent_sessions.mutation = Some((provider, session_id.clone()));
        self.agent_sessions.mutation_error = None;
        let access = self.config_paths.agent_session_access().clone();
        let remote_host = self
            .terminal
            .host_runtime
            .clone()
            .filter(|runtime| runtime.is_remote());
        let task = cx.background_spawn(async move {
            match remote_host {
                Some(runtime) => crate::runtime::agent_sessions::mutate_remote_agent_session(
                    &runtime,
                    provider,
                    &session_id,
                    &project_path,
                    &operation,
                ),
                None => crate::runtime::agent_sessions::mutate_agent_session(
                    provider,
                    &session_id,
                    &project_path,
                    &access,
                    &operation,
                ),
            }
        });
        cx.spawn_in(window, async move |this, cx| {
            let result = task.await;
            let _ = this.update_in(cx, |root, _, cx| {
                root.agent_sessions.mutation = None;
                root.agent_sessions.loading = false;
                root.agent_sessions.pending_scan = false;
                if let Err(error) = result {
                    let error = format!(
                        "{}: {error}",
                        root.ui_text.get(UiTextKey::AgentSessionsMutationFailed)
                    );
                    if root.agent_sessions.key.as_ref() == Some(&key) {
                        root.agent_sessions.mutation_error = Some(error);
                    } else {
                        root.load_error = Some(error);
                    }
                }
                // Also reconcile failures: a transport timeout can follow a committed write.
                root.refresh_agent_sessions();
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn agent_session_title(&self, session: &AgentSession) -> String {
        if session.title.is_empty() {
            self.ui_text
                .get(UiTextKey::AgentSessionsUntitled)
                .to_string()
        } else {
            session.title.clone()
        }
    }

    pub(super) fn resume_agent_session(
        &mut self,
        provider: BuiltinAgent,
        session_id: &str,
    ) -> Result<(), String> {
        if self.agent_sessions.mutation.is_some() {
            return Err(self.ui_text.get(UiTextKey::AgentSessionsMutating).into());
        }
        let session = self
            .agent_sessions
            .sessions
            .iter()
            .find(|session| session.provider == provider && session.id == session_id)
            .ok_or_else(|| {
                self.ui_text
                    .get(UiTextKey::WorkbenchI18nAgentSessionUnavailable)
                    .to_string()
            })?;
        let title = self.agent_session_title(session);
        let resume = self
            .agent_manager
            .resume_command(session.provider.id(), &session.metadata())
            .ok_or_else(|| {
                self.ui_text
                    .get(UiTextKey::WorkbenchI18nAgentSessionProviderNoResumeCommand)
                    .replace("{provider}", session.provider.display_name())
            })?;
        let tab_id = self
            .workspace
            .create_agent_tab(title, resume.program, resume.arguments)
            .map_err(|error| {
                format!(
                    "{}: {error}",
                    self.ui_text
                        .get(UiTextKey::WorkbenchI18nAgentSessionResumeFailed)
                )
            })?;
        self.select_work_item(WorkItemId::Terminal(tab_id))
            .map(|_| ())
            .map_err(|error| {
                format!(
                    "{}: {error}",
                    self.ui_text
                        .get(UiTextKey::WorkbenchI18nAgentSessionResumeFailed)
                )
            })
    }

    pub(super) fn agent_session_age_label(&self, updated_at_ms: u64) -> String {
        if updated_at_ms == 0 {
            return String::new();
        }
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let seconds = now.saturating_sub(updated_at_ms) / 1_000;
        if seconds < 60 {
            self.ui_text
                .get(UiTextKey::AgentSessionsJustNow)
                .to_string()
        } else {
            let (key, count) = if seconds < 60 * 60 {
                (
                    UiTextKey::WorkbenchI18nAgentSessionsMinutesAgo,
                    seconds / 60,
                )
            } else if seconds < 24 * 60 * 60 {
                (
                    UiTextKey::WorkbenchI18nAgentSessionsHoursAgo,
                    seconds / (60 * 60),
                )
            } else {
                (
                    UiTextKey::WorkbenchI18nAgentSessionsDaysAgo,
                    seconds / (24 * 60 * 60),
                )
            };
            self.ui_text.get(key).replace("{count}", &count.to_string())
        }
    }
}
