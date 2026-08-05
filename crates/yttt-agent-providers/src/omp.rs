use serde_json::Value;
use yttt_agent_core::{
    AgentAction, AgentEventKind, AgentProvider, AgentSessionMetadata, AgentTask, AgentTaskSource,
    AgentTurnState, ChildAgentDescriptor, ChildAgentUpdate, ProviderDescriptor, ProviderError,
    ProviderHookEvent, ProviderId, ProviderResumeCommand, TurnOutcome, WaitingReason,
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
        matches!(command_basename(command), Some("omp" | "oh-my-pi"))
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
            "session_start" => vec![AgentEventKind::SessionStarted {
                metadata: session_metadata(payload),
            }],
            "session_updated" => vec![AgentEventKind::SessionUpdated {
                metadata: session_metadata(payload),
            }],
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
                outcome: TurnOutcome::Completed,
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
                    turn_state: string_field(payload, "status")
                        .map(|status| child_turn_state(&status))
                        .transpose()?,
                },
            }],
            "child_finished" => vec![AgentEventKind::ChildFinished {
                child_id: required_string_field(payload, "childId")?,
                outcome: child_outcome(
                    string_field(payload, "outcome")
                        .as_deref()
                        .unwrap_or("completed"),
                )?,
            }],
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

fn child_outcome(value: &str) -> Result<TurnOutcome, ProviderError> {
    match value {
        "completed" => Ok(TurnOutcome::Completed),
        "failed" => Ok(TurnOutcome::Failed),
        "aborted" | "interrupted" => Ok(TurnOutcome::Interrupted),
        _ => Err(ProviderError::InvalidPayload(format!(
            "unsupported child outcome: {value}"
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

fn command_basename(command: &str) -> Option<&str> {
    let program = command.split_whitespace().next()?;
    program
        .rsplit(['/', '\\'])
        .find(|component| !component.is_empty())
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
}
