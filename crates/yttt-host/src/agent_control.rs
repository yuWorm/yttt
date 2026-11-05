//! What an orchestrator Agent may do to other Agents. The hook listener has
//! already established who is asking; this module decides what that caller
//! may touch and changes the layout through the same intents Clients use.
//!
//! An orchestrator acts only on workers it started or that a person assigned
//! to it (`PaneConfig::spawned_by`). These rules keep an Agent from driving
//! panes a person did not hand it; they do not defend against other
//! processes of the same user.

use std::sync::Arc;

use yttt_agent_providers::builtin_providers;
use yttt_core::model::{
    host_workspace::{NewTab, PaneAddress, WorkspaceIntent},
    ids::TerminalSessionId,
    layout::PaneRole,
    workspace::PaneProcessState,
};
use yttt_protocol::{
    agent::{
        AGENT_CONTROL_CLOSE_PATH, AGENT_CONTROL_LIST_PATH, AGENT_CONTROL_PROVIDERS,
        AGENT_CONTROL_SPAWN_PATH, AgentControlError, AgentControlErrorCode, AgentControlIdentity,
        AgentControlResponse, AgentSpawnRequest, AgentTargetRequest, ControlledAgent,
        ControlledAgentState,
    },
    desktop_control::MAX_CONTROL_TEXT_BYTES,
    workspace::WorkspaceId,
};

use crate::{
    agent_hooks::AgentControlHandler,
    audit::{AuditEntry, AuditResult, HostAuditLog},
    pane_runtime::PaneRuntime,
    project::HostProjectRuntime,
    runtime::{HostRuntime, HostRuntimeError},
    workspace::{AppliedIntent, LocatedPane, WorkspaceService},
};

/// The pane ID an Agent tab's only pane receives.
const AGENT_PANE_ID: &str = "agent";
const MAX_TITLE_BYTES: usize = 256;

pub(crate) struct AgentControl {
    workspaces: Arc<WorkspaceService>,
    projects: Arc<HostProjectRuntime>,
    runtime: Arc<HostRuntime>,
    pane_runtime: Arc<PaneRuntime>,
    audit: Arc<HostAuditLog>,
    /// Requests arrive on the hook listener's threads; launching a pane may
    /// need the Host's async runtime.
    tokio: tokio::runtime::Handle,
}

impl AgentControl {
    pub(crate) fn new(
        workspaces: Arc<WorkspaceService>,
        projects: Arc<HostProjectRuntime>,
        runtime: Arc<HostRuntime>,
        pane_runtime: Arc<PaneRuntime>,
        audit: Arc<HostAuditLog>,
        tokio: tokio::runtime::Handle,
    ) -> Self {
        Self {
            workspaces,
            projects,
            runtime,
            pane_runtime,
            audit,
            tokio,
        }
    }

    fn spawn(
        &self,
        caller: &AgentControlIdentity,
        request: AgentSpawnRequest,
    ) -> Result<AgentControlResponse, AgentControlError> {
        let orchestrator = caller_pane(&self.workspaces, caller)?;
        let (intent, address) = spawn_intent(caller, &orchestrator, request)?;
        self.apply(&orchestrator.workspace_id, intent)?;
        let worker = self
            .workspaces
            .locate_pane(&address.terminal_session_id())
            .ok_or_else(|| failed("the new worker tab was not found after it was created"))?;
        Ok(AgentControlResponse::Spawned {
            agent: Box::new(controlled(&worker)),
        })
    }

    fn close(
        &self,
        caller: &AgentControlIdentity,
        request: AgentTargetRequest,
    ) -> Result<AgentControlResponse, AgentControlError> {
        let worker = owned_worker(&self.workspaces, caller, &request.agent)?;
        self.apply(
            &worker.workspace_id,
            close_intent(&self.workspaces, &worker)?,
        )?;
        Ok(AgentControlResponse::Closed {
            agent: request.agent,
        })
    }

    /// Applies an intent and settles its terminals the way a Client's
    /// `WorkspaceRequest::Apply` does.
    fn apply(
        &self,
        workspace_id: &WorkspaceId,
        intent: WorkspaceIntent,
    ) -> Result<AppliedIntent, AgentControlError> {
        let applied = self
            .workspaces
            .apply(workspace_id.clone(), intent, None)
            .map_err(|error| failed(error.message))?;
        for pane in &applied.removed_panes {
            let session_id = pane.terminal_session_id();
            match self.runtime.terminate(&session_id) {
                Ok(_) | Err(HostRuntimeError::NotFound(_)) => {}
                Err(error) => {
                    eprintln!("closed pane terminal {session_id} was not terminated: {error}");
                }
            }
        }
        self.projects
            .sync_workspaces(self.workspaces.referenced_projects());
        self.pane_runtime.ensure(workspace_id);
        Ok(applied)
    }
}

impl AgentControlHandler for AgentControl {
    fn handle(
        &self,
        caller: &AgentControlIdentity,
        path: &str,
        body: &[u8],
    ) -> Result<AgentControlResponse, AgentControlError> {
        let _runtime = self.tokio.enter();
        let (action, resource, result) = match path {
            AGENT_CONTROL_SPAWN_PATH => {
                let result = parse::<AgentSpawnRequest>(body)
                    .and_then(|request| self.spawn(caller, request));
                let resource = match &result {
                    Ok(AgentControlResponse::Spawned { agent }) => agent.agent.as_str().to_string(),
                    _ => String::new(),
                };
                ("agent_control.spawn", resource, result)
            }
            AGENT_CONTROL_LIST_PATH => (
                "agent_control.list",
                String::new(),
                Ok(list(&self.workspaces, caller)),
            ),
            AGENT_CONTROL_CLOSE_PATH => {
                let request = parse::<AgentTargetRequest>(body);
                let resource = request
                    .as_ref()
                    .map(|request| request.agent.as_str().to_string())
                    .unwrap_or_default();
                (
                    "agent_control.close",
                    resource,
                    request.and_then(|request| self.close(caller, request)),
                )
            }
            _ => return Err(not_found("unknown control request")),
        };
        self.audit.record(AuditEntry {
            request_id: 0,
            actor_client_id: format!("agent:{}", caller.terminal_session_id.as_str()),
            actor_device_id: None,
            action: action.to_string(),
            resource,
            result: if result.is_ok() {
                AuditResult::Allowed
            } else {
                AuditResult::Denied
            },
        });
        result
    }
}

/// Validates a spawn request and builds the intent that creates its worker
/// tab beside the orchestrator, with the address the worker will have.
fn spawn_intent(
    caller: &AgentControlIdentity,
    orchestrator: &LocatedPane,
    request: AgentSpawnRequest,
) -> Result<(WorkspaceIntent, PaneAddress), AgentControlError> {
    if !AGENT_CONTROL_PROVIDERS.contains(&request.provider.as_str()) {
        return Err(invalid(format!(
            "supported providers: {}",
            AGENT_CONTROL_PROVIDERS.join(", ")
        )));
    }
    if request.args.iter().map(String::len).sum::<usize>() > MAX_CONTROL_TEXT_BYTES
        || request.args.iter().any(|arg| arg.contains('\0'))
    {
        return Err(invalid("provider arguments are too long or contain NUL"));
    }
    let title = match request.title {
        Some(title) if title.trim().is_empty() || title.len() > MAX_TITLE_BYTES => {
            return Err(invalid("title must be 1 to 256 bytes"));
        }
        Some(title) => title,
        None => provider_title(&request.provider),
    };
    let tab_id = uuid::Uuid::new_v4().to_string();
    let address = PaneAddress {
        project_id: orchestrator.address.project_id.clone(),
        tab_id: tab_id.clone(),
        pane_id: AGENT_PANE_ID.to_string(),
    };
    let intent = WorkspaceIntent::CreateTab {
        project_id: orchestrator.address.project_id.clone(),
        tab: NewTab::Agent {
            title,
            command: request.provider,
            args: request.args,
            role: PaneRole::Worker,
            spawned_by: Some(caller.terminal_session_id.as_str().to_string()),
        },
        tab_id: Some(tab_id),
    };
    Ok((intent, address))
}

/// Closes a worker's pane, or its tab when it is the tab's only pane, as a
/// person closing it from the CLI would. A project's last terminal tab stays.
fn close_intent(
    workspaces: &WorkspaceService,
    worker: &LocatedPane,
) -> Result<WorkspaceIntent, AgentControlError> {
    let project =
        workspaces.panes_where(|address, _| address.project_id == worker.address.project_id);
    let in_workspace = |pane: &&LocatedPane| pane.workspace_id == worker.workspace_id;
    let tab_panes = project
        .iter()
        .filter(in_workspace)
        .filter(|pane| pane.address.tab_id == worker.address.tab_id)
        .count();
    if tab_panes > 1 {
        return Ok(WorkspaceIntent::ClosePane {
            project_id: worker.address.project_id.clone(),
            tab_id: worker.address.tab_id.clone(),
            pane_id: worker.address.pane_id.clone(),
        });
    }
    let tabs = project
        .iter()
        .filter(in_workspace)
        .map(|pane| pane.address.tab_id.as_str())
        .collect::<std::collections::HashSet<_>>();
    if tabs.len() <= 1 {
        return Err(failed(
            "the last terminal tab of a project cannot be closed; create another tab first",
        ));
    }
    Ok(WorkspaceIntent::CloseTabs {
        project_id: worker.address.project_id.clone(),
        tab_ids: vec![worker.address.tab_id.clone()],
    })
}

/// The caller's own pane, which must still be an orchestrator: a role
/// removed since its process started takes effect here.
fn caller_pane(
    workspaces: &WorkspaceService,
    caller: &AgentControlIdentity,
) -> Result<LocatedPane, AgentControlError> {
    let pane = workspaces
        .locate_pane(&caller.terminal_session_id)
        .ok_or_else(|| not_found("the orchestrator's pane is not in any workspace"))?;
    if pane.config.role != PaneRole::Orchestrator {
        return Err(forbidden("this pane is no longer an orchestrator"));
    }
    Ok(pane)
}

fn owned_worker(
    workspaces: &WorkspaceService,
    caller: &AgentControlIdentity,
    agent: &TerminalSessionId,
) -> Result<LocatedPane, AgentControlError> {
    caller_pane(workspaces, caller)?;
    let worker = workspaces
        .locate_pane(agent)
        .ok_or_else(|| not_found("no pane runs that Agent"))?;
    if worker.config.spawned_by.as_deref() != Some(caller.terminal_session_id.as_str()) {
        return Err(forbidden(
            "that Agent was neither started by nor assigned to this orchestrator",
        ));
    }
    Ok(worker)
}

fn list(workspaces: &WorkspaceService, caller: &AgentControlIdentity) -> AgentControlResponse {
    let owner = caller.terminal_session_id.as_str();
    AgentControlResponse::Agents {
        agents: workspaces
            .panes_where(|_, config| config.spawned_by.as_deref() == Some(owner))
            .iter()
            .map(controlled)
            .collect(),
    }
}

fn controlled(pane: &LocatedPane) -> ControlledAgent {
    ControlledAgent {
        agent: pane.address.terminal_session_id(),
        project_id: pane.address.project_id.to_string(),
        tab_id: pane.address.tab_id.clone(),
        pane_id: pane.address.pane_id.clone(),
        title: pane.config.title.clone(),
        command: pane.config.command.clone(),
        pane_state: match pane.state.process_state {
            PaneProcessState::Idle => "idle",
            PaneProcessState::Running => "running",
            PaneProcessState::Exited => "exited",
        }
        .to_string(),
        agent_state: pane
            .state
            .agent_snapshot
            .as_ref()
            .map(|snapshot| ControlledAgentState {
                provider: snapshot.provider_id.as_str().to_string(),
                process: snapshot.process_state,
                turn: snapshot.turn_state,
                waiting: snapshot.waiting_reason,
                task: snapshot.task.as_ref().map(|task| task.single_line_title()),
                updated_at: snapshot.updated_at,
            }),
    }
}

fn provider_title(provider: &str) -> String {
    builtin_providers()
        .into_iter()
        .map(|provider| provider.descriptor())
        .find(|descriptor| descriptor.id.as_str() == provider)
        .map_or_else(
            || provider.to_string(),
            |descriptor| descriptor.display_name.to_string(),
        )
}

fn parse<T: serde::de::DeserializeOwned>(body: &[u8]) -> Result<T, AgentControlError> {
    serde_json::from_slice(body).map_err(|error| invalid(error.to_string()))
}

fn invalid(message: impl Into<String>) -> AgentControlError {
    AgentControlError::new(AgentControlErrorCode::InvalidRequest, message)
}

fn forbidden(message: impl Into<String>) -> AgentControlError {
    AgentControlError::new(AgentControlErrorCode::Forbidden, message)
}

fn not_found(message: impl Into<String>) -> AgentControlError {
    AgentControlError::new(AgentControlErrorCode::NotFound, message)
}

fn failed(message: impl Into<String>) -> AgentControlError {
    AgentControlError::new(AgentControlErrorCode::Failed, message)
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;
    use yttt_core::model::{
        host_workspace::ProjectPanel,
        ids::ProjectId,
        layout::ProjectLayout,
        project::{ProjectDescriptor, ProjectLocation},
    };
    use yttt_protocol::workspace::WorkspaceProjectConfig;

    use super::*;

    fn workspace_id() -> WorkspaceId {
        WorkspaceId::new("default").unwrap()
    }

    /// A project with two orchestrator tabs and a plain shell tab.
    fn workspaces(root: &std::path::Path) -> WorkspaceService {
        let service =
            WorkspaceService::new(root, root.join("config"), WorkspaceProjectConfig::Project)
                .unwrap();
        let project_root = root.join("project");
        std::fs::create_dir_all(&project_root).unwrap();
        let layout: ProjectLayout = toml::from_str(
            r#"
            [project]
            name = "Orchestrated"

            [[tabs]]
            id = "lead"
            title = "Lead"
            layout = { type = "pane", id = "lead", title = "Lead", command = "claude", kind = "agent", role = "orchestrator" }

            [[tabs]]
            id = "rival"
            title = "Rival"
            layout = { type = "pane", id = "rival", title = "Rival", command = "claude", kind = "agent", role = "orchestrator" }

            [[tabs]]
            id = "plain"
            title = "Plain"
            layout = { type = "pane", id = "plain", title = "Plain", command = "$SHELL" }
            "#,
        )
        .unwrap();
        service
            .apply(
                workspace_id(),
                WorkspaceIntent::OpenProject {
                    descriptor: ProjectDescriptor::new(
                        ProjectId::new("project"),
                        ProjectLocation::local(&project_root),
                    ),
                    layout,
                    panel: ProjectPanel {
                        visible: true,
                        width: 240.0,
                    },
                },
                None,
            )
            .unwrap();
        service
    }

    fn caller(tab: &str) -> AgentControlIdentity {
        AgentControlIdentity {
            project_id: "project".into(),
            terminal_session_id: PaneAddress {
                project_id: ProjectId::new("project"),
                tab_id: tab.into(),
                pane_id: tab.into(),
            }
            .terminal_session_id(),
            generation: 1,
            role: PaneRole::Orchestrator,
        }
    }

    fn spawn_request(provider: &str, title: Option<&str>) -> AgentSpawnRequest {
        AgentSpawnRequest {
            provider: provider.into(),
            args: vec!["Review the parser".into()],
            title: title.map(str::to_owned),
        }
    }

    fn spawn(
        workspaces: &WorkspaceService,
        caller: &AgentControlIdentity,
        request: AgentSpawnRequest,
    ) -> Result<LocatedPane, AgentControlError> {
        let orchestrator = caller_pane(workspaces, caller)?;
        let (intent, address) = spawn_intent(caller, &orchestrator, request)?;
        workspaces
            .apply(orchestrator.workspace_id.clone(), intent, None)
            .unwrap();
        Ok(workspaces
            .locate_pane(&address.terminal_session_id())
            .unwrap())
    }

    fn code<T: std::fmt::Debug>(result: Result<T, AgentControlError>) -> AgentControlErrorCode {
        result.unwrap_err().code
    }

    #[test]
    fn a_spawned_worker_belongs_to_the_orchestrator_that_started_it() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let lead = caller("lead");

        let worker = spawn(&workspaces, &lead, spawn_request("codex", None)).unwrap();
        assert_eq!(worker.address.project_id, ProjectId::new("project"));
        assert_eq!(worker.config.command, "codex");
        assert_eq!(worker.config.args, ["Review the parser"]);
        assert_eq!(worker.config.role, PaneRole::Worker);
        assert_eq!(
            worker.config.spawned_by.as_deref(),
            Some(lead.terminal_session_id.as_str())
        );

        let AgentControlResponse::Agents { agents } = list(&workspaces, &lead) else {
            panic!("list answers with agents");
        };
        assert_eq!(agents.len(), 1);
        assert_eq!(agents[0].agent, worker.address.terminal_session_id());
        assert_eq!(agents[0].title, provider_title("codex"));
        let AgentControlResponse::Agents { agents } = list(&workspaces, &caller("rival")) else {
            panic!("list answers with agents");
        };
        assert!(agents.is_empty());
    }

    #[test]
    fn an_orchestrator_cannot_touch_panes_it_was_not_given() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let worker = spawn(&workspaces, &caller("lead"), spawn_request("codex", None))
            .unwrap()
            .address
            .terminal_session_id();
        let plain = caller("plain").terminal_session_id;

        assert!(owned_worker(&workspaces, &caller("lead"), &worker).is_ok());
        assert_eq!(
            code(owned_worker(&workspaces, &caller("rival"), &worker)),
            AgentControlErrorCode::Forbidden
        );
        assert_eq!(
            code(owned_worker(&workspaces, &caller("lead"), &plain)),
            AgentControlErrorCode::Forbidden
        );
        assert_eq!(
            code(owned_worker(
                &workspaces,
                &caller("lead"),
                &TerminalSessionId::new("project:gone:gone")
            )),
            AgentControlErrorCode::NotFound
        );
        // A pane whose role was taken away cannot act, whatever its credential.
        assert_eq!(
            code(spawn(
                &workspaces,
                &caller("plain"),
                spawn_request("codex", None)
            )),
            AgentControlErrorCode::Forbidden
        );
    }

    #[test]
    fn closing_a_worker_closes_its_tab_but_never_a_projects_last_one() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let worker = spawn(&workspaces, &caller("lead"), spawn_request("codex", None)).unwrap();

        let intent = close_intent(&workspaces, &worker).unwrap();
        assert_eq!(
            intent,
            WorkspaceIntent::CloseTabs {
                project_id: ProjectId::new("project"),
                tab_ids: vec![worker.address.tab_id.clone()],
            }
        );
        workspaces.apply(workspace_id(), intent, None).unwrap();
        assert!(
            workspaces
                .locate_pane(&worker.address.terminal_session_id())
                .is_none()
        );

        for tab in ["rival", "plain"] {
            let pane = workspaces
                .locate_pane(&caller(tab).terminal_session_id)
                .unwrap();
            let intent = close_intent(&workspaces, &pane).unwrap();
            workspaces.apply(workspace_id(), intent, None).unwrap();
        }
        let lead = workspaces
            .locate_pane(&caller("lead").terminal_session_id)
            .unwrap();
        assert_eq!(
            code(close_intent(&workspaces, &lead)),
            AgentControlErrorCode::Failed
        );
    }

    #[test]
    fn spawn_requests_name_a_known_provider_and_a_usable_title() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let lead = caller("lead");
        for request in [
            spawn_request("bash", None),
            spawn_request("codex", Some("  ")),
            spawn_request("codex", Some(&"x".repeat(MAX_TITLE_BYTES + 1))),
            AgentSpawnRequest {
                provider: "codex".into(),
                args: vec!["a\0b".into()],
                title: None,
            },
        ] {
            assert_eq!(
                code(spawn(&workspaces, &lead, request)),
                AgentControlErrorCode::InvalidRequest
            );
        }
        let named = spawn(
            &workspaces,
            &lead,
            spawn_request("claude", Some("Reviewer")),
        )
        .unwrap();
        assert_eq!(named.config.title, "Reviewer");
    }
}
