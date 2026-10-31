use serde_json::Value;
use yttt_agent_core::{
    AgentAction, AgentBoardEvent, AgentEventKind, AgentProvider, AgentSessionMetadata, AgentTask,
    AgentTaskSource, AgentTokenUsage, AgentTurnState, ChildAgentDescriptor, ChildAgentUpdate,
    ProviderDescriptor, ProviderError, ProviderHookEvent, ProviderId, ProviderResumeCommand,
    TurnOutcome, WaitingReason,
};

pub const OMP_PROVIDER_ID: &str = "omp";
pub const OMP_EXTENSION_FILE_NAME: &str = "yttt-agent-extension.ts";
pub const OMP_EXTENSION_SOURCE: &str = include_str!("yttt-agent-extension.ts");

#[derive(Clone, Copy, Debug, Default)]
pub struct OmpProvider;

impl AgentProvider for OmpProvider {
    fn descriptor(&self) -> ProviderDescriptor {
        ProviderDescriptor {
            id: ProviderId::from_static(OMP_PROVIDER_ID),
            display_name: "Oh My Pi",
        }
    }

    fn matches_command(&self, command: &str) -> bool {
        crate::provider_for_command(command) == Some(OMP_PROVIDER_ID)
    }

    fn resume_command(&self, session: &AgentSessionMetadata) -> Option<ProviderResumeCommand> {
        let session_id = session.session_id.as_deref()?.trim();
        (!session_id.is_empty()).then(|| ProviderResumeCommand {
            program: OMP_PROVIDER_ID,
            arguments: vec!["--resume".to_string(), session_id.to_string()],
        })
    }

    fn normalize_hook(
        &self,
        event: ProviderHookEvent<'_>,
    ) -> Result<Vec<AgentEventKind>, ProviderError> {
        let payload = event.payload;
        let events = match event.name {
            "board_event" => vec![AgentEventKind::Board {
                event: serde_json::from_value::<AgentBoardEvent>(
                    payload.get("board").cloned().ok_or_else(|| {
                        ProviderError::InvalidPayload("board is required".to_string())
                    })?,
                )
                .map_err(|error| {
                    ProviderError::InvalidPayload(format!("invalid board event: {error}"))
                })?,
            }],
            "session_start" | "session_switch" => with_observed_activity(
                payload,
                AgentEventKind::SessionStarted {
                    metadata: session_metadata(payload),
                },
            ),
            "session_updated" => with_observed_activity(
                payload,
                AgentEventKind::SessionUpdated {
                    metadata: session_metadata(payload),
                },
            ),
            "session_shutdown" => vec![AgentEventKind::SessionEnded],
            "before_agent_start" => with_session_update(
                payload,
                AgentEventKind::TurnStarted {
                    task: string_field(payload, "prompt")
                        .and_then(|prompt| AgentTask::new(prompt, AgentTaskSource::UserPromptHook)),
                },
            ),
            "agent_start" => vec![AgentEventKind::Working],
            "agent_end" if bool_field(payload, "willContinue") == Some(true) => {
                vec![AgentEventKind::Working]
            }
            "agent_end" => vec![AgentEventKind::TurnFinished {
                outcome: turn_outcome(
                    string_field(payload, "outcome")
                        .as_deref()
                        .unwrap_or("completed"),
                )?,
            }],
            "tool_call" | "tool_execution_start" => action_started(payload)?,
            "tool_approval_requested" => vec![AgentEventKind::Waiting {
                reason: WaitingReason::Approval,
                message: string_field(payload, "reason")
                    .or_else(|| Some("Approval required".to_string())),
            }],
            "tool_approval_resolved" => vec![AgentEventKind::ActionFinished {
                action_id: string_field(payload, "toolCallId"),
                failed: bool_field(payload, "approved") == Some(false),
            }],
            "tool_execution_end" | "tool_result" => vec![AgentEventKind::ActionFinished {
                action_id: string_field(payload, "toolCallId"),
                failed: bool_field(payload, "isError").unwrap_or(false),
            }],
            "child_started" => vec![AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id: required_string_field(payload, "childId")?,
                    name: string_field(payload, "name"),
                    task: string_field(payload, "task")
                        .and_then(|task| AgentTask::new(task, AgentTaskSource::External)),
                },
            }],
            "child_updated" => vec![AgentEventKind::ChildUpdated {
                child_id: required_string_field(payload, "childId")?,
                update: ChildAgentUpdate {
                    task: string_field(payload, "task")
                        .and_then(|task| AgentTask::new(task, AgentTaskSource::External)),
                    current_action: string_field(payload, "action")
                        .and_then(|action| AgentAction::new(None, action, None)),
                    clear_current_action: bool_field(payload, "clearCurrentAction")
                        .unwrap_or(false),
                    turn_state: string_field(payload, "status")
                        .map(|status| child_turn_state(&status))
                        .transpose()?,
                },
            }],
            "child_finished" => vec![AgentEventKind::ChildFinished {
                child_id: required_string_field(payload, "childId")?,
                outcome: turn_outcome(
                    string_field(payload, "outcome")
                        .as_deref()
                        .unwrap_or("completed"),
                )?,
            }],
            "token_usage" => vec![token_usage(payload)?],
            name => return Err(ProviderError::UnsupportedEvent(name.to_string())),
        };
        Ok(events)
    }
}

fn session_metadata(payload: &Value) -> AgentSessionMetadata {
    AgentSessionMetadata {
        session_id: string_fields(payload, &["sessionId", "session_id"]),
        model: string_fields(payload, &["model", "modelId", "model_id"]),
        title: string_fields(
            payload,
            &[
                "title",
                "sessionTitle",
                "session_title",
                "customTitle",
                "custom_title",
            ],
        ),
        transcript_path: string_fields(
            payload,
            &[
                "sessionFile",
                "session_file",
                "transcriptPath",
                "transcript_path",
            ],
        ),
    }
}

fn with_observed_activity(payload: &Value, event: AgentEventKind) -> Vec<AgentEventKind> {
    let activity =
        bool_field(payload, "isIdle").map(|is_idle| AgentEventKind::Activity { is_idle });
    let mut events = Vec::with_capacity(1 + usize::from(activity.is_some()));
    events.push(event);
    events.extend(activity);
    events
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

fn string_fields(payload: &Value, names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| string_field(payload, name))
}

fn action_started(payload: &Value) -> Result<Vec<AgentEventKind>, ProviderError> {
    let name = string_field(payload, "toolName")
        .ok_or_else(|| ProviderError::InvalidPayload("toolName is required".to_string()))?;
    let action = AgentAction::new(
        string_field(payload, "toolCallId"),
        name,
        string_field(payload, "detail"),
    )
    .ok_or_else(|| ProviderError::InvalidPayload("toolName cannot be empty".to_string()))?;
    Ok(vec![AgentEventKind::ActionStarted { action }])
}

fn token_usage(payload: &Value) -> Result<AgentEventKind, ProviderError> {
    let usage = payload
        .get("usage")
        .ok_or_else(|| ProviderError::InvalidPayload("usage is required".to_string()))?;
    let parent_id = match usage.get("parent_id") {
        None | Some(Value::Null) => None,
        Some(Value::String(value)) => {
            let value = value.trim();
            if value.is_empty() {
                return Err(ProviderError::InvalidPayload(
                    "usage.parent_id cannot be empty".to_string(),
                ));
            }
            Some(value.to_string())
        }
        Some(_) => {
            return Err(ProviderError::InvalidPayload(
                "usage.parent_id must be a string or null".to_string(),
            ));
        }
    };
    let tokens = usage
        .get("tokens")
        .and_then(Value::as_u64)
        .ok_or_else(|| ProviderError::InvalidPayload("usage.tokens is required".to_string()))?;
    let includes_children = bool_field(usage, "includes_children").ok_or_else(|| {
        ProviderError::InvalidPayload("usage.includes_children is required".to_string())
    })?;
    let partial = bool_field(usage, "partial")
        .ok_or_else(|| ProviderError::InvalidPayload("usage.partial is required".to_string()))?;
    Ok(AgentEventKind::TokenUsage {
        usage: AgentTokenUsage {
            id: required_string_field(usage, "id")?,
            parent_id,
            tokens,
            includes_children,
            partial,
        },
    })
}
fn required_string_field(payload: &Value, name: &str) -> Result<String, ProviderError> {
    string_field(payload, name)
        .ok_or_else(|| ProviderError::InvalidPayload(format!("{name} is required")))
}

fn child_turn_state(value: &str) -> Result<AgentTurnState, ProviderError> {
    match value {
        "pending" | "idle" => Ok(AgentTurnState::Idle),
        "running" | "working" => Ok(AgentTurnState::Working),
        "waiting" => Ok(AgentTurnState::Waiting),
        "completed" => Ok(AgentTurnState::Completed),
        "failed" => Ok(AgentTurnState::Failed),
        "aborted" | "interrupted" => Ok(AgentTurnState::Interrupted),
        _ => Err(ProviderError::InvalidPayload(format!(
            "unsupported child status: {value}"
        ))),
    }
}

fn turn_outcome(value: &str) -> Result<TurnOutcome, ProviderError> {
    match value {
        "completed" => Ok(TurnOutcome::Completed),
        "failed" => Ok(TurnOutcome::Failed),
        "aborted" | "interrupted" => Ok(TurnOutcome::Interrupted),
        _ => Err(ProviderError::InvalidPayload(format!(
            "unsupported turn outcome: {value}"
        ))),
    }
}

fn string_field(payload: &Value, name: &str) -> Option<String> {
    payload
        .get(name)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn bool_field(payload: &Value, name: &str) -> Option<bool> {
    payload.get(name).and_then(Value::as_bool)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use yttt_agent_core::{AgentProvider, AgentTaskSource, WaitingReason};

    use super::*;

    #[test]
    fn recognizes_omp_commands() {
        let provider = OmpProvider;
        assert!(provider.matches_command("omp"));
        assert!(provider.matches_command("/opt/bin/omp --model opus"));
        assert!(provider.matches_command(r"C:\\tools\\omp"));
        assert!(!provider.matches_command("claude"));
    }

    #[test]
    fn normalizes_prompt_tool_wait_and_finish() {
        let provider = OmpProvider;
        let prompt = json!({ "prompt": "Implement OMP status" });
        let events = provider
            .normalize_hook(ProviderHookEvent {
                name: "before_agent_start",
                payload: &prompt,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::TurnStarted {
                task: Some(AgentTask {
                    source: AgentTaskSource::UserPromptHook,
                    ..
                })
            }
        ));

        let approval = json!({ "toolCallId": "1", "toolName": "edit" });
        let events = provider
            .normalize_hook(ProviderHookEvent {
                name: "tool_approval_requested",
                payload: &approval,
            })
            .unwrap();
        assert!(matches!(
            events[0],
            AgentEventKind::Waiting {
                reason: WaitingReason::Approval,
                ..
            }
        ));

        let end = json!({ "willContinue": false });
        let events = provider
            .normalize_hook(ProviderHookEvent {
                name: "agent_end",
                payload: &end,
            })
            .unwrap();
        assert_eq!(
            events,
            vec![AgentEventKind::TurnFinished {
                outcome: TurnOutcome::Completed
            }]
        );

        let cancelled = json!({ "willContinue": false, "outcome": "interrupted" });
        assert_eq!(
            provider
                .normalize_hook(ProviderHookEvent {
                    name: "agent_end",
                    payload: &cancelled
                })
                .unwrap(),
            vec![AgentEventKind::TurnFinished {
                outcome: TurnOutcome::Interrupted
            }]
        );

        let events = provider
            .normalize_hook(ProviderHookEvent {
                name: "session_shutdown",
                payload: &Value::Null,
            })
            .unwrap();
        assert_eq!(events, vec![AgentEventKind::SessionEnded]);
    }

    #[test]
    fn normalizes_explicit_session_switch_as_new_root_session() {
        let payload = json!({ "sessionId": "resumed-root" });
        let events = OmpProvider
            .normalize_hook(ProviderHookEvent {
                name: "session_switch",
                payload: &payload,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::SessionStarted { metadata }
                if metadata.session_id.as_deref() == Some("resumed-root")
        ));
    }

    #[test]
    fn normalizes_child_agent_lifecycle() {
        let provider = OmpProvider;
        let started = json!({
            "childId": "child-1",
            "name": "Reviewer",
            "task": "Review runtime mapping"
        });
        let events = provider
            .normalize_hook(ProviderHookEvent {
                name: "child_started",
                payload: &started,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::ChildStarted {
                child: ChildAgentDescriptor {
                    id,
                    name: Some(name),
                    task: Some(_),
                }
            } if id == "child-1" && name == "Reviewer"
        ));

        let updated = json!({
            "childId": "child-1",
            "task": "Review runtime mapping",
            "action": "read",
            "status": "running"
        });
        let events = provider
            .normalize_hook(ProviderHookEvent {
                name: "child_updated",
                payload: &updated,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::ChildUpdated {
                child_id,
                update: ChildAgentUpdate {
                    turn_state: Some(AgentTurnState::Working),
                    ..
                }
            } if child_id == "child-1"
        ));

        let cleared = json!({
            "childId": "child-1",
            "clearCurrentAction": true,
            "status": "working"
        });
        let events = provider
            .normalize_hook(ProviderHookEvent {
                name: "child_updated",
                payload: &cleared,
            })
            .unwrap();
        assert!(matches!(
            &events[0],
            AgentEventKind::ChildUpdated {
                update: ChildAgentUpdate {
                    current_action: None,
                    clear_current_action: true,
                    ..
                },
                ..
            }
        ));

        let finished = json!({ "childId": "child-1", "outcome": "failed" });
        let events = provider
            .normalize_hook(ProviderHookEvent {
                name: "child_finished",
                payload: &finished,
            })
            .unwrap();
        assert_eq!(
            events,
            vec![AgentEventKind::ChildFinished {
                child_id: "child-1".to_string(),
                outcome: TurnOutcome::Failed,
            }]
        );
    }

    #[test]
    fn normalizes_nested_token_usage_without_synthesizing_missing_values() {
        let payload = json!({
            "usage": {
                "id": "omp:session-1:message:entry-1",
                "parent_id": null,
                "tokens": 0,
                "includes_children": false,
                "partial": false
            }
        });
        let events = OmpProvider
            .normalize_hook(ProviderHookEvent {
                name: "token_usage",
                payload: &payload,
            })
            .unwrap();
        assert!(matches!(
            &events[..],
            [AgentEventKind::TokenUsage { usage }]
                if usage.id == "omp:session-1:message:entry-1"
                    && usage.parent_id.is_none()
                    && usage.tokens == 0
                    && !usage.includes_children
                    && !usage.partial
        ));

        let missing_tokens = json!({
            "usage": {
                "id": "omp:session-1:message:entry-2",
                "parent_id": null,
                "includes_children": false,
                "partial": false
            }
        });
        let error = OmpProvider
            .normalize_hook(ProviderHookEvent {
                name: "token_usage",
                payload: &missing_tokens,
            })
            .unwrap_err();
        assert!(matches!(error, ProviderError::InvalidPayload(_)));
    }
    #[test]
    fn normalizes_structured_board_events() {
        let payload = json!({
            "board": {
                "kind": "message",
                "message": {
                    "id": "message-1",
                    "execution_id": "root-1",
                    "role": "assistant",
                    "text": "Done",
                    "timestamp": 42,
                    "truncated": false
                }
            }
        });
        let events = OmpProvider
            .normalize_hook(ProviderHookEvent {
                name: "board_event",
                payload: &payload,
            })
            .unwrap();
        assert!(matches!(
            &events[..],
            [AgentEventKind::Board {
                event: AgentBoardEvent::Message { message }
            }] if message.id == "message-1" && message.execution_id == "root-1"
        ));

        let error = OmpProvider
            .normalize_hook(ProviderHookEvent {
                name: "board_event",
                payload: &json!({ "board": { "kind": "not_real" } }),
            })
            .unwrap_err();
        assert!(matches!(error, ProviderError::InvalidPayload(_)));
    }
}
