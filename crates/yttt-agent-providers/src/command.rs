use crate::{OmpProvider, provider_for_command};
use serde_json::Value;
use yttt_agent_core::{
    AgentAction, AgentBoardEvent, AgentEventKind, AgentExecutionRecord, AgentExecutionTelemetry,
    AgentPlanDelta, AgentPlanStatus, AgentProvider, AgentSessionMetadata, AgentTask,
    AgentTaskSource, AgentTurnState, ChildAgentDescriptor, ChildAgentUpdate, ProviderDescriptor,
    ProviderError, ProviderHookEvent, ProviderId, ProviderResumeCommand, TurnOutcome,
    WaitingReason,
};

pub const CODEX_PROVIDER_ID: &str = "codex";
pub const CLAUDE_PROVIDER_ID: &str = "claude";
pub const GROK_PROVIDER_ID: &str = "grok";
pub const OPENCODE_PROVIDER_ID: &str = "opencode";
pub const PI_PROVIDER_ID: &str = "pi";

#[derive(Clone, Copy, Debug, Default)]
pub struct CodexProvider;

#[derive(Clone, Copy, Debug, Default)]
pub struct ClaudeProvider;

#[derive(Clone, Copy, Debug, Default)]
pub struct GrokProvider;

#[derive(Clone, Copy, Debug, Default)]
pub struct OpenCodeProvider;

#[derive(Clone, Copy, Debug, Default)]
pub struct PiProvider;

impl AgentProvider for CodexProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(CODEX_PROVIDER_ID, "Codex")
    }

    fn matches_command(&self, command: &str) -> bool {
        provider_for_command(command) == Some(CODEX_PROVIDER_ID)
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        resume_with_session_id(CODEX_PROVIDER_ID, "resume", session)
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        normalize_command_hook(event, Some(NativeObservationProvider::Codex))
    }
}

impl AgentProvider for ClaudeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(CLAUDE_PROVIDER_ID, "Claude Code")
    }

    fn matches_command(&self, command: &str) -> bool {
        provider_for_command(command) == Some(CLAUDE_PROVIDER_ID)
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        resume_with_session_id(CLAUDE_PROVIDER_ID, "--resume", session)
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        normalize_command_hook(event, Some(NativeObservationProvider::Claude))
    }
}

impl AgentProvider for GrokProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(GROK_PROVIDER_ID, "Grok")
    }

    fn matches_command(&self, command: &str) -> bool {
        provider_for_command(command) == Some(GROK_PROVIDER_ID)
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        resume_with_session_id(GROK_PROVIDER_ID, "--resume", session)
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        normalize_command_hook(event, None)
    }
}

impl AgentProvider for OpenCodeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(OPENCODE_PROVIDER_ID, "OpenCode")
    }

    fn matches_command(&self, command: &str) -> bool {
        provider_for_command(command) == Some(OPENCODE_PROVIDER_ID)
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        resume_with_session_id(OPENCODE_PROVIDER_ID, "--session", session)
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        match event.name {
            "board_event" | "child_started" | "child_updated" | "child_finished"
            | "token_usage" => {
                return OmpProvider.normalize_hook(event);
            }
            _ => {}
        }
        let payload = event.payload;
        let events = match event.name {
            "session_start" => vec![session_started(payload)],
            "session_updated" => vec![AgentEventKind::SessionUpdated {
                metadata: session_metadata(payload),
            }],
            "user_prompt" => {
                with_session_update(payload, turn_started(payload, &["prompt", "text"]))
            }
            "session_busy" => vec![AgentEventKind::Working],
            "session_idle" => vec![AgentEventKind::TurnFinished {
                outcome: TurnOutcome::Completed,
            }],
            // OpenCode emits recoverable session.error events during retries and compaction.
            // session.status/session.idle remains the lifecycle authority.
            "session_error" => Vec::new(),
            "permission_request" => waiting_events(payload, WaitingReason::Approval),
            "ask_user_question" => waiting_events(payload, WaitingReason::UserInput),
            "tool_execution_start" => vec![action_started(payload)?],
            "tool_execution_end" => vec![action_finished(payload)],
            name => return Err(ProviderError::UnsupportedEvent(name.to_string())),
        };
        Ok(events)
    }
}

impl AgentProvider for PiProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(PI_PROVIDER_ID, "Pi")
    }

    fn matches_command(&self, command: &str) -> bool {
        provider_for_command(command) == Some(PI_PROVIDER_ID)
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        let transcript_path = session.transcript_path.as_deref()?.trim();
        (!transcript_path.is_empty()).then(|| ProviderResumeCommand {
            program: PI_PROVIDER_ID,
            arguments: vec!["--session".to_string(), transcript_path.to_string()],
        })
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        if event.name == "token_usage" {
            return Err(ProviderError::UnsupportedEvent(event.name.to_string()));
        }
        OmpProvider.normalize_hook(event)
    }
}

#[derive(Clone, Copy)]
enum NativeObservationProvider {
    Claude,
    Codex,
}

fn normalize_command_hook(
    event: ProviderHookEvent<'_>,
    observation: Option<NativeObservationProvider>,
) -> Result<Vec<AgentEventKind>, ProviderError> {
    let payload = event.payload;
    let events = match event.name {
        "SessionStart" | "session_start" => vec![session_started(payload)],
        "SessionEnd" | "session_end" => vec![AgentEventKind::SessionEnded],
        "UserPromptSubmit" | "user_prompt_submit" => {
            observed_turn_started(payload, observation, &["prompt", "user_prompt"])
        }
        "PreToolUse" | "pre_tool_use" if is_user_question(payload) => {
            observed_waiting(payload, observation, WaitingReason::UserInput)
        }
        "PreToolUse" | "pre_tool_use" => observed_action_started(payload, observation)?,
        "PermissionRequest" | "permission_request" => {
            observed_waiting(payload, observation, WaitingReason::Approval)
        }
        "PostToolUse" | "post_tool_use" => observed_action_finished(payload, observation, false),
        "PostToolUseFailure" | "post_tool_use_failure" => {
            observed_action_finished(payload, observation, true)
        }
        "PermissionDenied" | "permission_denied" => {
            observed_action_finished(payload, observation, true)
        }
        "Stop" | "stop" => {
            let outcome = if bool_field(payload, &["is_interrupt", "interrupted"]) == Some(true) {
                TurnOutcome::Interrupted
            } else {
                TurnOutcome::Completed
            };
            observed_turn_finished(payload, observation, outcome)
        }
        "StopFailure" | "stop_failure" => {
            observed_turn_finished(payload, observation, TurnOutcome::Failed)
        }
        "SubagentStart" | "subagent_start" => observed_child_started(payload, observation)?,
        "SubagentStop" | "subagent_stop" => observed_child_finished(payload, observation)?,
        name => return Err(ProviderError::UnsupportedEvent(name.to_string())),
    };
    Ok(events)
}

fn descriptor(id: &'static str, display_name: &'static str) -> ProviderDescriptor {
    ProviderDescriptor {
        id: ProviderId::from_static(id),
        display_name,
    }
}

fn session_started(payload: &Value) -> AgentEventKind {
    AgentEventKind::SessionStarted {
        metadata: session_metadata(payload),
    }
}

fn session_metadata(payload: &Value) -> AgentSessionMetadata {
    AgentSessionMetadata {
        session_id: string_field(
            payload,
            &[
                "session_id",
                "sessionId",
                "sessionID",
                "conversation_id",
                "conversationId",
                "id",
            ],
        ),
        model: string_field(payload, &["model", "model_id", "modelId"]),
        title: string_field(
            payload,
            &[
                "title",
                "session_title",
                "sessionTitle",
                "custom_title",
                "customTitle",
            ],
        ),
        transcript_path: string_field(
            payload,
            &[
                "transcript_path",
                "transcriptPath",
                "session_file",
                "sessionFile",
            ],
        ),
    }
}

fn with_session_update(payload: &Value, event: AgentEventKind) -> Vec<AgentEventKind> {
    let metadata = session_metadata(payload);
    let has_metadata = metadata.session_id.is_some()
        || metadata.model.is_some()
        || metadata.title.is_some()
        || metadata.transcript_path.is_some();
    let mut events = Vec::with_capacity(if has_metadata { 2 } else { 1 });
    if has_metadata {
        events.push(AgentEventKind::SessionUpdated { metadata });
    }
    events.push(event);
    events
}

fn resume_with_session_id(
    program: &'static str,
    option: &'static str,
    session: &AgentSessionMetadata,
) -> Option<ProviderResumeCommand> {
    let session_id = session.session_id.as_deref()?.trim();
    (!session_id.is_empty()).then(|| ProviderResumeCommand {
        program,
        arguments: vec![option.to_string(), session_id.to_string()],
    })
}

fn turn_started(payload: &Value, fields: &[&str]) -> AgentEventKind {
    AgentEventKind::TurnStarted {
        task: string_field(payload, fields)
            .and_then(|prompt| AgentTask::new(prompt, AgentTaskSource::UserPromptHook)),
    }
}

fn action(payload: &Value) -> Result<AgentAction, ProviderError> {
    let name = string_field(payload, &["tool_name", "toolName", "name", "tool"])
        .ok_or_else(|| ProviderError::InvalidPayload("tool name is required".to_string()))?;
    let id = string_field(
        payload,
        &[
            "tool_use_id",
            "toolUseId",
            "tool_call_id",
            "toolCallId",
            "call_id",
            "callId",
        ],
    );
    let detail = string_field(payload, &["detail", "intent"])
        .or_else(|| payload.get("tool_input").and_then(tool_detail))
        .or_else(|| payload.get("toolInput").and_then(tool_detail))
        .or_else(|| payload.get("args").and_then(tool_detail))
        .or_else(|| payload.get("input").and_then(tool_detail));
    AgentAction::new(id, name, detail)
        .ok_or_else(|| ProviderError::InvalidPayload("tool name cannot be empty".to_string()))
}

fn action_started(payload: &Value) -> Result<AgentEventKind, ProviderError> {
    Ok(AgentEventKind::ActionStarted {
        action: action(payload)?,
    })
}

fn action_finished(payload: &Value) -> AgentEventKind {
    action_finished_with_failure(
        payload,
        bool_field(payload, &["is_error", "isError", "failed"]).unwrap_or(false),
    )
}

fn action_finished_with_failure(payload: &Value, failed: bool) -> AgentEventKind {
    AgentEventKind::ActionFinished {
        action_id: string_field(
            payload,
            &[
                "tool_use_id",
                "toolUseId",
                "tool_call_id",
                "toolCallId",
                "call_id",
                "callId",
            ],
        ),
        failed,
    }
}

fn waiting_events(payload: &Value, reason: WaitingReason) -> Vec<AgentEventKind> {
    let mut events = Vec::with_capacity(2);
    if let Ok(action) = action_started(payload) {
        events.push(action);
    }
    events.push(AgentEventKind::Waiting {
        reason,
        message: string_field(payload, &["reason", "message", "description"]),
    });
    events
}

fn child_started(payload: &Value) -> Result<Vec<AgentEventKind>, ProviderError> {
    let id = string_field(
        payload,
        &[
            "agent_id",
            "agentId",
            "child_id",
            "childId",
            "subagentId",
            "subagent_id",
        ],
    )
    .ok_or_else(|| ProviderError::InvalidPayload("child id is required".to_string()))?;
    Ok(vec![AgentEventKind::ChildStarted {
        child: ChildAgentDescriptor {
            id,
            name: string_field(
                payload,
                &[
                    "agent_type",
                    "agentType",
                    "subagentType",
                    "subagent_type",
                    "name",
                ],
            ),
            task: string_field(payload, &["description", "task", "prompt"])
                .and_then(|task| AgentTask::new(task, AgentTaskSource::External)),
        },
    }])
}

impl NativeObservationProvider {
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex",
        }
    }

    fn root_execution_id(self, payload: &Value) -> Option<String> {
        let session_id = string_field(
            payload,
            &[
                "session_id",
                "sessionId",
                "sessionID",
                "conversation_id",
                "conversationId",
            ],
        )?;
        let run_id = match self {
            Self::Claude => string_field(payload, &["prompt_id", "promptId"])?,
            Self::Codex => string_field(payload, &["turn_id", "turnId"])?,
        };
        let provider = match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        };
        Some(format!("{provider}:{session_id}:{run_id}"))
    }
}

fn child_id(payload: &Value) -> Option<String> {
    string_field(
        payload,
        &[
            "agent_id",
            "agentId",
            "child_id",
            "childId",
            "subagentId",
            "subagent_id",
        ],
    )
}

fn child_execution_id(root_id: &str, child_id: &str) -> String {
    format!("{root_id}:child:{child_id}")
}

fn outcome_state(outcome: TurnOutcome) -> AgentTurnState {
    match outcome {
        TurnOutcome::Completed => AgentTurnState::Completed,
        TurnOutcome::Failed => AgentTurnState::Failed,
        TurnOutcome::Interrupted => AgentTurnState::Interrupted,
    }
}

fn child_outcome(payload: &Value) -> TurnOutcome {
    if bool_field(payload, &["is_interrupt", "interrupted"]) == Some(true) {
        TurnOutcome::Interrupted
    } else if bool_field(payload, &["failed", "is_error", "isError"]) == Some(true) {
        TurnOutcome::Failed
    } else {
        TurnOutcome::Completed
    }
}

fn observed_turn_started(
    payload: &Value,
    observation: Option<NativeObservationProvider>,
    fields: &[&str],
) -> Vec<AgentEventKind> {
    let task = string_field(payload, fields);
    let mut events = with_session_update(payload, turn_started(payload, fields));
    if let Some(observation) = observation
        && let Some(event) =
            root_execution(observation, payload, task, AgentTurnState::Working, None)
    {
        events.push(AgentEventKind::Board { event });
    }
    events
}

fn observed_action_started(
    payload: &Value,
    observation: Option<NativeObservationProvider>,
) -> Result<Vec<AgentEventKind>, ProviderError> {
    let action = action(payload)?;
    let Some(observation) = observation else {
        return Ok(vec![AgentEventKind::ActionStarted { action }]);
    };
    let action_label = action.label();
    if let Some(child_id) = child_id(payload) {
        let mut events = vec![AgentEventKind::ChildUpdated {
            child_id: child_id.clone(),
            update: ChildAgentUpdate {
                task: None,
                current_action: Some(action),
                clear_current_action: false,
                turn_state: Some(AgentTurnState::Working),
            },
        }];
        if let Some(root_id) = observation.root_execution_id(payload) {
            events.push(AgentEventKind::ChildBoard {
                child_id: child_id.clone(),
                event: AgentBoardEvent::ExecutionActivity {
                    execution_id: child_execution_id(&root_id, &child_id),
                    current_action: Some(action_label),
                    clear_current_action: false,
                    state: Some(AgentTurnState::Working),
                },
            });
        }
        return Ok(events);
    }
    let mut events = vec![AgentEventKind::ActionStarted { action }];
    if let Some(root_id) = observation.root_execution_id(payload) {
        events.push(AgentEventKind::Board {
            event: AgentBoardEvent::ExecutionActivity {
                execution_id: root_id,
                current_action: Some(action_label),
                clear_current_action: false,
                state: Some(AgentTurnState::Working),
            },
        });
    }
    Ok(events)
}

fn observed_waiting(
    payload: &Value,
    observation: Option<NativeObservationProvider>,
    reason: WaitingReason,
) -> Vec<AgentEventKind> {
    let Some(observation) = observation else {
        return waiting_events(payload, reason);
    };
    let action = action(payload).ok();
    let action_label = action.as_ref().map(AgentAction::label);
    if let Some(child_id) = child_id(payload) {
        let mut events = vec![AgentEventKind::ChildUpdated {
            child_id: child_id.clone(),
            update: ChildAgentUpdate {
                task: None,
                current_action: action,
                clear_current_action: false,
                turn_state: Some(AgentTurnState::Waiting),
            },
        }];
        if let Some(root_id) = observation.root_execution_id(payload) {
            events.push(AgentEventKind::ChildBoard {
                child_id: child_id.clone(),
                event: AgentBoardEvent::ExecutionActivity {
                    execution_id: child_execution_id(&root_id, &child_id),
                    current_action: action_label,
                    clear_current_action: false,
                    state: Some(AgentTurnState::Waiting),
                },
            });
        }
        return events;
    }
    // Permission hooks can lack a tool_use_id. Do not overwrite a matching pre-tool
    // observation with an uncorrelatable action, or its later PostToolUse cannot clear it.
    let root_action = action.filter(|action| action.id.is_some());
    let root_action_label = root_action.as_ref().map(AgentAction::label);
    let mut events = Vec::with_capacity(3);
    if let Some(action) = root_action {
        events.push(AgentEventKind::ActionStarted { action });
    }
    events.push(AgentEventKind::Waiting {
        reason,
        message: string_field(payload, &["reason", "message", "description"]),
    });
    if let Some(root_id) = observation.root_execution_id(payload) {
        events.push(AgentEventKind::Board {
            event: AgentBoardEvent::ExecutionActivity {
                execution_id: root_id,
                current_action: root_action_label,
                clear_current_action: false,
                state: Some(AgentTurnState::Waiting),
            },
        });
    }
    events
}

fn observed_action_finished(
    payload: &Value,
    observation: Option<NativeObservationProvider>,
    failed: bool,
) -> Vec<AgentEventKind> {
    let Some(observation) = observation else {
        return vec![action_finished_with_failure(payload, failed)];
    };
    let mut events = Vec::with_capacity(4);
    if let Some(child_id) = child_id(payload) {
        events.push(AgentEventKind::ChildUpdated {
            child_id: child_id.clone(),
            update: ChildAgentUpdate {
                task: None,
                current_action: None,
                clear_current_action: true,
                turn_state: Some(AgentTurnState::Working),
            },
        });
        if let Some(root_id) = observation.root_execution_id(payload) {
            let execution_id = child_execution_id(&root_id, &child_id);
            events.push(AgentEventKind::ChildBoard {
                child_id: child_id.clone(),
                event: AgentBoardEvent::ExecutionActivity {
                    execution_id: execution_id.clone(),
                    current_action: None,
                    clear_current_action: true,
                    state: Some(AgentTurnState::Working),
                },
            });
            if !failed && let Some(delta) = confirmed_task_delta(payload) {
                events.push(AgentEventKind::ChildBoard {
                    child_id,
                    event: AgentBoardEvent::PlanDelta {
                        execution_id,
                        delta,
                    },
                });
            }
        }
    } else {
        events.push(action_finished_with_failure(payload, failed));
        if let Some(root_id) = observation.root_execution_id(payload) {
            events.push(AgentEventKind::Board {
                event: AgentBoardEvent::ExecutionActivity {
                    execution_id: root_id.clone(),
                    current_action: None,
                    clear_current_action: true,
                    state: Some(AgentTurnState::Working),
                },
            });
            if !failed && let Some(delta) = confirmed_task_delta(payload) {
                events.push(AgentEventKind::Board {
                    event: AgentBoardEvent::PlanDelta {
                        execution_id: root_id,
                        delta,
                    },
                });
            }
            if !failed {
                append_agent_result_telemetry(&mut events, observation, payload);
            }
        }
    }
    events
}

fn observed_turn_finished(
    payload: &Value,
    observation: Option<NativeObservationProvider>,
    outcome: TurnOutcome,
) -> Vec<AgentEventKind> {
    let mut events = vec![AgentEventKind::TurnFinished { outcome }];
    if let Some(observation) = observation
        && let Some(event) =
            root_execution(observation, payload, None, outcome_state(outcome), Some(0))
    {
        events.push(AgentEventKind::Board { event });
    }
    events
}

fn observed_child_started(
    payload: &Value,
    observation: Option<NativeObservationProvider>,
) -> Result<Vec<AgentEventKind>, ProviderError> {
    let mut events = child_started(payload)?;
    let (Some(observation), Some(child_id)) = (observation, child_id(payload)) else {
        return Ok(events);
    };
    if let Some(root_id) = observation.root_execution_id(payload) {
        events.push(AgentEventKind::ChildBoard {
            child_id: child_id.clone(),
            event: AgentBoardEvent::Execution {
                execution: child_execution(
                    observation,
                    payload,
                    &root_id,
                    &child_id,
                    AgentTurnState::Working,
                    None,
                ),
            },
        });
    }
    Ok(events)
}

fn observed_child_finished(
    payload: &Value,
    observation: Option<NativeObservationProvider>,
) -> Result<Vec<AgentEventKind>, ProviderError> {
    let child_id = child_id(payload)
        .ok_or_else(|| ProviderError::InvalidPayload("child id is required".to_string()))?;
    let outcome = child_outcome(payload);
    let mut events = Vec::with_capacity(2);
    if let Some(observation) = observation
        && let Some(root_id) = observation.root_execution_id(payload)
    {
        events.push(AgentEventKind::ChildBoard {
            child_id: child_id.clone(),
            event: AgentBoardEvent::Execution {
                execution: child_execution(
                    observation,
                    payload,
                    &root_id,
                    &child_id,
                    outcome_state(outcome),
                    Some(0),
                ),
            },
        });
    }
    events.push(AgentEventKind::ChildFinished { child_id, outcome });
    Ok(events)
}

fn root_execution(
    observation: NativeObservationProvider,
    payload: &Value,
    task: Option<String>,
    state: AgentTurnState,
    ended_at: Option<u64>,
) -> Option<AgentBoardEvent> {
    let id = observation.root_execution_id(payload)?;
    Some(AgentBoardEvent::Execution {
        execution: AgentExecutionRecord {
            id,
            parent_id: None,
            name: observation.name().to_string(),
            task,
            telemetry: telemetry(payload),
            current_action: None,
            state,
            started_at: 0,
            updated_at: 0,
            ended_at,
        },
    })
}

fn child_execution(
    observation: NativeObservationProvider,
    payload: &Value,
    root_id: &str,
    child_id: &str,
    state: AgentTurnState,
    ended_at: Option<u64>,
) -> AgentExecutionRecord {
    AgentExecutionRecord {
        id: child_execution_id(root_id, child_id),
        parent_id: Some(root_id.to_string()),
        name: string_field(
            payload,
            &[
                "agent_type",
                "agentType",
                "subagentType",
                "subagent_type",
                "name",
            ],
        )
        .unwrap_or_else(|| observation.name().to_string()),
        task: string_field(payload, &["description", "task", "prompt"]),
        telemetry: telemetry(payload),
        current_action: None,
        state,
        started_at: 0,
        updated_at: 0,
        ended_at,
    }
}

fn append_agent_result_telemetry(
    events: &mut Vec<AgentEventKind>,
    observation: NativeObservationProvider,
    payload: &Value,
) {
    if string_field(payload, &["tool_name", "toolName", "name", "tool"])
        .is_none_or(|name| name != "Agent")
    {
        return;
    }
    let Some(response) = payload
        .get("tool_response")
        .or_else(|| payload.get("toolResponse"))
    else {
        return;
    };
    let Some(child_id) = string_field(response, &["agentId", "agent_id"]) else {
        return;
    };
    let Some(root_id) = observation.root_execution_id(payload) else {
        return;
    };
    events.push(AgentEventKind::Board {
        event: AgentBoardEvent::ExecutionTelemetry {
            execution_id: child_execution_id(&root_id, &child_id),
            telemetry: telemetry(response),
        },
    });
}

fn confirmed_task_delta(payload: &Value) -> Option<AgentPlanDelta> {
    let tool_name = string_field(payload, &["tool_name", "toolName", "name", "tool"])?;
    let input = payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))?;
    let response = payload
        .get("tool_response")
        .or_else(|| payload.get("toolResponse"));
    if response.is_some_and(|response| {
        bool_field(response, &["success"]).is_some_and(|success| !success)
            || bool_field(response, &["is_error", "isError", "failed"]) == Some(true)
    }) {
        return None;
    }
    match tool_name.as_str() {
        "TaskCreate" => {
            let task = response?.get("task")?;
            Some(AgentPlanDelta {
                id: string_field(task, &["id"])?,
                title: string_field(input, &["subject"])
                    .or_else(|| string_field(task, &["subject"])),
                phase: None,
                status: Some(AgentPlanStatus::Pending),
                blocker: None,
                retired: None,
            })
        }
        "TaskUpdate" => {
            let status_name = string_field(input, &["status"]);
            let retired = (status_name.as_deref() == Some("deleted")).then_some(true);
            Some(AgentPlanDelta {
                id: string_field(input, &["taskId", "id", "task_id"])?,
                title: string_field(input, &["subject"]),
                phase: string_field(input, &["activeForm", "active_form"]),
                status: status_name.and_then(|status| match status.as_str() {
                    "pending" => Some(AgentPlanStatus::Pending),
                    "in_progress" => Some(AgentPlanStatus::InProgress),
                    "completed" => Some(AgentPlanStatus::Completed),
                    "blocked" => Some(AgentPlanStatus::Blocked),
                    "abandoned" => Some(AgentPlanStatus::Abandoned),
                    _ => None,
                }),
                blocker: None,
                retired,
            })
        }
        _ => None,
    }
}

fn telemetry(payload: &Value) -> AgentExecutionTelemetry {
    let usage = payload
        .get("usage")
        .or_else(|| {
            payload
                .get("tool_response")
                .and_then(|response| response.get("usage"))
        })
        .or_else(|| {
            payload
                .get("toolResponse")
                .and_then(|response| response.get("usage"))
        });
    let response = payload
        .get("tool_response")
        .or_else(|| payload.get("toolResponse"))
        .unwrap_or(payload);
    AgentExecutionTelemetry {
        model: string_field(response, &["resolvedModel", "resolved_model", "model"])
            .or_else(|| string_field(payload, &["model", "model_id", "modelId"])),
        tokens: u64_field(response, &["totalTokens", "total_tokens"])
            .or_else(|| u64_field(payload, &["tokens", "totalTokens", "total_tokens"])),
        input_tokens: usage.and_then(|usage| u64_field(usage, &["input_tokens", "inputTokens"])),
        output_tokens: usage.and_then(|usage| u64_field(usage, &["output_tokens", "outputTokens"])),
        cache_read_tokens: usage
            .and_then(|usage| u64_field(usage, &["cache_read_input_tokens", "cache_read_tokens"])),
        cache_write_tokens: usage.and_then(|usage| {
            u64_field(
                usage,
                &["cache_creation_input_tokens", "cache_write_tokens"],
            )
        }),
        context_tokens: None,
        context_window: None,
        requests: None,
        tool_calls: u64_field(response, &["totalToolUseCount", "tool_calls"])
            .or_else(|| u64_field(payload, &["tool_calls", "totalToolUseCount"])),
        duration_ms: u64_field(response, &["totalDurationMs", "duration_ms"])
            .or_else(|| u64_field(payload, &["duration_ms", "totalDurationMs"])),
        cost_microusd: u64_field(payload, &["cost_microusd", "costMicrousd"]),
        self_estimated_completion: None,
        observed_at: u64_field(payload, &["observed_at", "observedAt"]),
    }
}

fn is_user_question(payload: &Value) -> bool {
    string_field(payload, &["tool_name", "toolName", "name", "tool"])
        .map(|name| {
            let normalized = name
                .chars()
                .filter(|character| character.is_ascii_alphanumeric())
                .flat_map(char::to_lowercase)
                .collect::<String>();
            matches!(
                normalized.as_str(),
                "askuserquestion" | "requestuserinput" | "askuser"
            )
        })
        .unwrap_or(false)
}

fn string_field(payload: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| {
        payload
            .get(*name)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(ToOwned::to_owned)
    })
}

fn bool_field(payload: &Value, names: &[&str]) -> Option<bool> {
    names
        .iter()
        .find_map(|name| payload.get(*name).and_then(Value::as_bool))
}

fn u64_field(payload: &Value, names: &[&str]) -> Option<u64> {
    names
        .iter()
        .find_map(|name| payload.get(*name).and_then(Value::as_u64))
}

fn tool_detail(value: &Value) -> Option<String> {
    if let Some(value) = value.as_str() {
        return non_empty(value);
    }
    let object = value.as_object()?;
    for name in [
        "path",
        "file_path",
        "command",
        "pattern",
        "query",
        "task",
        "prompt",
        "description",
        "url",
    ] {
        if let Some(value) = object.get(name).and_then(Value::as_str).and_then(non_empty) {
            return Some(value);
        }
    }
    serde_json::to_string(value)
        .ok()
        .and_then(|value| non_empty(&value))
}

fn non_empty(value: &str) -> Option<String> {
    let value = value.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yttt_agent_core::{AgentProvider as _, AgentTurnState};

    use super::*;

    #[test]
    fn recognizes_all_five_non_omp_commands() {
        assert!(CodexProvider.matches_command("/usr/local/bin/codex --model gpt"));
        assert!(ClaudeProvider.matches_command("claude --dangerously-skip-permissions"));
        assert!(GrokProvider.matches_command("grok --model grok-code-fast"));
        assert!(GrokProvider.matches_command("/usr/local/bin/groky"));
        assert!(OpenCodeProvider.matches_command(r"C:\\tools\\opencode"));
        assert!(PiProvider.matches_command("pi"));
        assert!(!PiProvider.matches_command("npm run pi"));
    }

    #[test]
    fn token_usage_is_open_code_only_among_non_omp_providers() {
        let payload = json!({
            "usage": {
                "id": "opencode:session-1:message:message-1",
                "parent_id": null,
                "tokens": 0,
                "includes_children": false,
                "partial": false
            }
        });
        let events = OpenCodeProvider
            .normalize_hook(ProviderHookEvent {
                name: "token_usage",
                payload: &payload,
            })
            .unwrap();
        assert!(matches!(
            &events[..],
            [AgentEventKind::TokenUsage { usage }]
                if usage.id == "opencode:session-1:message:message-1" && usage.tokens == 0
        ));

        for provider in [
            &ClaudeProvider as &dyn yttt_agent_core::AgentProvider,
            &CodexProvider,
            &GrokProvider,
            &PiProvider,
        ] {
            let error = provider
                .normalize_hook(ProviderHookEvent {
                    name: "token_usage",
                    payload: &payload,
                })
                .unwrap_err();
            assert!(matches!(error, ProviderError::UnsupportedEvent(_)));
        }
    }

    #[test]
    fn claude_maps_prompt_tool_wait_and_stop() {
        let prompt = json!({ "prompt": "Fix the hook transport" });
        let events = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "UserPromptSubmit",
                payload: &prompt,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::TurnStarted { task: Some(task) }
                if task.title == "Fix the hook transport"
        ));

        let ask = json!({ "tool_name": "AskUserQuestion", "tool_use_id": "ask-1" });
        let events = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "PreToolUse",
                payload: &ask,
            })
            .unwrap();
        assert!(matches!(
            events.last(),
            Some(AgentEventKind::Waiting {
                reason: WaitingReason::UserInput,
                ..
            })
        ));

        let stop = json!({ "is_interrupt": true });
        let events = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "Stop",
                payload: &stop,
            })
            .unwrap();
        assert_eq!(
            events,
            vec![AgentEventKind::TurnFinished {
                outcome: TurnOutcome::Interrupted
            }]
        );
    }

    #[test]
    fn root_permission_without_a_tool_id_does_not_replace_the_active_action() {
        let permission = json!({
            "session_id": "session-1",
            "prompt_id": "prompt-1",
            "tool_name": "Bash",
            "tool_input": { "command": "git status" }
        });
        let events = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "PermissionRequest",
                payload: &permission,
            })
            .unwrap();
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, AgentEventKind::ActionStarted { .. }))
        );
        assert!(matches!(
            &events[..],
            [
                AgentEventKind::Waiting {
                    reason: WaitingReason::Approval,
                    ..
                },
                AgentEventKind::Board {
                    event: AgentBoardEvent::ExecutionActivity {
                        current_action: None,
                        clear_current_action: false,
                        state: Some(AgentTurnState::Waiting),
                        ..
                    },
                },
            ]
        ));
    }

    #[test]
    fn codex_session_start_is_metadata_not_synthetic_work() {
        let payload = json!({ "session_id": "codex-1", "model": "gpt-5" });
        let events = CodexProvider
            .normalize_hook(ProviderHookEvent {
                name: "SessionStart",
                payload: &payload,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::SessionStarted { metadata }
                if metadata.session_id.as_deref() == Some("codex-1")
        ));
    }

    #[test]
    fn claude_child_hooks_update_only_child_observation_and_never_finish_on_tool_failure() {
        let root = json!({
            "session_id": "session-1",
            "prompt_id": "prompt-1",
            "prompt": "Inspect the native hooks"
        });
        let root_events = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "UserPromptSubmit",
                payload: &root,
            })
            .unwrap();
        assert!(root_events.iter().any(|event| matches!(
            event,
            AgentEventKind::Board {
                event: AgentBoardEvent::Execution { execution }
            } if execution.id == "claude:session-1:prompt-1"
        )));

        let child = json!({
            "session_id": "session-1",
            "prompt_id": "prompt-1",
            "agent_id": "agent-1",
            "agent_type": "Explore",
            "description": "Inspect adapters"
        });
        let started = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "SubagentStart",
                payload: &child,
            })
            .unwrap();
        assert!(matches!(started[0], AgentEventKind::ChildStarted { .. }));
        assert!(matches!(
            started[1],
            AgentEventKind::ChildBoard {
                event: AgentBoardEvent::Execution { ref execution },
                ..
            } if execution.parent_id.as_deref() == Some("claude:session-1:prompt-1")
        ));

        let tool = json!({
            "session_id": "session-1",
            "prompt_id": "prompt-1",
            "agent_id": "agent-1",
            "tool_name": "Read",
            "tool_use_id": "tool-1",
            "tool_input": { "file_path": "/tmp/repo/src/lib.rs" }
        });
        let active = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "PreToolUse",
                payload: &tool,
            })
            .unwrap();
        assert!(matches!(active[0], AgentEventKind::ChildUpdated { .. }));
        assert!(
            !active
                .iter()
                .any(|event| matches!(event, AgentEventKind::ActionStarted { .. }))
        );

        let waiting = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "PermissionRequest",
                payload: &tool,
            })
            .unwrap();
        assert!(matches!(
            &waiting[0],
            AgentEventKind::ChildUpdated { update, .. }
                if update.turn_state == Some(AgentTurnState::Waiting)
        ));
        let failed = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "PostToolUseFailure",
                payload: &tool,
            })
            .unwrap();
        assert!(matches!(
            &failed[0],
            AgentEventKind::ChildUpdated { update, .. } if update.clear_current_action
        ));
        assert!(
            !failed
                .iter()
                .any(|event| matches!(event, AgentEventKind::TurnFinished { .. }))
        );
        let stopped = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "SubagentStop",
                payload: &child,
            })
            .unwrap();
        assert!(matches!(
            stopped[0],
            AgentEventKind::ChildBoard {
                event: AgentBoardEvent::Execution { ref execution },
                ..
            } if execution.ended_at.is_some()
        ));
        assert!(matches!(stopped[1], AgentEventKind::ChildFinished { .. }));
        let result = json!({
            "session_id": "session-1",
            "prompt_id": "prompt-1",
            "tool_name": "Agent",
            "tool_response": {
                "agentId": "agent-1",
                "resolvedModel": "claude-model",
                "totalTokens": 17,
                "usage": { "input_tokens": 11 }
            }
        });
        let result_events = ClaudeProvider
            .normalize_hook(ProviderHookEvent {
                name: "PostToolUse",
                payload: &result,
            })
            .unwrap();
        assert!(result_events.iter().any(|event| matches!(
            event,
            AgentEventKind::Board {
                event: AgentBoardEvent::ExecutionTelemetry {
                    execution_id,
                    telemetry,
                },
            } if execution_id == "claude:session-1:prompt-1:child:agent-1"
                && telemetry.tokens == Some(17)
                && telemetry.input_tokens == Some(11)
                && telemetry.context_tokens.is_none()
                && telemetry.self_estimated_completion.is_none()
        )));
        assert!(
            !result_events
                .iter()
                .any(|event| matches!(event, AgentEventKind::TokenUsage { .. })),
            "Claude Agent tool totals describe one final request but have no stable non-overlapping billing subject",
        );
    }

    #[test]
    fn codex_turn_ids_create_new_run_identities_and_successful_task_results_emit_deltas() {
        let run = |turn_id| {
            let payload = json!({
                "session_id": "session-1",
                "turn_id": turn_id,
                "prompt": "Update the task plan"
            });
            CodexProvider
                .normalize_hook(ProviderHookEvent {
                    name: "UserPromptSubmit",
                    payload: &payload,
                })
                .unwrap()
                .into_iter()
                .find_map(|event| match event {
                    AgentEventKind::Board {
                        event: AgentBoardEvent::Execution { execution },
                    } => Some(execution.id),
                    _ => None,
                })
                .unwrap()
        };
        assert_ne!(run("turn-1"), run("turn-2"));

        let succeeded = json!({
            "session_id": "session-1",
            "turn_id": "turn-2",
            "tool_name": "TaskCreate",
            "tool_use_id": "tool-1",
            "tool_input": { "subject": "Verify telemetry" },
            "tool_response": { "task": { "id": "task-1", "subject": "Verify telemetry" } }
        });
        let events = CodexProvider
            .normalize_hook(ProviderHookEvent {
                name: "PostToolUse",
                payload: &succeeded,
            })
            .unwrap();
        assert!(matches!(
            events.last(),
            Some(AgentEventKind::Board {
                event: AgentBoardEvent::PlanDelta { delta, .. }
            }) if delta.id == "task-1" && delta.status == Some(AgentPlanStatus::Pending)
        ));

        let failed = CodexProvider
            .normalize_hook(ProviderHookEvent {
                name: "PostToolUseFailure",
                payload: &succeeded,
            })
            .unwrap();
        assert!(!failed.iter().any(|event| matches!(
            event,
            AgentEventKind::Board {
                event: AgentBoardEvent::PlanDelta { .. }
            }
        )));
    }

    #[test]
    fn opencode_delegates_structured_board_and_child_events_to_omp_normalization() {
        let board = json!({
            "board": {
                "kind": "execution",
                "execution": {
                    "id": "open-root",
                    "parent_id": null,
                    "name": "OpenCode",
                    "task": null,
                    "telemetry": { "tokens": 0 },
                    "current_action": null,
                    "state": "working",
                    "started_at": 1,
                    "updated_at": 1,
                    "ended_at": null
                }
            }
        });
        let events = OpenCodeProvider
            .normalize_hook(ProviderHookEvent {
                name: "board_event",
                payload: &board,
            })
            .unwrap();
        assert!(matches!(
            &events[..],
            [AgentEventKind::Board {
                event: AgentBoardEvent::Execution { execution }
            }] if execution.telemetry.tokens == Some(0)
        ));

        let child = json!({ "childId": "child-1", "status": "waiting" });
        let events = OpenCodeProvider
            .normalize_hook(ProviderHookEvent {
                name: "child_updated",
                payload: &child,
            })
            .unwrap();
        assert!(matches!(
            &events[..],
            [AgentEventKind::ChildUpdated { child_id, update }]
                if child_id == "child-1" && update.turn_state == Some(AgentTurnState::Waiting)
        ));
    }

    #[test]
    fn grok_maps_native_snake_case_hooks() {
        let session = json!({
            "hookEventName": "session_start",
            "hook_event_name": "session_start",
            "sessionId": "grok-1",
            "session_id": "grok-1",
            "workspaceRoot": "/tmp/project"
        });
        let events = GrokProvider
            .normalize_hook(ProviderHookEvent {
                name: "session_start",
                payload: &session,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::SessionStarted { metadata }
                if metadata.session_id.as_deref() == Some("grok-1")
        ));

        let events = GrokProvider
            .normalize_hook(ProviderHookEvent {
                name: "session_end",
                payload: &session,
            })
            .unwrap();
        assert_eq!(events, vec![AgentEventKind::SessionEnded]);

        let denied = json!({
            "tool_name": "Bash",
            "tool_input": { "command": "rm -rf build" }
        });
        let events = GrokProvider
            .normalize_hook(ProviderHookEvent {
                name: "permission_denied",
                payload: &denied,
            })
            .unwrap();
        assert_eq!(
            events,
            vec![AgentEventKind::ActionFinished {
                action_id: None,
                failed: true,
            }]
        );
    }

    #[test]
    fn grok_subagent_fields_support_native_and_snake_case_payloads() {
        for payload in [
            json!({"sessionId":"root", "subagentId":"child", "subagentType":"explore"}),
            json!({"session_id":"root", "subagent_id":"child", "subagent_type":"explore"}),
        ] {
            let started = GrokProvider
                .normalize_hook(ProviderHookEvent {
                    name: "subagent_start",
                    payload: &payload,
                })
                .unwrap();
            assert!(matches!(&started[0], AgentEventKind::ChildStarted { child }
                if child.id == "child" && child.name.as_deref() == Some("explore")));
            let stopped = GrokProvider
                .normalize_hook(ProviderHookEvent {
                    name: "subagent_stop",
                    payload: &payload,
                })
                .unwrap();
            assert!(
                matches!(&stopped[0], AgentEventKind::ChildFinished { child_id, .. } if child_id == "child")
            );
        }
    }

    #[test]
    fn snake_case_command_hooks_match_canonical_events() {
        let payload = json!({
            "session_id": "session-1",
            "prompt": "Inspect the lifecycle",
            "tool_name": "Bash",
            "tool_use_id": "tool-1",
            "agent_id": "child-1",
            "agent_type": "task",
            "description": "Inspect a child task"
        });
        for (canonical, native) in [
            ("SessionStart", "session_start"),
            ("SessionEnd", "session_end"),
            ("UserPromptSubmit", "user_prompt_submit"),
            ("PreToolUse", "pre_tool_use"),
            ("PermissionRequest", "permission_request"),
            ("PostToolUse", "post_tool_use"),
            ("PostToolUseFailure", "post_tool_use_failure"),
            ("PermissionDenied", "permission_denied"),
            ("Stop", "stop"),
            ("StopFailure", "stop_failure"),
            ("SubagentStart", "subagent_start"),
            ("SubagentStop", "subagent_stop"),
        ] {
            let canonical_events = GrokProvider
                .normalize_hook(ProviderHookEvent {
                    name: canonical,
                    payload: &payload,
                })
                .unwrap();
            let native_events = GrokProvider
                .normalize_hook(ProviderHookEvent {
                    name: native,
                    payload: &payload,
                })
                .unwrap();
            assert_eq!(native_events, canonical_events, "{native}");
        }
    }

    #[test]
    fn opencode_maps_normalized_plugin_events() {
        let payload = json!({ "prompt": "Review the state machine" });
        let events = OpenCodeProvider
            .normalize_hook(ProviderHookEvent {
                name: "user_prompt",
                payload: &payload,
            })
            .unwrap();
        assert!(matches!(events[0], AgentEventKind::TurnStarted { .. }));

        let payload = json!({ "toolName": "bash", "detail": "cargo test" });
        let events = OpenCodeProvider
            .normalize_hook(ProviderHookEvent {
                name: "tool_execution_start",
                payload: &payload,
            })
            .unwrap();
        assert!(matches!(events[0], AgentEventKind::ActionStarted { .. }));
        let events = OpenCodeProvider
            .normalize_hook(ProviderHookEvent {
                name: "session_error",
                payload: &payload,
            })
            .unwrap();
        assert!(events.is_empty());
    }

    #[test]
    fn pi_reuses_the_compatible_event_contract() {
        let payload = json!({ "status": "working" });
        let events = PiProvider
            .normalize_hook(ProviderHookEvent {
                name: "agent_start",
                payload: &payload,
            })
            .unwrap();
        assert_eq!(events, vec![AgentEventKind::Working]);
        let _ = AgentTurnState::Working;
    }
    #[test]
    fn builds_provider_specific_resume_commands() {
        let session = AgentSessionMetadata {
            session_id: Some("session-1".to_string()),
            model: None,
            title: None,
            transcript_path: Some("/tmp/pi-session.jsonl".to_string()),
        };
        let cases: [(&dyn AgentProvider, &str, &[&str]); 6] = [
            (&CodexProvider, "codex", &["resume", "session-1"]),
            (&ClaudeProvider, "claude", &["--resume", "session-1"]),
            (&GrokProvider, "grok", &["--resume", "session-1"]),
            (&OpenCodeProvider, "opencode", &["--session", "session-1"]),
            (&PiProvider, "pi", &["--session", "/tmp/pi-session.jsonl"]),
            (&OmpProvider, "omp", &["--resume", "session-1"]),
        ];

        for (provider, program, expected_arguments) in cases {
            let command = provider.resume_command(&session).unwrap();
            assert_eq!(command.program, program);
            assert_eq!(
                command.arguments,
                expected_arguments
                    .iter()
                    .map(|argument| argument.to_string())
                    .collect::<Vec<_>>()
            );
        }
    }

    #[test]
    fn opencode_session_updates_capture_identity_and_title() {
        let payload = json!({
            "id": "open-session-1",
            "title": "Refactor the auth middleware"
        });
        let events = OpenCodeProvider
            .normalize_hook(ProviderHookEvent {
                name: "session_updated",
                payload: &payload,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::SessionUpdated { metadata }
                if metadata.session_id.as_deref() == Some("open-session-1")
                    && metadata.title.as_deref() == Some("Refactor the auth middleware")
        ));
    }
    #[test]
    fn pi_session_start_and_explicit_switch_capture_the_resume_file() {
        let payload = json!({
            "sessionId": "pi-session-1",
            "sessionFile": "/tmp/pi-session.jsonl",
            "model": "model-1"
        });
        for name in ["session_start", "session_switch"] {
            let events = PiProvider
                .normalize_hook(ProviderHookEvent {
                    name,
                    payload: &payload,
                })
                .unwrap();
            assert!(matches!(
                &events[0],
                AgentEventKind::SessionStarted { metadata }
                    if metadata.session_id.as_deref() == Some("pi-session-1")
                        && metadata.transcript_path.as_deref() == Some("/tmp/pi-session.jsonl")
                        && metadata.model.as_deref() == Some("model-1")
            ));
        }
    }
}
