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
    collections::{HashMap, VecDeque},
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
    layout::{PaneController, PaneRole},
    workspace::PaneProcessState,
};
use yttt_protocol::{
    agent::{
        AGENT_CONTROL_CLOSE_PATH, AGENT_CONTROL_INTERRUPT_PATH, AGENT_CONTROL_LIST_PATH,
        AGENT_CONTROL_PROVIDERS, AGENT_CONTROL_RELEASE_PATH, AGENT_CONTROL_RESULT_PATH,
        AGENT_CONTROL_SEND_PATH, AGENT_CONTROL_SPAWN_PATH, AGENT_CONTROL_WAIT_PATH,
        AgentControlError, AgentControlErrorCode, AgentControlIdentity, AgentControlResponse,
        AgentInterruptRequest, AgentResultSource, AgentSendRequest, AgentSpawnRequest,
        AgentTargetRequest, AgentTurnResult, AgentWaitCondition, AgentWaitReason, AgentWaitRequest,
        ControlledAgent, ControlledAgentState, MAX_AGENT_QUEUED_MESSAGES, MAX_AGENT_RESULT_BYTES,
        MAX_AGENT_SEND_BYTES, MAX_AGENT_WAIT_SECONDS,
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
/// How long a write may go without the worker starting a turn before the
/// Host stops waiting for one.
const WRITE_CONFIRM_TIMEOUT: Duration = Duration::from_secs(30);
/// How often queued messages are checked against their workers.
const DELIVERY_POLL: Duration = Duration::from_millis(300);

pub(crate) struct AgentControl {
    workspaces: Arc<WorkspaceService>,
    projects: Arc<HostProjectRuntime>,
    runtime: Arc<HostRuntime>,
    pane_runtime: Arc<PaneRuntime>,
    audit: Arc<HostAuditLog>,
    /// Requests arrive on the hook listener's threads; launching a pane may
    /// need the Host's async runtime.
    tokio: tokio::runtime::Handle,
    deliveries: parking_lot::Mutex<HashMap<TerminalSessionId, Deliveries>>,
}

/// What the Host wrote to a worker and still holds for it.
#[derive(Default)]
struct Deliveries {
    queued: VecDeque<String>,
    /// The worker's turn number when the Host last wrote to it, and when.
    /// Nothing more is written until a later turn ends, so two messages never
    /// share one, unless no turn started within `WRITE_CONFIRM_TIMEOUT`: input
    /// such as a local slash command starts none.
    written_after_turn: Option<(u64, Instant)>,
}

impl Deliveries {
    fn may_write(&self, snapshot: &AgentSnapshot) -> bool {
        accepts_input(snapshot)
            && self.written_after_turn.is_none_or(|(turn, at)| {
                snapshot.turn_seq > turn || at.elapsed() >= WRITE_CONFIRM_TIMEOUT
            })
    }
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
            deliveries: parking_lot::Mutex::new(HashMap::new()),
        }
    }

    /// Delivers queued messages as workers become ready, until the Host stops.
    pub(crate) fn start(self: &Arc<Self>, mut stop: tokio::sync::watch::Receiver<bool>) {
        let control = Arc::downgrade(self);
        self.tokio.spawn(async move {
            let mut interval = tokio::time::interval(DELIVERY_POLL);
            loop {
                tokio::select! {
                    _ = interval.tick() => {}
                    _ = stop.changed() => break,
                }
                let Some(control) = control.upgrade() else {
                    break;
                };
                let _ = tokio::task::spawn_blocking(move || control.deliver_queued()).await;
            }
        });
    }

    /// Called for every terminal input a Client sends. A person typing in a
    /// worker the orchestrator drives takes it over, and what the
    /// orchestrator queued is dropped: the person may have changed the task.
    pub(crate) fn person_typed(&self, session_id: &TerminalSessionId, bytes: &[u8]) {
        // A terminal answers its program on its own: focus and mouse reports,
        // cursor and device replies. Only keys and pastes are a person's.
        if is_terminal_report(bytes) {
            return;
        }
        match take_over(&self.workspaces, session_id) {
            Ok(true) => {
                self.deliveries.lock().remove(session_id);
            }
            Ok(false) => {}
            Err(error) => tracing::warn!(
                "worker {} was not marked as driven by a person: {}",
                session_id.as_str(),
                error.message
            ),
        }
    }

    fn deliver_queued(&self) {
        let agents = self.deliveries.lock().keys().cloned().collect::<Vec<_>>();
        for agent in agents {
            let Some(worker) = self
                .workspaces
                .locate_pane(&agent)
                .filter(|worker| !has_ended(worker))
            else {
                self.deliveries.lock().remove(&agent);
                continue;
            };
            if worker.config.controller != PaneController::Auto {
                continue;
            }
            let Some(snapshot) = worker.state.agent_snapshot.as_ref() else {
                continue;
            };
            let text = {
                let mut deliveries = self.deliveries.lock();
                let Some(entry) = deliveries.get_mut(&agent) else {
                    continue;
                };
                if !entry.may_write(snapshot) {
                    continue;
                }
                let Some(text) = entry.queued.pop_front() else {
                    continue;
                };
                entry.written_after_turn = Some((snapshot.turn_seq, Instant::now()));
                text
            };
            if let Err(error) = self.paste_and_enter(&agent, &text) {
                tracing::warn!(
                    "a queued message for {} was not delivered: {}",
                    agent.as_str(),
                    error.message
                );
                let mut deliveries = self.deliveries.lock();
                let entry = deliveries.entry(agent.clone()).or_default();
                entry.queued.push_front(text);
                entry.written_after_turn = None;
            }
        }
    }

    fn queued(&self, agent: &TerminalSessionId) -> usize {
        self.deliveries
            .lock()
            .get(agent)
            .map_or(0, |entry| entry.queued.len())
    }

    /// Fills in the queue lengths only this service knows.
    fn with_queue_lengths(&self, mut response: AgentControlResponse) -> AgentControlResponse {
        let fill = |agent: &mut ControlledAgent| agent.queued = self.queued(&agent.agent);
        match &mut response {
            AgentControlResponse::Spawned { agent } => fill(agent),
            AgentControlResponse::Agents { agents }
            | AgentControlResponse::Waited { agents, .. } => agents.iter_mut().for_each(fill),
            _ => {}
        }
        response
    }

    fn set_controller(
        &self,
        worker: &LocatedPane,
        controller: PaneController,
    ) -> Result<(), AgentControlError> {
        set_controller(&self.workspaces, worker, controller)
    }

    /// Hands a worker to a person, as their typing in it would.
    fn release(
        &self,
        caller: &AgentControlIdentity,
        request: AgentTargetRequest,
    ) -> Result<AgentControlResponse, AgentControlError> {
        let worker = owned_worker(&self.workspaces, caller, &request.agent)?;
        self.deliveries.lock().remove(&request.agent);
        if worker.config.controller != PaneController::Human {
            self.set_controller(&worker, PaneController::Human)?;
        }
        Ok(AgentControlResponse::Released {
            agent: request.agent,
        })
    }

    fn spawn(
        &self,
        caller: &AgentControlIdentity,
        request: AgentSpawnRequest,
    ) -> Result<AgentControlResponse, AgentControlError> {
        let orchestrator = caller_pane(&self.workspaces, caller)?;
        let (intents, address) = spawn_intents(&self.workspaces, caller, &orchestrator, request)?;
        for intent in intents {
            self.apply(&orchestrator.workspace_id, intent)?;
        }
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
        let snapshot = worker.state.agent_snapshot.as_ref();
        let turn_seq_before = snapshot.map_or(0, |snapshot| snapshot.turn_seq);
        let mut deliveries = self.deliveries.lock();
        let entry = deliveries.entry(request.agent.clone()).or_default();
        match send_decision(worker.config.controller, entry, snapshot, request.queue) {
            SendDecision::Write => {
                entry.written_after_turn = Some((turn_seq_before, Instant::now()));
                drop(deliveries);
                if let Err(error) = self.paste_and_enter(&request.agent, &request.text) {
                    if let Some(entry) = self.deliveries.lock().get_mut(&request.agent) {
                        entry.written_after_turn = None;
                    }
                    return Err(error);
                }
                Ok(AgentControlResponse::Sent {
                    agent: request.agent,
                    turn_seq_before,
                    queued: false,
                })
            }
            SendDecision::Queue => {
                entry.queued.push_back(request.text);
                Ok(AgentControlResponse::Sent {
                    agent: request.agent,
                    turn_seq_before,
                    queued: true,
                })
            }
            SendDecision::Refuse(error) => {
                if entry.queued.is_empty() && entry.written_after_turn.is_none() {
                    deliveries.remove(&request.agent);
                }
                Err(error)
            }
        }
    }

    fn paste_and_enter(
        &self,
        agent: &TerminalSessionId,
        text: &str,
    ) -> Result<(), AgentControlError> {
        let mut bytes = self.paste(agent, text)?;
        bytes.push(b'\r');
        self.write(agent, bytes)
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
        if worker.config.controller != PaneController::Auto {
            return Err(human_control("a person is driving or paused this Agent"));
        }
        let snapshot = worker
            .state
            .agent_snapshot
            .as_ref()
            .ok_or_else(|| not_ready("the Agent has not reported a turn yet"))?;
        let key = interrupt_key(snapshot.provider_id.as_str()).ok_or_else(|| {
            invalid(format!(
                "interrupting {} is not supported; ask a person to stop its turn",
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
                    tracing::warn!("closed pane terminal {session_id} was not terminated: {error}");
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
            AGENT_CONTROL_RELEASE_PATH => {
                let request = parse::<AgentTargetRequest>(body);
                let resource = target_of(request.as_ref().map(|request| &request.agent));
                (
                    "agent_control.release",
                    resource,
                    request.and_then(|request| self.release(caller, request)),
                )
            }
            _ => return Err(not_found("unknown control request")),
        };
        let result = result.map(|response| self.with_queue_lengths(response));
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
    if let Some(arg) = loosening_argument(&request.provider, &request.args) {
        return Err(forbidden(format!(
            "{arg} would let the worker skip approvals a person must give; \
             start such a worker yourself instead"
        )));
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
    if worker.config.controller != PaneController::Auto {
        return Err(human_control(
            "a person is driving or paused this Agent; it is theirs to close",
        ));
    }
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

/// Provider options that skip, pre-grant or reconfigure approvals. An
/// orchestrator may not start a worker with them: approvals are a person's
/// (K5). Options providers add later are not known here.
fn loosening_argument<'a>(provider: &str, args: &'a [String]) -> Option<&'a str> {
    let options: &[&str] = match provider {
        "claude" => &[
            "--dangerously-skip-permissions",
            "--permission-mode",
            "--allowedTools",
            "--allowed-tools",
            "--settings",
            "--mcp-config",
        ],
        "codex" => &[
            "--dangerously-bypass-approvals-and-sandbox",
            "--yolo",
            "--full-auto",
            "--ask-for-approval",
            "-a",
            "--sandbox",
            "-s",
            "--config",
            "-c",
            "--profile",
            "-p",
        ],
        _ => &[],
    };
    args.iter()
        .take_while(|arg| *arg != "--")
        .map(String::as_str)
        .find(|arg| {
            options.iter().any(|option| {
                *arg == *option
                    || arg.strip_prefix(option).is_some_and(|rest| {
                        rest.starts_with('=') || (option.len() == 2 && !rest.is_empty())
                    })
            })
        })
}

/// Whether input holds only replies a terminal sends its program by itself.
fn is_terminal_report(bytes: &[u8]) -> bool {
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] != 0x1b {
            return false;
        }
        match bytes.get(index + 1) {
            Some(b'[') if bytes.get(index + 2) == Some(&b'M') => {
                // X10 mouse: ESC [ M and three coordinate bytes.
                if index + 6 > bytes.len() {
                    return false;
                }
                index += 6;
            }
            Some(b'[') => {
                let start = index + 2;
                let mut end = start;
                if matches!(bytes.get(end), Some(b'<' | b'?' | b'>')) {
                    end += 1;
                }
                while bytes
                    .get(end)
                    .is_some_and(|byte| byte.is_ascii_digit() || matches!(byte, b';' | b':'))
                {
                    end += 1;
                }
                let Some(&last) = bytes.get(end) else {
                    return false;
                };
                let parameters = &bytes[start..end];
                let report = match last {
                    b'I' | b'O' => parameters.is_empty(),
                    b'M' | b'm' => parameters.first() == Some(&b'<') || parameters.contains(&b';'),
                    b'R' | b't' => parameters.contains(&b';'),
                    b'c' => matches!(parameters.first(), Some(b'?' | b'>')),
                    b'n' => !parameters.is_empty(),
                    _ => false,
                };
                if !report {
                    return false;
                }
                index = end + 1;
            }
            Some(b']' | b'P' | b'_') => {
                // OSC, DCS and APC replies end with BEL or ESC \.
                let body = &bytes[index + 2..];
                let Some(end) = body
                    .iter()
                    .enumerate()
                    .find_map(|(offset, byte)| match byte {
                        0x07 => Some(offset + 1),
                        0x1b if body.get(offset + 1) == Some(&b'\\') => Some(offset + 2),
                        _ => None,
                    })
                else {
                    return false;
                };
                index += 2 + end;
            }
            _ => return false,
        }
    }
    !bytes.is_empty()
}

/// The intents that create a worker tab and then show again what the project
/// showed: creating a tab selects it, and a person's view and keys stay put.
fn spawn_intents(
    workspaces: &WorkspaceService,
    caller: &AgentControlIdentity,
    orchestrator: &LocatedPane,
    request: AgentSpawnRequest,
) -> Result<(Vec<WorkspaceIntent>, PaneAddress), AgentControlError> {
    let (create, address) = spawn_intent(caller, orchestrator, request)?;
    let shown =
        workspaces.active_work_item(&orchestrator.workspace_id, &orchestrator.address.project_id);
    let mut intents = vec![create];
    intents.extend(shown.map(|item| WorkspaceIntent::ActivateItem {
        project_id: orchestrator.address.project_id.clone(),
        item,
    }));
    Ok((intents, address))
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
            let now = observed(worker);
            if now.controller != before.controller {
                return Ok(found(AgentWaitReason::ControlChanged, agent));
            }
            if condition_holds(request.until, request.after_turn, before, &now) {
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

enum SendDecision {
    Write,
    Queue,
    Refuse(AgentControlError),
}

/// Whether text for a worker is written now, held, or refused. Queued text
/// keeps its order: nothing is written past it.
fn send_decision(
    controller: PaneController,
    entry: &Deliveries,
    snapshot: Option<&AgentSnapshot>,
    queue: bool,
) -> SendDecision {
    if controller == PaneController::Auto
        && entry.queued.is_empty()
        && snapshot.is_some_and(|snapshot| entry.may_write(snapshot))
    {
        return SendDecision::Write;
    }
    if queue {
        if entry.queued.len() >= MAX_AGENT_QUEUED_MESSAGES {
            return SendDecision::Refuse(not_ready(
                "32 messages are already queued for this Agent",
            ));
        }
        return SendDecision::Queue;
    }
    SendDecision::Refuse(match controller {
        PaneController::Auto => {
            not_ready("the Agent has not reported that it is ready for a new instruction")
        }
        PaneController::Human => human_control("a person is driving this Agent"),
        PaneController::Paused => human_control("a person paused this Agent"),
    })
}

/// Marks a worker its orchestrator drives as driven by a person. Returns
/// whether that changed anything.
fn take_over(
    workspaces: &WorkspaceService,
    session_id: &TerminalSessionId,
) -> Result<bool, AgentControlError> {
    let Some(worker) = workspaces
        .locate_pane(session_id)
        .filter(|pane| pane.config.spawned_by.is_some())
        .filter(|pane| pane.config.controller == PaneController::Auto)
    else {
        return Ok(false);
    };
    set_controller(workspaces, &worker, PaneController::Human).map(|()| true)
}

fn set_controller(
    workspaces: &WorkspaceService,
    worker: &LocatedPane,
    controller: PaneController,
) -> Result<(), AgentControlError> {
    workspaces
        .apply(
            worker.workspace_id.clone(),
            WorkspaceIntent::SetPaneController {
                project_id: worker.address.project_id.clone(),
                tab_id: worker.address.tab_id.clone(),
                pane_id: worker.address.pane_id.clone(),
                controller,
            },
            None,
        )
        .map(|_| ())
        .map_err(|error| failed(error.message))
}

/// What a wait compares: whether the Agent runs, its turn and turn number.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Observed {
    process: Option<AgentProcessState>,
    turn: Option<AgentTurnState>,
    turn_seq: u64,
    controller: PaneController,
}

fn observed(pane: &LocatedPane) -> Observed {
    let snapshot = pane.state.agent_snapshot.as_ref();
    Observed {
        controller: pane.config.controller,
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
        controller: pane.config.controller,
        queued: 0,
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

fn human_control(message: impl Into<String>) -> AgentControlError {
    AgentControlError::new(AgentControlErrorCode::HumanControl, message)
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
        let (intents, address) = spawn_intents(workspaces, caller, &orchestrator, request)?;
        for intent in intents {
            workspaces
                .apply(orchestrator.workspace_id.clone(), intent, None)
                .unwrap();
        }
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

        let shown = workspaces.active_work_item(&workspace_id(), &ProjectId::new("project"));
        assert!(shown.is_some());
        let worker = spawn(&workspaces, &lead, spawn_request("codex", None)).unwrap();
        assert_eq!(
            workspaces.active_work_item(&workspace_id(), &ProjectId::new("project")),
            shown,
            "spawning keeps what the person was looking at"
        );
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
    fn a_person_typing_takes_over_only_workers_an_orchestrator_drives() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let worker = spawn(&workspaces, &caller("lead"), spawn_request("codex", None)).unwrap();
        let agent = worker.address.terminal_session_id();
        let controller = || workspaces.locate_pane(&agent).unwrap().config.controller;

        assert!(take_over(&workspaces, &agent).unwrap());
        assert_eq!(controller(), PaneController::Human);
        assert!(!take_over(&workspaces, &agent).unwrap());
        for pane in ["lead", "plain"] {
            assert!(!take_over(&workspaces, &caller(pane).terminal_session_id).unwrap());
        }
        assert!(!take_over(&workspaces, &TerminalSessionId::new("project:gone:gone")).unwrap());

        let paused = workspaces.locate_pane(&agent).unwrap();
        set_controller(&workspaces, &paused, PaneController::Paused).unwrap();
        // Typing in a paused worker does not turn a pause into a takeover.
        assert!(!take_over(&workspaces, &agent).unwrap());
        assert_eq!(controller(), PaneController::Paused);
    }

    #[test]
    fn text_is_written_once_per_turn_held_in_order_or_refused() {
        let ready = agent_snapshot("codex", AgentTurnState::Completed, 2);
        let busy = agent_snapshot("codex", AgentTurnState::Working, 3);
        let decide = |controller, entry: &Deliveries, snapshot, queue| match send_decision(
            controller, entry, snapshot, queue,
        ) {
            SendDecision::Write => "write".to_string(),
            SendDecision::Queue => "queue".to_string(),
            SendDecision::Refuse(error) => format!("{:?}", error.code),
        };
        let empty = Deliveries::default();
        assert_eq!(
            decide(PaneController::Auto, &empty, Some(&ready), false),
            "write"
        );
        assert_eq!(
            decide(PaneController::Auto, &empty, Some(&busy), false),
            "NotReady"
        );
        assert_eq!(
            decide(PaneController::Auto, &empty, Some(&busy), true),
            "queue"
        );
        assert_eq!(
            decide(PaneController::Auto, &empty, None, false),
            "NotReady"
        );
        assert_eq!(
            decide(PaneController::Human, &empty, Some(&ready), false),
            "HumanControl"
        );
        assert_eq!(
            decide(PaneController::Paused, &empty, Some(&ready), true),
            "queue"
        );

        // Written after turn 2: the snapshot still says turn 2 completed
        // until the worker reports the new turn, so nothing else is written.
        let written = Deliveries {
            written_after_turn: Some((2, Instant::now())),
            ..Deliveries::default()
        };
        assert_eq!(
            decide(PaneController::Auto, &written, Some(&ready), false),
            "NotReady"
        );
        let next = agent_snapshot("codex", AgentTurnState::Completed, 3);
        assert_eq!(
            decide(PaneController::Auto, &written, Some(&next), false),
            "write"
        );
        // Input that started no turn, such as a slash command, does not block
        // the worker forever.
        let unanswered = Deliveries {
            written_after_turn: Some((
                2,
                Instant::now()
                    .checked_sub(WRITE_CONFIRM_TIMEOUT + Duration::from_secs(1))
                    .unwrap(),
            )),
            ..Deliveries::default()
        };
        assert_eq!(
            decide(PaneController::Auto, &unanswered, Some(&ready), false),
            "write"
        );

        // Queued text keeps its place ahead of later sends.
        let waiting = Deliveries {
            queued: VecDeque::from(["first".to_string()]),
            ..Deliveries::default()
        };
        assert_eq!(
            decide(PaneController::Auto, &waiting, Some(&ready), false),
            "NotReady"
        );
        let full = Deliveries {
            queued: (0..MAX_AGENT_QUEUED_MESSAGES)
                .map(|i| i.to_string())
                .collect(),
            ..Deliveries::default()
        };
        assert_eq!(
            decide(PaneController::Auto, &full, Some(&busy), true),
            "NotReady"
        );
    }

    #[test]
    fn a_wait_ends_when_a_person_takes_the_worker() {
        let root = tempdir().unwrap();
        let workspaces = Arc::new(workspaces(root.path()));
        let worker = spawn(&workspaces, &caller("lead"), spawn_request("codex", None)).unwrap();
        let agent = worker.address.terminal_session_id();
        report(
            &workspaces,
            &worker,
            agent_snapshot("codex", AgentTurnState::Working, 1),
        );
        let person = std::thread::spawn({
            let workspaces = workspaces.clone();
            let agent = agent.clone();
            move || {
                std::thread::sleep(Duration::from_millis(300));
                take_over(&workspaces, &agent).unwrap();
            }
        });
        assert_eq!(
            wait_for(&workspaces, &agent, AgentWaitCondition::Finished, None, 10).unwrap(),
            (AgentWaitReason::ControlChanged, Some(agent))
        );
        person.join().unwrap();
    }

    #[test]
    fn terminal_reports_are_not_a_person_typing() {
        for report in [
            &b"\x1b[I"[..],
            b"\x1b[O",
            b"\x1b[<0;12;5M\x1b[<0;12;5m",
            b"\x1b[<64;3;4M",
            b"\x1b[M !!",
            b"\x1b[32;10;5M",
            b"\x1b[12;40R",
            b"\x1b[?62;22c",
            b"\x1b[>1;10;0c",
            b"\x1b[0n",
            b"\x1b]11;rgb:0000/0000/0000\x1b\\",
            b"\x1b]10;rgb:ffff/ffff/ffff\x07",
            b"\x1bP1$r0m\x1b\\",
            b"\x1b[8;24;80t",
        ] {
            assert!(is_terminal_report(report), "{report:?}");
        }
        for input in [
            &b"y"[..],
            b"\r",
            b"\x1b",
            b"\x1b[A",
            b"\x1b[1;5C",
            b"\x1b[2~",
            b"\x1b[200~text\x1b[201~",
            b"\x1b[Ia",
            b"\x1b[<0;12",
            b"\x1b]11;unterminated",
            b"",
        ] {
            assert!(!is_terminal_report(input), "{input:?}");
        }
    }

    #[test]
    fn an_orchestrator_cannot_start_a_worker_that_skips_approvals() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let lead = caller("lead");
        let with_args = |provider: &str, args: &[&str]| AgentSpawnRequest {
            provider: provider.into(),
            args: args.iter().map(|arg| arg.to_string()).collect(),
            title: None,
        };
        for (provider, args) in [
            ("claude", &["--dangerously-skip-permissions", "task"][..]),
            ("claude", &["--permission-mode=bypassPermissions"]),
            ("claude", &["--allowedTools", "Bash"]),
            ("codex", &["--yolo"]),
            ("codex", &["-c", "approval_policy=never"]),
            ("codex", &["-capproval_policy=never"]),
            ("codex", &["--sandbox=danger-full-access"]),
        ] {
            assert_eq!(
                code(spawn(&workspaces, &lead, with_args(provider, args))),
                AgentControlErrorCode::Forbidden,
                "{provider} {args:?}"
            );
        }
        // Words after `--` are the prompt, and look-alikes are not options.
        for (provider, args) in [
            ("codex", &["--", "explain --yolo in the docs"][..]),
            ("codex", &["--cd", "crates"]),
            ("claude", &["--model", "sonnet", "Fix -c handling"]),
        ] {
            assert!(
                spawn(&workspaces, &lead, with_args(provider, args)).is_ok(),
                "{provider} {args:?}"
            );
        }
    }

    #[test]
    fn a_worker_a_person_drives_is_theirs_to_close() {
        let root = tempdir().unwrap();
        let workspaces = workspaces(root.path());
        let worker = spawn(&workspaces, &caller("lead"), spawn_request("codex", None)).unwrap();
        set_controller(&workspaces, &worker, PaneController::Human).unwrap();
        let taken = workspaces
            .locate_pane(&worker.address.terminal_session_id())
            .unwrap();
        assert_eq!(
            code(close_intent(&workspaces, &taken)),
            AgentControlErrorCode::HumanControl
        );
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
            controller: PaneController::Auto,
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
