//! What an orchestrator Agent may do to other Agents. The hook listener has
//! already established who is asking; this module decides what that caller
//! may touch and changes the layout through the same intents Clients use.
//!
//! An orchestrator acts only on workers it started or that a person assigned
//! to it (`PaneConfig::spawned_by`). These rules keep an Agent from driving
//! panes a person did not hand it; they do not defend against other
//! processes of the same user.

mod transcript;

use std::{
    path::Path,
    sync::Arc,
    time::{Duration, Instant},
};

use alacritty_terminal::term::TermMode;
use yttt_agent_core::{AgentProcessState, AgentSnapshot, AgentTurnState};

use yttt_agent_providers::builtin_providers;
use yttt_core::model::{
    host_workspace::{NewTab, PaneAddress, WorkspaceIntent},
    ids::TerminalSessionId,
    layout::PaneRole,
    workspace::PaneProcessState,
};
use yttt_protocol::{
    agent::{
        AGENT_CONTROL_CLOSE_PATH, AGENT_CONTROL_INTERRUPT_PATH, AGENT_CONTROL_LIST_PATH,
        AGENT_CONTROL_PROVIDERS, AGENT_CONTROL_RESULT_PATH, AGENT_CONTROL_SEND_PATH,
        AGENT_CONTROL_SPAWN_PATH, AGENT_CONTROL_WAIT_PATH, AgentControlError,
        AgentControlErrorCode, AgentControlIdentity, AgentControlResponse, AgentInterruptRequest,
        AgentResultSource, AgentSendRequest, AgentSpawnRequest, AgentTargetRequest,
        AgentTurnResult, AgentWaitCondition, AgentWaitReason, AgentWaitRequest, ControlledAgent,
        ControlledAgentState, MAX_AGENT_RESULT_BYTES, MAX_AGENT_SEND_BYTES, MAX_AGENT_WAIT_SECONDS,
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
const MAX_WAIT_AGENTS: usize = 16;
/// How often a wait looks at the Agents again.
const WAIT_POLL: Duration = Duration::from_millis(200);
const DEFAULT_INTERRUPT_SECONDS: u32 = 10;

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

    /// Pastes text into a ready worker's prompt and presses Enter, as a
    /// person would; the worker's terminal shows it like typed input.
    fn send(
        &self,
        caller: &AgentControlIdentity,
        request: AgentSendRequest,
    ) -> Result<AgentControlResponse, AgentControlError> {
        if request.text.trim().is_empty()
            || request.text.len() > MAX_AGENT_SEND_BYTES
            || request.text.contains('\0')
        {
            return Err(invalid("text must be 1 byte to 64 KiB without NUL"));
        }
        let worker = owned_worker(&self.workspaces, caller, &request.agent)?;
        let snapshot = worker
            .state
            .agent_snapshot
            .as_ref()
            .filter(|snapshot| accepts_input(snapshot))
            .ok_or_else(|| {
                not_ready("the Agent has not reported that it is ready for a new instruction")
            })?;
        let mut bytes = self.paste(&request.agent, &request.text)?;
        bytes.push(b'\r');
        self.write(&request.agent, bytes)?;
        Ok(AgentControlResponse::Sent {
            agent: request.agent,
            turn_seq_before: snapshot.turn_seq,
        })
    }

    /// Presses the provider's interrupt key in a working worker and waits
    /// for its turn to end. Runs on its own thread; see `agent_control_blocks`.
    fn interrupt(
        &self,
        caller: &AgentControlIdentity,
        request: AgentInterruptRequest,
    ) -> Result<AgentControlResponse, AgentControlError> {
        let timeout = request.timeout_seconds.unwrap_or(DEFAULT_INTERRUPT_SECONDS);
        if timeout > MAX_AGENT_WAIT_SECONDS {
            return Err(invalid("wait at most 600 seconds for an interruption"));
        }
        let worker = owned_worker(&self.workspaces, caller, &request.agent)?;
        let snapshot = worker
            .state
            .agent_snapshot
            .as_ref()
            .ok_or_else(|| not_ready("the Agent has not reported a turn yet"))?;
        let key = interrupt_key(snapshot.provider_id.as_str()).ok_or_else(|| {
            invalid(format!(
                "interrupting {} is not supported; use panes send for its own keys",
                snapshot.provider_id.as_str()
            ))
        })?;
        if !in_turn(snapshot) {
            return Err(not_ready("the Agent is not in a turn"));
        }
        self.write(&request.agent, key.to_vec())?;
        let deadline = Instant::now() + Duration::from_secs(timeout.into());
        let confirmed = loop {
            let ended = self
                .workspaces
                .locate_pane(&request.agent)
                .is_none_or(|worker| {
                    has_ended(&worker)
                        || worker
                            .state
                            .agent_snapshot
                            .as_ref()
                            .is_none_or(|snapshot| !in_turn(snapshot))
                });
            let now = Instant::now();
            if ended || now >= deadline {
                break ended;
            }
            std::thread::sleep(WAIT_POLL.min(deadline - now));
        };
        Ok(AgentControlResponse::Interrupted {
            agent: request.agent,
            confirmed,
        })
    }

    /// Text as the terminal expects a paste: bracketed when its program asked
    /// for that, and with newlines as Enter otherwise.
    fn paste(&self, agent: &TerminalSessionId, text: &str) -> Result<Vec<u8>, AgentControlError> {
        let terminal = self
            .runtime
            .terminal(agent)
            .ok_or_else(|| not_ready("the Agent's terminal is not running"))?;
        let bracketed = terminal.latest_viewport().is_some_and(|viewport| {
            TermMode::from_bits_truncate(viewport.modes.bits).contains(TermMode::BRACKETED_PASTE)
        });
        Ok(paste_bytes(text, bracketed))
    }

    fn write(&self, agent: &TerminalSessionId, bytes: Vec<u8>) -> Result<(), AgentControlError> {
        self.runtime
            .terminal(agent)
            .ok_or_else(|| not_ready("the Agent's terminal is not running"))?
            .input(bytes)
            .map_err(|error| failed(error.to_string()))
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
            AGENT_CONTROL_SEND_PATH => {
                let request = parse::<AgentSendRequest>(body);
                let resource = target_of(request.as_ref().map(|request| &request.agent));
                (
                    "agent_control.send",
                    resource,
                    request.and_then(|request| self.send(caller, request)),
                )
            }
            AGENT_CONTROL_WAIT_PATH => {
                let request = parse::<AgentWaitRequest>(body);
                let resource = request
                    .as_ref()
                    .map(|request| {
                        request
                            .agents
                            .iter()
                            .map(TerminalSessionId::as_str)
                            .collect::<Vec<_>>()
                            .join(",")
                    })
                    .unwrap_or_default();
                (
                    "agent_control.wait",
                    resource,
                    request.and_then(|request| wait(&self.workspaces, caller, request)),
                )
            }
            AGENT_CONTROL_RESULT_PATH => {
                let request = parse::<AgentTargetRequest>(body);
                let resource = target_of(request.as_ref().map(|request| &request.agent));
                (
                    "agent_control.result",
                    resource,
                    request.and_then(|request| result(&self.workspaces, caller, request)),
                )
            }
            AGENT_CONTROL_INTERRUPT_PATH => {
                let request = parse::<AgentInterruptRequest>(body);
                let resource = target_of(request.as_ref().map(|request| &request.agent));
                (
                    "agent_control.interrupt",
                    resource,
                    request.and_then(|request| self.interrupt(caller, request)),
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

/// Blocks until one of the Agents meets the condition, one ends, or the
/// timeout passes. Runs on its own thread; see `agent_control_blocks`.
fn wait(
    workspaces: &WorkspaceService,
    caller: &AgentControlIdentity,
    request: AgentWaitRequest,
) -> Result<AgentControlResponse, AgentControlError> {
    if request.agents.is_empty() || request.agents.len() > MAX_WAIT_AGENTS {
        return Err(invalid("wait for 1 to 16 Agents"));
    }
    if request.timeout_seconds > MAX_AGENT_WAIT_SECONDS {
        return Err(invalid(
            "wait at most 600 seconds; call again to keep waiting",
        ));
    }
    let initial = request
        .agents
        .iter()
        .map(|agent| owned_worker(workspaces, caller, agent).map(|worker| observed(&worker)))
        .collect::<Result<Vec<_>, _>>()?;
    let deadline = Instant::now() + Duration::from_secs(request.timeout_seconds.into());
    loop {
        let workers = request
            .agents
            .iter()
            .map(|agent| workspaces.locate_pane(agent))
            .collect::<Vec<_>>();
        let found = |reason, agent: &TerminalSessionId| AgentControlResponse::Waited {
            reason,
            agent: Some(agent.clone()),
            agents: workers.iter().flatten().map(controlled).collect(),
        };
        for ((agent, worker), before) in request.agents.iter().zip(&workers).zip(&initial) {
            let Some(worker) = worker.as_ref().filter(|worker| !has_ended(worker)) else {
                return Ok(found(AgentWaitReason::Exited, agent));
            };
            if condition_holds(request.until, request.after_turn, before, &observed(worker)) {
                return Ok(found(AgentWaitReason::Matched, agent));
            }
        }
        let now = Instant::now();
        if now >= deadline {
            return Ok(AgentControlResponse::Waited {
                reason: AgentWaitReason::TimedOut,
                agent: None,
                agents: workers.iter().flatten().map(controlled).collect(),
            });
        }
        std::thread::sleep(WAIT_POLL.min(deadline - now));
    }
}

fn result(
    workspaces: &WorkspaceService,
    caller: &AgentControlIdentity,
    request: AgentTargetRequest,
) -> Result<AgentControlResponse, AgentControlError> {
    let worker = owned_worker(workspaces, caller, &request.agent)?;
    let snapshot = worker
        .state
        .agent_snapshot
        .as_ref()
        .ok_or_else(|| not_ready("the Agent has not reported a turn yet"))?;
    let reply = snapshot
        .session
        .as_ref()
        .and_then(|session| session.transcript_path.as_deref())
        .and_then(|path| transcript::final_reply(snapshot.provider_id.as_str(), Path::new(path)));
    let (text, truncated) = match reply {
        Some(text) => {
            let (text, truncated) = bounded(text, MAX_AGENT_RESULT_BYTES);
            (Some(text), truncated)
        }
        None => (None, false),
    };
    Ok(AgentControlResponse::Result {
        result: Box::new(AgentTurnResult {
            agent: request.agent,
            turn_seq: snapshot.turn_seq,
            turn: snapshot.turn_state,
            source: if text.is_some() {
                AgentResultSource::Transcript
            } else {
                AgentResultSource::Unavailable
            },
            text,
            truncated,
        }),
    })
}

/// What a wait compares: whether the Agent runs, its turn and turn number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Observed {
    process: Option<AgentProcessState>,
    turn: Option<AgentTurnState>,
    turn_seq: u64,
}

fn observed(pane: &LocatedPane) -> Observed {
    let snapshot = pane.state.agent_snapshot.as_ref();
    Observed {
        process: snapshot.map(|snapshot| snapshot.process_state),
        turn: snapshot.map(|snapshot| snapshot.turn_state),
        turn_seq: snapshot.map_or(0, |snapshot| snapshot.turn_seq),
    }
}

fn has_ended(pane: &LocatedPane) -> bool {
    pane.state.process_state == PaneProcessState::Exited
        || pane
            .state
            .agent_snapshot
            .as_ref()
            .is_some_and(|snapshot| snapshot.process_state == AgentProcessState::Exited)
}

fn condition_holds(
    until: AgentWaitCondition,
    after_turn: Option<u64>,
    before: &Observed,
    now: &Observed,
) -> bool {
    let later_turn = after_turn.is_none_or(|after| now.turn_seq > after);
    let finished = matches!(
        now.turn,
        Some(AgentTurnState::Completed | AgentTurnState::Failed | AgentTurnState::Interrupted)
    );
    match until {
        AgentWaitCondition::Finished => finished && later_turn,
        AgentWaitCondition::Waiting => now.turn == Some(AgentTurnState::Waiting),
        AgentWaitCondition::Ready => {
            now.process == Some(AgentProcessState::Running)
                && (finished || now.turn == Some(AgentTurnState::Idle))
                && later_turn
        }
        AgentWaitCondition::Change => now != before,
    }
}

/// The states in which the desktop's `agents send` also accepts input.
fn accepts_input(snapshot: &AgentSnapshot) -> bool {
    snapshot.process_state == AgentProcessState::Running
        && matches!(
            snapshot.turn_state,
            AgentTurnState::Idle
                | AgentTurnState::Completed
                | AgentTurnState::Failed
                | AgentTurnState::Interrupted
        )
}

fn in_turn(snapshot: &AgentSnapshot) -> bool {
    snapshot.process_state == AgentProcessState::Running
        && matches!(
            snapshot.turn_state,
            AgentTurnState::Working | AgentTurnState::Waiting
        )
}

/// The key each provider's interface reads as "stop this turn". Others are
/// not driven blind: an unknown key could clear the prompt instead.
fn interrupt_key(provider: &str) -> Option<&'static [u8]> {
    match provider {
        "claude" | "codex" => Some(b"\x1b"),
        _ => None,
    }
}

/// Matches the desktop terminal's paste: control characters that would end
/// a bracketed paste are dropped, and unbracketed newlines become Enter.
fn paste_bytes(text: &str, bracketed: bool) -> Vec<u8> {
    if bracketed {
        let mut bytes = b"\x1b[200~".to_vec();
        bytes.extend(
            text.bytes()
                .filter(|byte| !matches!(byte, b'\x1b' | b'\x03')),
        );
        bytes.extend_from_slice(b"\x1b[201~");
        return bytes;
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

fn bounded(text: String, limit: usize) -> (String, bool) {
    if text.len() <= limit {
        return (text, false);
    }
    let mut end = limit;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (text[..end].to_string(), true)
}

fn target_of(agent: Result<&TerminalSessionId, &AgentControlError>) -> String {
    agent
        .map(|agent| agent.as_str().to_string())
        .unwrap_or_default()
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
                turn_seq: snapshot.turn_seq,
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

fn not_ready(message: impl Into<String>) -> AgentControlError {
    AgentControlError::new(AgentControlErrorCode::NotReady, message)
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

    fn agent_snapshot(
        provider: &'static str,
        turn: AgentTurnState,
        turn_seq: u64,
    ) -> AgentSnapshot {
        let mut snapshot = yttt_agent_core::AgentReducer::new(
            yttt_agent_core::AgentInstanceId::random(),
            yttt_agent_core::ProviderId::from_static(provider),
            1,
        )
        .snapshot()
        .clone();
        snapshot.process_state = AgentProcessState::Running;
        snapshot.turn_state = turn;
        snapshot.turn_seq = turn_seq;
        snapshot
    }

    fn report(workspaces: &WorkspaceService, worker: &LocatedPane, snapshot: AgentSnapshot) {
        workspaces.update_agent_snapshot(&worker.workspace_id, &worker.address, |_| Some(snapshot));
    }

    fn wait_for(
        workspaces: &WorkspaceService,
        agent: &TerminalSessionId,
        until: AgentWaitCondition,
        after_turn: Option<u64>,
        timeout_seconds: u32,
    ) -> Result<(AgentWaitReason, Option<TerminalSessionId>), AgentControlError> {
        let request = AgentWaitRequest {
            agents: vec![agent.clone()],
            until,
            after_turn,
            timeout_seconds,
        };
        match wait(workspaces, &caller("lead"), request)? {
            AgentControlResponse::Waited { reason, agent, .. } => Ok((reason, agent)),
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn a_wait_ends_when_a_turn_after_the_given_one_finishes() {
        let root = tempdir().unwrap();
        let workspaces = Arc::new(workspaces(root.path()));
        let worker = spawn(&workspaces, &caller("lead"), spawn_request("codex", None)).unwrap();
        let agent = worker.address.terminal_session_id();
        report(
            &workspaces,
            &worker,
            agent_snapshot("codex", AgentTurnState::Completed, 3),
        );

        // The turn that finished before the send does not count.
        assert_eq!(
            wait_for(
                &workspaces,
                &agent,
                AgentWaitCondition::Finished,
                Some(3),
                0
            )
            .unwrap(),
            (AgentWaitReason::TimedOut, None)
        );
        let reporter = std::thread::spawn({
            let workspaces = workspaces.clone();
            let worker = worker.clone();
            move || {
                std::thread::sleep(Duration::from_millis(300));
                report(
                    &workspaces,
                    &worker,
                    agent_snapshot("codex", AgentTurnState::Working, 4),
                );
                std::thread::sleep(Duration::from_millis(300));
                report(
                    &workspaces,
                    &worker,
                    agent_snapshot("codex", AgentTurnState::Completed, 4),
                );
            }
        });
        let started = Instant::now();
        assert_eq!(
            wait_for(
                &workspaces,
                &agent,
                AgentWaitCondition::Finished,
                Some(3),
                10
            )
            .unwrap(),
            (AgentWaitReason::Matched, Some(agent.clone()))
        );
        assert!(started.elapsed() >= Duration::from_millis(500));
        assert!(started.elapsed() < Duration::from_secs(5));
        reporter.join().unwrap();
        assert_eq!(
            wait_for(&workspaces, &agent, AgentWaitCondition::Ready, Some(3), 0).unwrap(),
            (AgentWaitReason::Matched, Some(agent))
        );
    }

    #[test]
    fn a_wait_ends_when_its_worker_goes_away_and_refuses_what_it_may_not_watch() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let worker = spawn(&workspaces, &caller("lead"), spawn_request("codex", None)).unwrap();
        let agent = worker.address.terminal_session_id();
        report(
            &workspaces,
            &worker,
            agent_snapshot("codex", AgentTurnState::Working, 1),
        );

        let rival = wait(
            &workspaces,
            &caller("rival"),
            AgentWaitRequest {
                agents: vec![agent.clone()],
                until: AgentWaitCondition::Finished,
                after_turn: None,
                timeout_seconds: 0,
            },
        );
        assert_eq!(code(rival), AgentControlErrorCode::Forbidden);
        assert_eq!(
            code(wait_for(
                &workspaces,
                &agent,
                AgentWaitCondition::Finished,
                None,
                MAX_AGENT_WAIT_SECONDS + 1
            )),
            AgentControlErrorCode::InvalidRequest
        );

        let mut ended = agent_snapshot("codex", AgentTurnState::Working, 1);
        ended.process_state = AgentProcessState::Exited;
        report(&workspaces, &worker, ended);
        assert_eq!(
            wait_for(&workspaces, &agent, AgentWaitCondition::Finished, None, 5).unwrap(),
            (AgentWaitReason::Exited, Some(agent))
        );
    }

    #[test]
    fn a_result_is_the_reply_in_the_workers_transcript() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let worker = spawn(&workspaces, &caller("lead"), spawn_request("claude", None)).unwrap();
        let agent = worker.address.terminal_session_id();
        let read = || match result(
            &workspaces,
            &caller("lead"),
            AgentTargetRequest {
                agent: agent.clone(),
            },
        ) {
            Ok(AgentControlResponse::Result { result }) => *result,
            other => panic!("unexpected {other:?}"),
        };
        assert_eq!(
            code(result(
                &workspaces,
                &caller("lead"),
                AgentTargetRequest {
                    agent: agent.clone()
                }
            )),
            AgentControlErrorCode::NotReady
        );

        report(
            &workspaces,
            &worker,
            agent_snapshot("claude", AgentTurnState::Completed, 2),
        );
        let unknown = read();
        assert_eq!(unknown.source, AgentResultSource::Unavailable);
        assert_eq!(unknown.text, None);

        let transcript = root.path().join("session.jsonl");
        std::fs::write(
            &transcript,
            format!(
                "{}\n",
                serde_json::json!({"type": "assistant", "message": {"content": [{"type": "text", "text": "Fixed 2 tests"}]}})
            ),
        )
        .unwrap();
        let mut snapshot = agent_snapshot("claude", AgentTurnState::Completed, 2);
        snapshot.session = Some(yttt_agent_core::AgentSessionMetadata {
            transcript_path: Some(transcript.display().to_string()),
            ..Default::default()
        });
        report(&workspaces, &worker, snapshot);
        let found = read();
        assert_eq!(found.source, AgentResultSource::Transcript);
        assert_eq!(found.text.as_deref(), Some("Fixed 2 tests"));
        assert_eq!((found.turn_seq, found.turn), (2, AgentTurnState::Completed));
    }

    #[test]
    fn input_helpers_match_the_desktop_and_refuse_unknown_interrupts() {
        assert_eq!(
            paste_bytes("one\ntwo\u{1b}\u{3}", true),
            b"\x1b[200~one\ntwo\x1b[201~".to_vec()
        );
        assert_eq!(
            paste_bytes("one\r\ntwo\nthree", false),
            b"one\rtwo\rthree".to_vec()
        );
        assert_eq!(interrupt_key("claude"), Some(&b"\x1b"[..]));
        assert_eq!(interrupt_key("omp"), None);
        assert_eq!(bounded("ab€".into(), 3), ("ab".into(), true));
        assert_eq!(bounded("ab".into(), 3), ("ab".into(), false));

        let idle = agent_snapshot("codex", AgentTurnState::Idle, 0);
        assert!(accepts_input(&idle) && !in_turn(&idle));
        let waiting = agent_snapshot("codex", AgentTurnState::Waiting, 1);
        assert!(!accepts_input(&waiting) && in_turn(&waiting));
        let seen = |turn, turn_seq| Observed {
            process: Some(AgentProcessState::Running),
            turn: Some(turn),
            turn_seq,
        };
        let before = seen(AgentTurnState::Working, 1);
        assert!(condition_holds(
            AgentWaitCondition::Waiting,
            None,
            &before,
            &seen(AgentTurnState::Waiting, 1)
        ));
        assert!(!condition_holds(
            AgentWaitCondition::Change,
            None,
            &before,
            &before
        ));
        assert!(condition_holds(
            AgentWaitCondition::Change,
            None,
            &before,
            &seen(AgentTurnState::Working, 2)
        ));
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
