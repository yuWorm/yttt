use serde_json::Value;
use yttt_agent_core::{
    AgentAction, AgentEventKind, AgentProvider, AgentSessionMetadata, AgentTask, AgentTaskSource,
    ChildAgentDescriptor, ProviderDescriptor, ProviderError, ProviderHookEvent, ProviderId,
    TurnOutcome, WaitingReason,
};
use yttt_agent_omp::OmpProvider;

pub const CODEX_PROVIDER_ID: &str = "codex";
pub const CLAUDE_PROVIDER_ID: &str = "claude";
pub const OPENCODE_PROVIDER_ID: &str = "opencode";
pub const PI_PROVIDER_ID: &str = "pi";

#[derive(Clone, Copy, Debug, Default)]
pub struct CodexProvider;

#[derive(Clone, Copy, Debug, Default)]
pub struct ClaudeProvider;

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

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        let payload = event.payload;
        let events = match event.name {
            "session_start" => vec![session_started(payload)],
            "user_prompt" => vec![turn_started(payload, &["prompt", "text"])],
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
        "UserPromptSubmit" => vec![turn_started(payload, &["prompt", "user_prompt"])],
        "PreToolUse" if is_user_question(payload) => {
            waiting_events(payload, WaitingReason::UserInput)
        }
        "PreToolUse" => vec![action_started(payload)?],
        "PermissionRequest" => waiting_events(payload, WaitingReason::Approval),
        "PostToolUse" => vec![action_finished_with_failure(payload, false)],
        "PostToolUseFailure" => vec![action_finished_with_failure(payload, true)],
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
        metadata: AgentSessionMetadata {
            session_id: string_field(
                payload,
                &[
                    "session_id",
                    "sessionId",
                    "conversation_id",
                    "conversationId",
                ],
            ),
            model: string_field(payload, &["model", "model_id", "modelId"]),
        },
    }
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
    fn recognizes_all_four_non_omp_commands() {
        assert!(CodexProvider.matches_command("/usr/local/bin/codex --model gpt"));
        assert!(ClaudeProvider.matches_command("claude --dangerously-skip-permissions"));
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
}
