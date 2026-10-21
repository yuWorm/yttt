use std::{collections::BTreeMap, ffi::OsString, io::Read as _, path::PathBuf};
use yttt_protocol::desktop_control::*;

use crate::{
    config::{
        profile::AppProfile,
        scope::read_device_language_setting,
        settings::LanguageSetting,
    },
    desktop_shell::{DesktopShellError, control_desktop},
    ui::{
        i18n::{UiText, UiTextKey},
        terminal::status::agent_status_label,
    },
};

pub fn ui_text_for_profile(profile: &AppProfile) -> UiText {
    read_device_language_setting(profile)
        .map(UiText::from_language)
        .unwrap_or_else(|_| UiText::from_language(LanguageSetting::System))
}

fn help(text: UiText) -> &'static str {
    text.get(UiTextKey::CliLocalizationCtlHelp)
}

#[derive(Debug)]
pub struct CliInvocation {
    pub request: DesktopControlRequest,
    pub json: bool,
}

pub fn parse(args: &[OsString], text: UiText) -> Result<CliInvocation, String> {
    let words = args
        .iter()
        .map(|arg| {
            arg.to_str().map(str::to_owned).ok_or_else(|| {
                text.get(UiTextKey::CliLocalizationArgumentsUtf8).to_string()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let resource = words
        .first()
        .ok_or_else(|| text.get(UiTextKey::CliLocalizationExpectedResource).to_string())?;
    let action = words
        .get(1)
        .ok_or_else(|| text.get(UiTextKey::CliLocalizationExpectedAction).to_string())?;
    let mut options = BTreeMap::new();
    let mut trailing = Vec::new();
    let mut index = 2;
    while index < words.len() {
        let key = &words[index];
        if key == "--" {
            trailing = words[index + 1..].to_vec();
            break;
        }
        if !key.starts_with("--") {
            return Err(text
                .get(UiTextKey::CliLocalizationUnexpectedArgument)
                .replace("{argument}", key));
        }
        let value = if matches!(key.as_str(), "--json" | "--enter" | "--raw" | "--stdin") {
            String::new()
        } else {
            index += 1;
            words
                .get(index)
                .ok_or_else(|| {
                    text.get(UiTextKey::CliLocalizationMissingOptionValue)
                        .replace("{option}", key)
                })?
                .clone()
        };
        if options.insert(key.clone(), value).is_some() {
            return Err(text
                .get(UiTextKey::CliLocalizationDuplicateOption)
                .replace("{option}", key));
        }
        index += 1;
    }
    let json = options.remove("--json").is_some();
    let window = options.remove("--window");
    let project = options.remove("--project");
    let tab = options.remove("--tab");
    let pane = options.remove("--pane");
    let command = match (resource.as_str(), action.as_str()) {
        ("windows", "list") => DesktopControlCommand::Windows,
        ("projects", "list") => DesktopControlCommand::Projects,
        ("tabs", "list") => DesktopControlCommand::Tabs,
        ("panes", "list") => DesktopControlCommand::Panes,
        ("agents", "list") => DesktopControlCommand::Agents,
        ("board", "show") => DesktopControlCommand::ShowCommandCenter,
        ("board", "answer") => DesktopControlCommand::AnswerAgentQuestion {
            request_id: options.remove("--request").ok_or_else(|| {
                text.get(UiTextKey::CliLocalizationRequestRequired).to_string()
            })?,
            answers: serde_json::from_str(
                &options.remove("--answers-json").ok_or_else(|| {
                    text.get(UiTextKey::CliLocalizationAnswersJsonRequired)
                        .to_string()
                })?,
            )
            .map_err(|error| {
                text.get(UiTextKey::CliLocalizationInvalidStructuredAnswers)
                    .replace("{error}", &error.to_string())
            })?,
        },
        ("tabs", "create") => DesktopControlCommand::CreateShell {
            command: options.remove("--command").unwrap_or_default(),
            title: options.remove("--title"),
        },
        ("agents", "create") => DesktopControlCommand::CreateAgent {
            provider: options.remove("--provider").ok_or_else(|| {
                text.get(UiTextKey::CliLocalizationProviderRequired).to_string()
            })?,
            args: std::mem::take(&mut trailing),
            title: options.remove("--title"),
        },
        ("panes", "split") => DesktopControlCommand::Split {
            direction: match options.remove("--direction").as_deref() {
                Some("horizontal") => ControlSplitDirection::Horizontal,
                Some("vertical") => ControlSplitDirection::Vertical,
                _ => {
                    return Err(text
                        .get(UiTextKey::CliLocalizationSplitDirection)
                        .to_string());
                }
            },
            command: options.remove("--command").unwrap_or_default(),
        },
        ("tabs" | "panes", "focus") => DesktopControlCommand::Focus,
        ("tabs" | "panes", "rename") => DesktopControlCommand::Rename {
            title: options.remove("--title").ok_or_else(|| {
                text.get(UiTextKey::CliLocalizationTitleRequired).to_string()
            })?,
        },
        ("tabs" | "panes", "close") => DesktopControlCommand::Close,
        ("panes", "resize") => DesktopControlCommand::Resize {
            direction: match options.remove("--direction").as_deref() {
                Some("left") => ControlResizeDirection::Left,
                Some("right") => ControlResizeDirection::Right,
                Some("up") => ControlResizeDirection::Up,
                Some("down") => ControlResizeDirection::Down,
                _ => {
                    return Err(text
                        .get(UiTextKey::CliLocalizationResizeDirection)
                        .to_string());
                }
            },
            percent: options
                .remove("--percent")
                .map(|value| value.parse::<u8>())
                .transpose()
                .map_err(|_| {
                    text.get(UiTextKey::CliLocalizationPercentRange)
                        .to_string()
                })?
                .unwrap_or(5),
        },
        ("panes" | "agents", "send") => {
            let text_value = options.remove("--text");
            let file = options.remove("--prompt-file");
            let stdin = options.remove("--stdin").is_some();
            if usize::from(text_value.is_some()) + usize::from(file.is_some()) + usize::from(stdin)
                != 1
            {
                return Err(text
                    .get(UiTextKey::CliLocalizationInputSourceExactlyOne)
                    .to_string());
            }
            let text_value = if let Some(text_value) = text_value {
                text_value
            } else {
                let reader: Box<dyn std::io::Read> = if let Some(file) = file {
                    Box::new(std::fs::File::open(PathBuf::from(file)).map_err(|error| {
                        text.get(UiTextKey::CliLocalizationPromptInputReadFailure)
                            .replace("{error}", &error.to_string())
                    })?)
                } else {
                    Box::new(std::io::stdin())
                };
                let mut text_value = String::new();
                reader
                    .take(MAX_CONTROL_TEXT_BYTES as u64 + 1)
                    .read_to_string(&mut text_value)
                    .map_err(|error| {
                        text.get(UiTextKey::CliLocalizationPromptInputReadFailure)
                            .replace("{error}", &error.to_string())
                    })?;
                text_value
            };
            let enter = options.remove("--enter").is_some() || resource == "agents";
            let raw = options.remove("--raw").is_some();
            if resource == "agents" && raw {
                return Err(text
                    .get(UiTextKey::CliLocalizationAgentRawInput)
                    .to_string());
            }
            DesktopControlCommand::Send {
                text: text_value,
                enter,
                raw,
                agent_only: resource == "agents",
            }
        }
        ("panes", "read") => DesktopControlCommand::Read,
        _ => {
            return Err(text
                .get(UiTextKey::CliLocalizationUnknownCommand)
                .replace("{resource}", resource)
                .replace("{action}", action));
        }
    };
    if let Some((key, _)) = options.first_key_value() {
        return Err(text
            .get(UiTextKey::CliLocalizationUnknownOption)
            .replace("{option}", key));
    }
    if !trailing.is_empty() {
        return Err(text
            .get(UiTextKey::CliLocalizationProviderArgumentsOnly)
            .to_string());
    }
    if resource == "tabs" && pane.is_some() {
        return Err(text
            .get(UiTextKey::CliLocalizationTabsRejectPane)
            .to_string());
    }
    if resource == "panes" && action != "list" && pane.is_none() {
        return Err(text
            .get(UiTextKey::CliLocalizationPaneRequired)
            .to_string());
    }
    let request = DesktopControlRequest {
        window,
        project,
        tab,
        pane,
        command,
    };
    request.validate().map_err(|error| {
        text.get(UiTextKey::CliLocalizationRequestValidationError)
            .replace("{error}", &error.message)
    })?;
    Ok(CliInvocation { request, json })
}

fn error_code_label(code: &DesktopControlErrorCode, text: UiText) -> &'static str {
    text.get(match code {
        DesktopControlErrorCode::InvalidRequest => UiTextKey::CliLocalizationErrorInvalidRequest,
        DesktopControlErrorCode::NotFound => UiTextKey::CliLocalizationErrorNotFound,
        DesktopControlErrorCode::AmbiguousTarget => UiTextKey::CliLocalizationErrorAmbiguousTarget,
        DesktopControlErrorCode::NotReady => UiTextKey::CliLocalizationErrorNotReady,
        DesktopControlErrorCode::PermissionDenied => UiTextKey::CliLocalizationErrorPermissionDenied,
        DesktopControlErrorCode::Busy => UiTextKey::CliLocalizationErrorBusy,
        DesktopControlErrorCode::OutcomeUnknown => UiTextKey::CliLocalizationErrorOutcomeUnknown,
        DesktopControlErrorCode::Failed => UiTextKey::CliLocalizationErrorFailed,
    })
}

pub fn run(profile: AppProfile, args: &[OsString]) -> i32 {
    let text = ui_text_for_profile(&profile);
    if args.is_empty()
        || args
            .first()
            .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!("{}", help(text));
        return 0;
    }
    let json = args
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json");
    let output_text = if json { UiText::english() } else { text };
    let result = parse(args, output_text)
        .map_err(|message| {
            DesktopControlError::new(DesktopControlErrorCode::InvalidRequest, message)
        })
        .and_then(|invocation| {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .map_err(|error| {
                    DesktopControlError::new(
                        DesktopControlErrorCode::Failed,
                        output_text
                            .get(UiTextKey::CliLocalizationControlRuntimeFailure)
                            .replace("{error}", &error.to_string()),
                    )
                })?;
            runtime
                .block_on(control_desktop(&profile, invocation.request))
                .map_err(|error| {
                    let code = match &error {
                        DesktopShellError::Transport(_) => DesktopControlErrorCode::NotReady,
                        DesktopShellError::Rejected(_) => DesktopControlErrorCode::InvalidRequest,
                        _ => DesktopControlErrorCode::OutcomeUnknown,
                    };
                    DesktopControlError::new(
                        code,
                        output_text
                            .get(UiTextKey::CliLocalizationControlFailure)
                            .replace("{error}", &error.to_string()),
                    )
                })?
        });
    match result {
        Ok(response) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&response).expect("control response serializes")
                );
            } else {
                println!("{}", format_response(&response, text));
            }
            0
        }
        Err(error) => {
            if json {
                eprintln!(
                    "{}",
                    serde_json::to_string(&error).expect("control error serializes")
                );
            } else {
                eprintln!("{}: {}", error_code_label(&error.code, text), error.message);
            }
            if error.code == DesktopControlErrorCode::InvalidRequest {
                2
            } else {
                1
            }
        }
    }
}

fn clean(value: &str) -> String {
    value
        .chars()
        .flat_map(|c| {
            if c.is_control() {
                c.escape_default().collect::<Vec<_>>()
            } else {
                vec![c]
            }
        })
        .collect()
}

fn target_label(target: &ControlTarget, text: UiText) -> String {
    let pane = target
        .pane
        .as_ref()
        .map(|pane| {
            text.get(UiTextKey::CliLocalizationTargetPane)
                .replace("{pane}", &clean(pane))
        })
        .unwrap_or_default();
    text.get(UiTextKey::CliLocalizationTarget)
        .replace("{window}", &clean(&target.window))
        .replace("{project}", &clean(&target.project))
        .replace("{tab}", &clean(&target.tab))
        .replace("{pane}", &pane)
}

fn control_state_label<'a>(state: &'a str, text: UiText) -> &'a str {
    text.get(match state {
        "idle" => UiTextKey::TerminalI18nLifecycleIdle,
        "starting" => UiTextKey::TerminalI18nLifecycleStarting,
        "reconciling" => UiTextKey::TerminalI18nLifecycleReconciling,
        "running" => UiTextKey::TerminalI18nLifecycleRunning,
        "stopping" => UiTextKey::TerminalI18nLifecycleStopping,
        "exited" => UiTextKey::TerminalI18nLifecycleExited,
        "failed" => UiTextKey::TerminalI18nLifecycleSpawnFailed,
        "lost" => UiTextKey::TerminalI18nLifecycleLost,
        "restoring" => UiTextKey::CliLocalizationStateRestoring,
        "pending" => UiTextKey::CliLocalizationStatePending,
        "initializing" => UiTextKey::CliLocalizationStateInitializing,
        state => return state,
    })
}

fn format_response(response: &DesktopControlResponse, text: UiText) -> String {
    match response {
        DesktopControlResponse::Windows(items) => items
            .iter()
            .map(|item| {
                text.get(UiTextKey::CliLocalizationWindowItem)
                    .replace("{id}", &clean(&item.id))
                    .replace(
                        "{writable}",
                        text.get(if item.writable {
                            UiTextKey::CliLocalizationBooleanTrue
                        } else {
                            UiTextKey::CliLocalizationBooleanFalse
                        }),
                    )
                    .replace(
                        "{loading}",
                        text.get(if item.loading {
                            UiTextKey::CliLocalizationBooleanTrue
                        } else {
                            UiTextKey::CliLocalizationBooleanFalse
                        }),
                    )
                    .replace(
                        "{project}",
                        &clean(item.selected_project.as_deref().unwrap_or("-")),
                    )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        DesktopControlResponse::Projects(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{}\t{}\t{}\t{}",
                    clean(&item.window),
                    clean(&item.id),
                    clean(&item.name),
                    clean(&item.path)
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        DesktopControlResponse::Tabs(items) => items
            .iter()
            .map(|item| {
                text.get(UiTextKey::CliLocalizationTabItem)
                    .replace("{target}", &target_label(&item.target, text))
                    .replace("{title}", &clean(&item.title))
                    .replace("{panes}", &item.panes.to_string())
            })
            .collect::<Vec<_>>()
            .join("\n"),
        DesktopControlResponse::Panes(items) | DesktopControlResponse::Agents(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{}\t{}\t{}\t{}",
                    target_label(&item.target, text),
                    clean(&item.title),
                    clean(control_state_label(&item.state, text)),
                    item.agent
                        .as_ref()
                        .map(|agent| agent_status_label(agent.view_state(), &text))
                        .unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        DesktopControlResponse::Created { target, state } => text
            .get(UiTextKey::CliLocalizationCreated)
            .replace("{target}", &target_label(target, text))
            .replace("{state}", control_state_label(state, text)),
        DesktopControlResponse::Updated { target } => text
            .get(UiTextKey::CliLocalizationUpdated)
            .replace("{target}", &target_label(target, text)),
        DesktopControlResponse::Closed { target } => text
            .get(UiTextKey::CliLocalizationClosed)
            .replace("{target}", &target_label(target, text)),
        DesktopControlResponse::InputAccepted { target, bytes } => text
            .get(UiTextKey::CliLocalizationInputAccepted)
            .replace("{bytes}", &bytes.to_string())
            .replace("{target}", &target_label(target, text)),
        DesktopControlResponse::Text { text: output, .. } => output.clone(),
        DesktopControlResponse::CommandCenterShown { window } => text
            .get(UiTextKey::CliLocalizationCommandCenterShown)
            .replace("{window}", &clean(window)),
        DesktopControlResponse::AgentQuestionSubmitted { target, request_id } => text
            .get(UiTextKey::CliLocalizationAnswerQueued)
            .replace("{target}", &target_label(target, text))
            .replace("{request}", &clean(request_id)),
    }
}
