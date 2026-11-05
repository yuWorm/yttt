use std::{collections::BTreeMap, ffi::OsString, io::Read as _, path::PathBuf};
use yttt_core::model::ids::TerminalSessionId;
use yttt_core::model::layout::{PaneController, PaneRole};
use yttt_protocol::{
    agent::{
        AGENT_CONTROL_CLOSE_PATH, AGENT_CONTROL_INTERRUPT_PATH, AGENT_CONTROL_LIST_PATH,
        AGENT_CONTROL_RELEASE_PATH, AGENT_CONTROL_RESULT_PATH, AGENT_CONTROL_SEND_PATH,
        AGENT_CONTROL_SPAWN_PATH, AGENT_CONTROL_WAIT_PATH, AgentControlError,
        AgentControlErrorCode, AgentControlResponse, AgentInterruptRequest, AgentSendRequest,
        AgentSpawnRequest, AgentTargetRequest, AgentWaitCondition, AgentWaitReason,
        AgentWaitRequest, ControlledAgent, MAX_AGENT_WAIT_SECONDS,
    },
    desktop_control::*,
};

use crate::{
    agent_control_client::{PaneCredentials, WhoAmI},
    config::{profile::AppProfile, scope::read_device_language_setting, settings::LanguageSetting},
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

/// A command line split into resource, action, `--option value` pairs and
/// the words after `--`.
struct CommandWords {
    resource: String,
    action: String,
    options: BTreeMap<String, String>,
    trailing: Vec<String>,
}

const FLAG_OPTIONS: [&str; 7] = [
    "--json", "--enter", "--raw", "--stdin", "--mine", "--clear", "--queue",
];

fn split_command(args: &[OsString], text: UiText) -> Result<CommandWords, String> {
    let words = args
        .iter()
        .map(|arg| {
            arg.to_str().map(str::to_owned).ok_or_else(|| {
                text.get(UiTextKey::CliLocalizationArgumentsUtf8)
                    .to_string()
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let resource = words.first().ok_or_else(|| {
        text.get(UiTextKey::CliLocalizationExpectedResource)
            .to_string()
    })?;
    let action = words.get(1).ok_or_else(|| {
        text.get(UiTextKey::CliLocalizationExpectedAction)
            .to_string()
    })?;
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
        let value = if FLAG_OPTIONS.contains(&key.as_str()) {
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
    Ok(CommandWords {
        resource: resource.clone(),
        action: action.clone(),
        options,
        trailing,
    })
}

fn take_agent(
    options: &mut BTreeMap<String, String>,
    text: UiText,
) -> Result<TerminalSessionId, String> {
    options
        .remove("--agent")
        .map(TerminalSessionId::new)
        .ok_or_else(|| {
            text.get(UiTextKey::CliLocalizationAgentRequired)
                .to_string()
        })
}

const DEFAULT_WAIT_SECONDS: u32 = 300;
const DEFAULT_INTERRUPT_SECONDS: u32 = 10;

fn seconds(
    options: &mut BTreeMap<String, String>,
    default: u32,
    text: UiText,
) -> Result<u32, String> {
    options
        .remove("--timeout")
        .map(|value| value.parse::<u32>())
        .transpose()
        .map(|value| value.unwrap_or(default))
        .ok()
        .filter(|seconds| *seconds <= MAX_AGENT_WAIT_SECONDS)
        .ok_or_else(|| text.get(UiTextKey::CliLocalizationTimeoutRange).to_string())
}

/// The text of `--text`, `--prompt-file` or `--stdin`; exactly one is given.
fn input_text(options: &mut BTreeMap<String, String>, text: UiText) -> Result<String, String> {
    let text_value = options.remove("--text");
    let file = options.remove("--prompt-file");
    let stdin = options.remove("--stdin").is_some();
    if usize::from(text_value.is_some()) + usize::from(file.is_some()) + usize::from(stdin) != 1 {
        return Err(text
            .get(UiTextKey::CliLocalizationInputSourceExactlyOne)
            .to_string());
    }
    if let Some(text_value) = text_value {
        return Ok(text_value);
    }
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
    Ok(text_value)
}

/// A request an orchestrator pane sends straight to the Host.
#[derive(Debug)]
pub struct AgentControlInvocation {
    pub path: &'static str,
    pub body: serde_json::Value,
    pub json: bool,
}

/// Recognizes the commands served by the Host for orchestrators; any other
/// command line is left to [`parse`].
pub fn parse_agent_control(
    args: &[OsString],
    text: UiText,
) -> Result<Option<AgentControlInvocation>, String> {
    let Ok(CommandWords {
        resource,
        action,
        mut options,
        trailing,
    }) = split_command(args, text)
    else {
        return Ok(None);
    };
    let path = match (resource.as_str(), action.as_str()) {
        ("agents", "spawn") => AGENT_CONTROL_SPAWN_PATH,
        ("agents", "list") if options.contains_key("--mine") => AGENT_CONTROL_LIST_PATH,
        ("agents", "close") => AGENT_CONTROL_CLOSE_PATH,
        ("agents", "send") if options.contains_key("--agent") => AGENT_CONTROL_SEND_PATH,
        ("agents", "wait") => AGENT_CONTROL_WAIT_PATH,
        ("agents", "result") => AGENT_CONTROL_RESULT_PATH,
        ("agents", "interrupt") => AGENT_CONTROL_INTERRUPT_PATH,
        ("agents", "release") => AGENT_CONTROL_RELEASE_PATH,
        _ => return Ok(None),
    };

    let json = options.remove("--json").is_some();
    let body = match path {
        AGENT_CONTROL_SPAWN_PATH => serde_json::to_value(AgentSpawnRequest {
            provider: options.remove("--provider").ok_or_else(|| {
                text.get(UiTextKey::CliLocalizationProviderRequired)
                    .to_string()
            })?,
            args: trailing.clone(),
            title: options.remove("--title"),
        }),
        AGENT_CONTROL_CLOSE_PATH | AGENT_CONTROL_RESULT_PATH | AGENT_CONTROL_RELEASE_PATH => {
            serde_json::to_value(AgentTargetRequest {
                agent: take_agent(&mut options, text)?,
            })
        }
        AGENT_CONTROL_SEND_PATH => {
            let agent = take_agent(&mut options, text)?;
            let queue = options.remove("--queue").is_some();
            serde_json::to_value(AgentSendRequest {
                agent,
                text: input_text(&mut options, text)?,
                queue,
            })
        }
        AGENT_CONTROL_WAIT_PATH => {
            let agents = take_agent(&mut options, text)?
                .as_str()
                .split(',')
                .map(|agent| TerminalSessionId::new(agent.trim()))
                .collect();
            let until = match options.remove("--until").as_deref() {
                None | Some("finished") => AgentWaitCondition::Finished,
                Some("waiting") => AgentWaitCondition::Waiting,
                Some("ready") => AgentWaitCondition::Ready,
                Some("change") => AgentWaitCondition::Change,
                Some(_) => {
                    return Err(text
                        .get(UiTextKey::CliLocalizationWaitCondition)
                        .to_string());
                }
            };
            let after_turn = options
                .remove("--after-turn")
                .map(|value| value.parse::<u64>())
                .transpose()
                .map_err(|_| text.get(UiTextKey::CliLocalizationAfterTurn).to_string())?;
            serde_json::to_value(AgentWaitRequest {
                agents,
                until,
                after_turn,
                timeout_seconds: seconds(&mut options, DEFAULT_WAIT_SECONDS, text)?,
            })
        }
        AGENT_CONTROL_INTERRUPT_PATH => {
            let agent = take_agent(&mut options, text)?;
            serde_json::to_value(AgentInterruptRequest {
                agent,
                timeout_seconds: Some(seconds(&mut options, DEFAULT_INTERRUPT_SECONDS, text)?),
            })
        }
        _ => {
            options.remove("--mine");
            Ok(serde_json::json!({}))
        }
    }
    .expect("control requests serialize");
    if let Some((key, _)) = options.first_key_value() {
        return Err(text
            .get(UiTextKey::CliLocalizationUnknownOption)
            .replace("{option}", key));
    }
    if path != AGENT_CONTROL_SPAWN_PATH && !trailing.is_empty() {
        return Err(text
            .get(UiTextKey::CliLocalizationProviderArgumentsOnly)
            .to_string());
    }
    Ok(Some(AgentControlInvocation { path, body, json }))
}

pub fn parse(args: &[OsString], text: UiText) -> Result<CliInvocation, String> {
    let CommandWords {
        resource,
        action,
        mut options,
        mut trailing,
    } = split_command(args, text)?;
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
        ("agents", "control") => DesktopControlCommand::SetAgentController {
            controller: match options.remove("--set").as_deref() {
                Some("auto") => PaneController::Auto,
                Some("human") => PaneController::Human,
                Some("paused") => PaneController::Paused,
                _ => {
                    return Err(text
                        .get(UiTextKey::CliLocalizationControllerInvalid)
                        .to_string());
                }
            },
        },
        ("agents", "assign") => DesktopControlCommand::AssignAgent {
            owner: match (options.remove("--to"), options.remove("--clear")) {
                (Some(owner), None) => Some(owner),
                (None, Some(_)) => None,
                _ => {
                    return Err(text.get(UiTextKey::CliLocalizationAssignTarget).to_string());
                }
            },
        },
        ("board", "answer") => DesktopControlCommand::AnswerAgentQuestion {
            request_id: options.remove("--request").ok_or_else(|| {
                text.get(UiTextKey::CliLocalizationRequestRequired)
                    .to_string()
            })?,
            answers: serde_json::from_str(&options.remove("--answers-json").ok_or_else(|| {
                text.get(UiTextKey::CliLocalizationAnswersJsonRequired)
                    .to_string()
            })?)
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
                text.get(UiTextKey::CliLocalizationProviderRequired)
                    .to_string()
            })?,
            args: std::mem::take(&mut trailing),
            title: options.remove("--title"),
            role: match options.remove("--role").as_deref() {
                None | Some("worker") => PaneRole::Worker,
                Some("orchestrator") => PaneRole::Orchestrator,
                Some(_) => {
                    return Err(text.get(UiTextKey::CliLocalizationRoleInvalid).to_string());
                }
            },
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
                text.get(UiTextKey::CliLocalizationTitleRequired)
                    .to_string()
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
                .map_err(|_| text.get(UiTextKey::CliLocalizationPercentRange).to_string())?
                .unwrap_or(5),
        },
        ("panes" | "agents", "send") => {
            let text_value = input_text(&mut options, text)?;
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
                .replace("{resource}", &resource)
                .replace("{action}", &action));
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
        return Err(text.get(UiTextKey::CliLocalizationPaneRequired).to_string());
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
        DesktopControlErrorCode::PermissionDenied => {
            UiTextKey::CliLocalizationErrorPermissionDenied
        }
        DesktopControlErrorCode::Busy => UiTextKey::CliLocalizationErrorBusy,
        DesktopControlErrorCode::OutcomeUnknown => UiTextKey::CliLocalizationErrorOutcomeUnknown,
        DesktopControlErrorCode::Failed => UiTextKey::CliLocalizationErrorFailed,
    })
}

/// Sends an orchestrator's request to the Host; the desktop is not involved.
fn run_agent_control(invocation: &AgentControlInvocation, text: UiText) -> i32 {
    let output_text = if invocation.json {
        UiText::english()
    } else {
        text
    };
    let pane = PaneCredentials::from_env().filter(PaneCredentials::has_control_token);
    let Some(pane) = pane else {
        return report_agent_control_error(
            &AgentControlError::new(
                AgentControlErrorCode::Forbidden,
                output_text.get(UiTextKey::CliLocalizationOrchestratorRequired),
            ),
            invocation.json,
            text,
        );
    };
    match pane.control(invocation.path, &invocation.body) {
        Ok(response) => {
            if invocation.json {
                println!(
                    "{}",
                    serde_json::to_string(&response).expect("control response serializes")
                );
            } else {
                println!("{}", format_agent_control(&response, text));
            }
            0
        }
        Err(Some(error)) => report_agent_control_error(&error, invocation.json, text),
        Err(None) => {
            eprintln!(
                "{}: {}",
                error_code_label(&DesktopControlErrorCode::OutcomeUnknown, text),
                text.get(UiTextKey::CliLocalizationControlUnreachable)
            );
            1
        }
    }
}

fn report_agent_control_error(error: &AgentControlError, json: bool, text: UiText) -> i32 {
    if json {
        eprintln!(
            "{}",
            serde_json::to_string(error).expect("control error serializes")
        );
    } else {
        let code = match error.code {
            AgentControlErrorCode::InvalidRequest => DesktopControlErrorCode::InvalidRequest,
            AgentControlErrorCode::Forbidden => DesktopControlErrorCode::PermissionDenied,
            AgentControlErrorCode::NotFound => DesktopControlErrorCode::NotFound,
            AgentControlErrorCode::NotReady | AgentControlErrorCode::HumanControl => {
                DesktopControlErrorCode::Busy
            }
            AgentControlErrorCode::Failed => DesktopControlErrorCode::Failed,
        };
        eprintln!("{}: {}", error_code_label(&code, text), error.message);
    }
    if error.code == AgentControlErrorCode::InvalidRequest {
        2
    } else {
        1
    }
}

fn format_agent_control(response: &AgentControlResponse, text: UiText) -> String {
    let line = |agent: &ControlledAgent| {
        format!(
            "{}\t{}\t{}\t{}\t{}\t{}",
            agent.agent.as_str(),
            agent.title,
            agent.pane_state,
            protocol_word(&agent.controller),
            agent.queued,
            agent.agent_state.as_ref().map_or_else(
                || "-".to_string(),
                |state| format!("{} #{}", protocol_word(&state.turn), state.turn_seq)
            )
        )
    };
    match response {
        AgentControlResponse::Spawned { agent } => line(agent),
        AgentControlResponse::Agents { agents } if agents.is_empty() => text
            .get(UiTextKey::CliLocalizationNoControlledAgents)
            .to_string(),
        AgentControlResponse::Agents { agents } => {
            agents.iter().map(line).collect::<Vec<_>>().join("\n")
        }
        AgentControlResponse::Closed { agent } => text
            .get(UiTextKey::CliLocalizationAgentClosed)
            .replace("{agent}", agent.as_str()),
        AgentControlResponse::Sent { agent, queued, .. } if *queued => text
            .get(UiTextKey::CliLocalizationAgentQueued)
            .replace("{agent}", agent.as_str()),
        AgentControlResponse::Sent {
            agent,
            turn_seq_before,
            ..
        } => text
            .get(UiTextKey::CliLocalizationAgentSent)
            .replace("{agent}", agent.as_str())
            .replace("{turn}", &turn_seq_before.to_string()),
        AgentControlResponse::Released { agent } => text
            .get(UiTextKey::CliLocalizationAgentReleased)
            .replace("{agent}", agent.as_str()),
        AgentControlResponse::Waited {
            reason,
            agent,
            agents,
        } => {
            let headline = match (reason, agent) {
                (AgentWaitReason::TimedOut, _) | (_, None) => {
                    text.get(UiTextKey::CliLocalizationWaitTimedOut).to_string()
                }
                (AgentWaitReason::Matched, Some(agent)) => text
                    .get(UiTextKey::CliLocalizationWaitMatched)
                    .replace("{agent}", agent.as_str()),
                (AgentWaitReason::Exited, Some(agent)) => text
                    .get(UiTextKey::CliLocalizationWaitExited)
                    .replace("{agent}", agent.as_str()),
                (AgentWaitReason::ControlChanged, Some(agent)) => text
                    .get(UiTextKey::CliLocalizationWaitControlChanged)
                    .replace("{agent}", agent.as_str()),
            };
            std::iter::once(headline)
                .chain(agents.iter().map(line))
                .collect::<Vec<_>>()
                .join("\n")
        }
        AgentControlResponse::Result { result } => result.text.clone().unwrap_or_else(|| {
            text.get(UiTextKey::CliLocalizationNoAgentReply)
                .replace("{turn}", &result.turn_seq.to_string())
                .replace("{state}", &protocol_word(&result.turn))
        }),
        AgentControlResponse::Interrupted { agent, confirmed } => text
            .get(if *confirmed {
                UiTextKey::CliLocalizationAgentInterrupted
            } else {
                UiTextKey::CliLocalizationAgentInterruptUnconfirmed
            })
            .replace("{agent}", agent.as_str()),
    }
}

/// A protocol value as it appears in JSON, shown untranslated in text output.
fn protocol_word(value: &impl serde::Serialize) -> String {
    serde_json::to_value(value)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// Answers from the calling process's own terminal; the desktop is not involved.
fn whoami(args: &[OsString], text: UiText) -> i32 {
    let json = match args {
        [] => false,
        [flag] if flag == "--json" => true,
        [other, ..] => {
            eprintln!(
                "{}: {}",
                error_code_label(&DesktopControlErrorCode::InvalidRequest, text),
                text.get(UiTextKey::CliLocalizationUnexpectedArgument)
                    .replace("{argument}", &other.to_string_lossy())
            );
            return 2;
        }
    };
    let whoami = PaneCredentials::from_env().map_or(WhoAmI::External, |pane| pane.whoami());
    if json {
        println!(
            "{}",
            serde_json::to_string(&whoami).expect("whoami serializes")
        );
        return 0;
    }
    println!(
        "{}",
        match &whoami {
            WhoAmI::External => text
                .get(UiTextKey::CliLocalizationWhoamiExternal)
                .to_string(),
            WhoAmI::Pane {
                project_id,
                terminal_session_id,
                generation,
                role,
                control,
            } => text
                .get(UiTextKey::CliLocalizationWhoamiPane)
                .replace("{terminal}", terminal_session_id)
                .replace("{project}", project_id)
                .replace("{generation}", &generation.to_string())
                .replace("{role}", &protocol_word(role))
                .replace("{control}", &protocol_word(control)),
        }
    );
    0
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
    if args.first().is_some_and(|arg| arg == "whoami") {
        return whoami(&args[1..], text);
    }
    let json = args
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json");
    match parse_agent_control(args, if json { UiText::english() } else { text }) {
        Ok(Some(invocation)) => return run_agent_control(&invocation, text),
        Ok(None) => {}
        Err(message) => {
            return report_agent_control_error(
                &AgentControlError::new(AgentControlErrorCode::InvalidRequest, message),
                json,
                text,
            );
        }
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

fn control_state_label(state: &str, text: UiText) -> &str {
    text.get(match state {
        "idle" => UiTextKey::TerminalI18nLifecycleIdle,
        "starting" => UiTextKey::TerminalI18nLifecycleStarting,
        "running" => UiTextKey::TerminalI18nLifecycleRunning,
        "stopping" => UiTextKey::TerminalI18nLifecycleStopping,
        "exited" => UiTextKey::TerminalI18nLifecycleExited,
        "failed" => UiTextKey::TerminalI18nLifecycleSpawnFailed,
        "lost" => UiTextKey::TerminalI18nLifecycleLost,
        "pending" => UiTextKey::CliLocalizationStatePending,
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
