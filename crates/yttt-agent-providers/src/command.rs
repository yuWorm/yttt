use crate::OmpProvider;
use serde_json::Value;
use yttt_agent_core::{
    AgentAction, AgentEventKind, AgentProvider, AgentSessionMetadata, AgentTask, AgentTaskSource,
    ChildAgentDescriptor, ProviderDescriptor, ProviderError, ProviderHookEvent, ProviderId,
    ProviderResumeCommand, TurnOutcome, WaitingReason,
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
        command_basename(command) == Some("codex")
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        resume_with_session_id(CODEX_PROVIDER_ID, "resume", session)
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        normalize_command_hook(event)
    }
}

impl AgentProvider for ClaudeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(CLAUDE_PROVIDER_ID, "Claude Code")
    }

    fn matches_command(&self, command: &str) -> bool {
        matches!(command_basename(command), Some("claude" | "claude-code"))
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        resume_with_session_id(CLAUDE_PROVIDER_ID, "--resume", session)
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        normalize_command_hook(event)
    }
}

impl AgentProvider for GrokProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(GROK_PROVIDER_ID, "Grok")
    }

    fn matches_command(&self, command: &str) -> bool {
        matches!(command_basename(command), Some("grok" | "groky"))
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        resume_with_session_id(GROK_PROVIDER_ID, "--resume", session)
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        normalize_command_hook(event)
    }
}

impl AgentProvider for OpenCodeProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        descriptor(OPENCODE_PROVIDER_ID, "OpenCode")
    }

    fn matches_command(&self, command: &str) -> bool {
        matches!(command_basename(command), Some("opencode" | "open-code"))
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        resume_with_session_id(OPENCODE_PROVIDER_ID, "--session", session)
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
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
        command_basename(command) == Some("pi")
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
        OmpProvider.normalize_hook(event)
    }
}

fn normalize_command_hook(
    event: ProviderHookEvent<'_>,
) -> Result<Vec<AgentEventKind>, ProviderError> {
    let payload = event.payload;
    let events = match event.name {
        "SessionStart" => vec![session_started(payload)],
        "UserPromptSubmit" => {
            with_session_update(payload, turn_started(payload, &["prompt", "user_prompt"]))
        }
        "PreToolUse" if is_user_question(payload) => {
            waiting_events(payload, WaitingReason::UserInput)
        }
        "PreToolUse" => vec![action_started(payload)?],
        "PermissionRequest" => waiting_events(payload, WaitingReason::Approval),
        "PostToolUse" => vec![action_finished_with_failure(payload, false)],
        "PostToolUseFailure" => vec![action_finished_with_failure(payload, true)],
        "PermissionDenied" => vec![action_finished_with_failure(payload, true)],
        "Stop" => vec![AgentEventKind::TurnFinished {
            outcome: if bool_field(payload, &["is_interrupt", "interrupted"]) == Some(true) {
                TurnOutcome::Interrupted
            } else {
                TurnOutcome::Completed
            },
        }],
        "StopFailure" => vec![AgentEventKind::TurnFinished {
            outcome: TurnOutcome::Failed,
        }],
        "SubagentStart" => child_started(payload)?,
        "SubagentStop" => child_finished(payload)?,
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

fn action_started(payload: &Value) -> Result<AgentEventKind, ProviderError> {
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
    let action = AgentAction::new(id, name, detail)
        .ok_or_else(|| ProviderError::InvalidPayload("tool name cannot be empty".to_string()))?;
    Ok(AgentEventKind::ActionStarted { action })
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
    let id = string_field(payload, &["agent_id", "agentId", "child_id", "childId"])
        .ok_or_else(|| ProviderError::InvalidPayload("child id is required".to_string()))?;
    Ok(vec![AgentEventKind::ChildStarted {
        child: ChildAgentDescriptor {
            id,
            name: string_field(payload, &["agent_type", "agentType", "name"]),
            task: string_field(payload, &["description", "task", "prompt"])
                .and_then(|task| AgentTask::new(task, AgentTaskSource::External)),
        },
    }])
}

fn child_finished(payload: &Value) -> Result<Vec<AgentEventKind>, ProviderError> {
    let id = string_field(payload, &["agent_id", "agentId", "child_id", "childId"])
        .ok_or_else(|| ProviderError::InvalidPayload("child id is required".to_string()))?;
    Ok(vec![AgentEventKind::ChildFinished {
        child_id: id,
        outcome: if bool_field(payload, &["is_interrupt", "interrupted"]) == Some(true) {
            TurnOutcome::Interrupted
        } else if bool_field(payload, &["failed", "is_error", "isError"]) == Some(true) {
            TurnOutcome::Failed
        } else {
            TurnOutcome::Completed
        },
    }])
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

fn command_basename(command: &str) -> Option<&str> {
    command
        .split_whitespace()
        .next()?
        .rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
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
    fn grok_maps_native_camel_case_hooks() {
        let session = json!({
            "sessionId": "grok-1",
            "model": "grok-code-fast"
        });
        let events = GrokProvider
            .normalize_hook(ProviderHookEvent {
                name: "SessionStart",
                payload: &session,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::SessionStarted { metadata }
                if metadata.session_id.as_deref() == Some("grok-1")
                    && metadata.model.as_deref() == Some("grok-code-fast")
        ));

        let denied = json!({ "toolName": "Bash", "toolInput": { "command": "rm -rf build" } });
        let events = GrokProvider
            .normalize_hook(ProviderHookEvent {
                name: "PermissionDenied",
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
    fn pi_session_hook_captures_the_resume_file() {
        let payload = json!({
            "sessionId": "pi-session-1",
            "sessionFile": "/tmp/pi-session.jsonl",
            "model": "model-1"
        });
        let events = PiProvider
            .normalize_hook(ProviderHookEvent {
                name: "session_start",
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
