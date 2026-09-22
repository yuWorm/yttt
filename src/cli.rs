use std::{collections::BTreeMap, ffi::OsString, io::Read as _, path::PathBuf};
use yttt_protocol::desktop_control::*;

use crate::{
    config::profile::AppProfile,
    desktop_shell::{DesktopShellError, control_desktop},
};

pub const HELP: &str = r#"Usage: yttt ctl <resource> <action> [options]

  windows list
  projects list
  tabs list | create | focus | rename | close
  panes list | split | focus | rename | close | resize | send | read
  agents list | create | send

Targets:
  --window ID                  Window ID from windows list (required if ambiguous)
  --project ID                 Project ID from projects list
  --tab ID                     Terminal tab ID from tabs list
  --pane ID                    Pane ID within the tab

Create / manage:
  tabs create --project ID [--command 'cargo test'] [--title NAME]
  agents create --project ID --provider codex [--title NAME] [-- <provider arguments>]
  panes split --project ID --tab ID --pane ID --direction horizontal|vertical [--command CMD]
  panes resize --project ID --tab ID --pane ID --direction left|right|up|down [--percent 5]
  tabs|panes rename <targets> --title NAME
  tabs|panes close <targets>   Terminates processes in the target

Input:
  panes send <targets> --text TEXT [--enter] [--raw]
  agents send <targets> --text TEXT
  agents send <targets> --prompt-file PATH
  panes|agents send <targets> --stdin
  panes read <targets>        Reads the current terminal viewport

  --json                      Machine-readable response (errors go to stderr)
  --help                      Show this help

The desktop must already be running. Set YTTT_PROFILE_ROOT for a custom profile.
Lists cover open desktop windows and terminal tabs. Mutations select their target.
Created means the layout exists and startup was requested; list panes to check readiness.
InputAccepted means Host accepted the bytes, not that a command or Agent turn completed.
Agent send submits text plus Enter and rejects working/waiting/stale Agent states.
Use panes send for deliberate raw interaction with a busy Agent or an approval prompt.
Requests are never automatically retried. After a timeout inspect state before retrying.
"#;

#[derive(Debug)]
pub struct CliInvocation {
    pub request: DesktopControlRequest,
    pub json: bool,
}

pub fn parse(args: &[OsString]) -> Result<CliInvocation, String> {
    let words = args
        .iter()
        .map(|arg| {
            arg.to_str()
                .map(str::to_owned)
                .ok_or_else(|| "CLI arguments must be valid UTF-8".to_string())
        })
        .collect::<Result<Vec<_>, _>>()?;
    let resource = words
        .first()
        .ok_or("Expected a resource; see yttt ctl --help")?;
    let action = words
        .get(1)
        .ok_or("Expected an action; see yttt ctl --help")?;
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
            return Err(format!("Unexpected argument: {key}"));
        }
        let value = if matches!(key.as_str(), "--json" | "--enter" | "--raw" | "--stdin") {
            String::new()
        } else {
            index += 1;
            words
                .get(index)
                .ok_or_else(|| format!("Missing value for {key}"))?
                .clone()
        };
        if options.insert(key.clone(), value).is_some() {
            return Err(format!("Duplicate option: {key}"));
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
        ("tabs", "create") => DesktopControlCommand::CreateShell {
            command: options.remove("--command").unwrap_or_default(),
            title: options.remove("--title"),
        },
        ("agents", "create") => DesktopControlCommand::CreateAgent {
            provider: options
                .remove("--provider")
                .ok_or("--provider is required")?,
            args: std::mem::take(&mut trailing),
            title: options.remove("--title"),
        },
        ("panes", "split") => DesktopControlCommand::Split {
            direction: match options.remove("--direction").as_deref() {
                Some("horizontal") => ControlSplitDirection::Horizontal,
                Some("vertical") => ControlSplitDirection::Vertical,
                _ => return Err("--direction must be horizontal or vertical".into()),
            },
            command: options.remove("--command").unwrap_or_default(),
        },
        ("tabs" | "panes", "focus") => DesktopControlCommand::Focus,
        ("tabs" | "panes", "rename") => DesktopControlCommand::Rename {
            title: options.remove("--title").ok_or("--title is required")?,
        },
        ("tabs" | "panes", "close") => DesktopControlCommand::Close,
        ("panes", "resize") => DesktopControlCommand::Resize {
            direction: match options.remove("--direction").as_deref() {
                Some("left") => ControlResizeDirection::Left,
                Some("right") => ControlResizeDirection::Right,
                Some("up") => ControlResizeDirection::Up,
                Some("down") => ControlResizeDirection::Down,
                _ => return Err("--direction must be left, right, up or down".into()),
            },
            percent: options
                .remove("--percent")
                .map(|value| value.parse::<u8>())
                .transpose()
                .map_err(|_| "--percent must be a number between 1 and 90")?
                .unwrap_or(5),
        },
        ("panes" | "agents", "send") => {
            let text = options.remove("--text");
            let file = options.remove("--prompt-file");
            let stdin = options.remove("--stdin").is_some();
            if usize::from(text.is_some()) + usize::from(file.is_some()) + usize::from(stdin) != 1 {
                return Err("Supply exactly one of --text, --prompt-file or --stdin".into());
            }
            let text = if let Some(text) = text {
                text
            } else {
                let reader: Box<dyn std::io::Read> = if let Some(file) = file {
                    Box::new(
                        std::fs::File::open(PathBuf::from(file))
                            .map_err(|error| error.to_string())?,
                    )
                } else {
                    Box::new(std::io::stdin())
                };
                let mut text = String::new();
                reader
                    .take(MAX_CONTROL_TEXT_BYTES as u64 + 1)
                    .read_to_string(&mut text)
                    .map_err(|error| error.to_string())?;
                text
            };
            let enter = options.remove("--enter").is_some() || resource == "agents";
            let raw = options.remove("--raw").is_some();
            if resource == "agents" && raw {
                return Err("Use panes send --raw for raw terminal input".into());
            }
            DesktopControlCommand::Send {
                text,
                enter,
                raw,
                agent_only: resource == "agents",
            }
        }
        ("panes", "read") => DesktopControlCommand::Read,
        _ => {
            return Err(format!(
                "Unknown command: {resource} {action}; see yttt ctl --help"
            ));
        }
    };
    if let Some((key, _)) = options.first_key_value() {
        return Err(format!("Unknown option for this command: {key}"));
    }
    if !trailing.is_empty() {
        return Err("Provider arguments are only accepted by agents create".into());
    }
    if resource == "tabs" && pane.is_some() {
        return Err("Tab commands do not accept --pane".into());
    }
    if resource == "panes" && action != "list" && pane.is_none() {
        return Err("--pane is required".into());
    }
    let request = DesktopControlRequest {
        window,
        project,
        tab,
        pane,
        command,
    };
    request.validate().map_err(|error| error.message)?;
    Ok(CliInvocation { request, json })
}

pub fn run(profile: AppProfile, args: &[OsString]) -> i32 {
    if args.is_empty()
        || args
            .first()
            .is_some_and(|arg| arg == "--help" || arg == "-h")
    {
        println!("{HELP}");
        return 0;
    }
    let json = args
        .iter()
        .take_while(|arg| *arg != "--")
        .any(|arg| arg == "--json");
    let result = parse(args).map_err(|message| DesktopControlError::new(DesktopControlErrorCode::InvalidRequest, message))
        .and_then(|invocation| {
            let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build()
                .map_err(|error| DesktopControlError::new(DesktopControlErrorCode::Failed, error.to_string()))?;
            runtime.block_on(control_desktop(&profile, invocation.request))
                .map_err(|error| {
                    let code = match &error {
                        DesktopShellError::Transport(_) => DesktopControlErrorCode::NotReady,
                        DesktopShellError::Rejected(_) => DesktopControlErrorCode::InvalidRequest,
                        _ => DesktopControlErrorCode::OutcomeUnknown,
                    };
                    DesktopControlError::new(code, format!("{error}. Ensure the matching desktop is running; inspect state before retrying mutations"))
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
                println!("{}", format_response(&response));
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
                eprintln!("{:?}: {}", error.code, error.message);
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

fn target_label(target: &ControlTarget) -> String {
    format!(
        "window={} project={} tab={}{}",
        clean(&target.window),
        clean(&target.project),
        clean(&target.tab),
        target
            .pane
            .as_ref()
            .map(|pane| format!(" pane={}", clean(pane)))
            .unwrap_or_default()
    )
}

fn format_response(response: &DesktopControlResponse) -> String {
    match response {
        DesktopControlResponse::Windows(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{}\twritable={}\tloading={}\tproject={}",
                    clean(&item.id),
                    item.writable,
                    item.loading,
                    clean(item.selected_project.as_deref().unwrap_or("-"))
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
                format!(
                    "{}\t{}\tpanes={}",
                    target_label(&item.target),
                    clean(&item.title),
                    item.panes
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        DesktopControlResponse::Panes(items) | DesktopControlResponse::Agents(items) => items
            .iter()
            .map(|item| {
                format!(
                    "{}\t{}\t{}\t{}",
                    target_label(&item.target),
                    clean(&item.title),
                    clean(&item.state),
                    item.agent
                        .as_ref()
                        .map(|agent| agent.view_state().label())
                        .unwrap_or("-")
                )
            })
            .collect::<Vec<_>>()
            .join("\n"),
        DesktopControlResponse::Created { target, state } => {
            format!("Created {} ({state})", target_label(target))
        }
        DesktopControlResponse::Updated { target } => format!("Updated {}", target_label(target)),
        DesktopControlResponse::Closed { target } => format!("Closed {}", target_label(target)),
        DesktopControlResponse::InputAccepted { target, bytes } => {
            format!("Host accepted {bytes} bytes for {}", target_label(target))
        }
        DesktopControlResponse::Text { text, .. } => text.clone(),
    }
}
