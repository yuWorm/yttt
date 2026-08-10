use std::time::{SystemTime, UNIX_EPOCH};

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
        save_settings(&self.config_paths, &self.app_settings)?;
        if self.agent_sessions_enabled() {
            self.refresh_agent_sessions();
        }
        Ok(())
    }

    pub fn set_agent_sessions_enabled(&mut self, enabled: bool) -> Result<(), WorkbenchError> {
        self.app_settings.agent.sessions_enabled = enabled;
        save_settings(&self.config_paths, &self.app_settings)?;
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
        self.agent_sessions.generation = self.agent_sessions.generation.wrapping_add(1);
        self.agent_sessions.pending_scan = true;
        self.agent_sessions.loading = true;
        self.agent_sessions.key = Some(key);
        self.agent_sessions.sessions = Arc::new(Vec::new());
        self.agent_sessions.error = None;
    }

    pub(super) fn flush_pending_agent_session_scan(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.agent_sessions.pending_scan {
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
        let task = cx.background_spawn(async move {
            scan_agent_sessions(&agents, &project_path).map_err(|error| error.to_string())
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
                    Ok(sessions) => {
                        root.agent_sessions.sessions = Arc::new(sessions);
                        root.agent_sessions.error = None;
                    }
                    Err(error) => {
                        root.agent_sessions.sessions = Arc::new(Vec::new());
                        root.agent_sessions.error = Some(error);
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

    pub(super) fn agent_session_title(&self, session: &AgentSession) -> String {
        if session.title.is_empty() {
            self.ui_text
                .get(UiTextKey::AgentSessionsUntitled)
                .to_string()
        } else {
            session.title.clone()
        }
    }

    pub(super) fn resume_agent_session(&mut self, index: usize) -> Result<(), String> {
        let session = self
            .agent_sessions
            .sessions
            .get(index)
            .cloned()
            .ok_or_else(|| "Agent session is no longer available".to_string())?;
        let title = self.agent_session_title(&session);
        let resume = self
            .agent_manager
            .resume_command(session.provider.id(), &session.metadata())
            .ok_or_else(|| {
                format!(
                    "{} does not provide a resume command for this session",
                    session.provider.display_name()
                )
            })?;
        let tab_id = self
            .workspace
            .create_agent_tab(title, resume.program, resume.arguments)
            .map_err(|error| error.to_string())?;
        self.select_work_item(WorkItemId::Terminal(tab_id))
            .map(|_| ())
            .map_err(|error| error.to_string())
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
        } else if seconds < 60 * 60 {
            format!("{}m", seconds / 60)
        } else if seconds < 24 * 60 * 60 {
            format!("{}h", seconds / (60 * 60))
        } else {
            format!("{}d", seconds / (24 * 60 * 60))
        }
    }
}
