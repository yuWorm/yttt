//! `yttt mcp`: the Agent control commands as tools of a Model Context
//! Protocol server on stdio, for an orchestrator that prefers typed tools to
//! running `yttt ctl`. Each tool sends the same request the CLI would, with
//! the calling pane's own credentials, so the Host applies the same rules.

use std::io::{BufRead as _, Write as _};

use serde_json::{Value, json};
use yttt_protocol::agent::{
    AGENT_CONTROL_CLOSE_PATH, AGENT_CONTROL_INTERRUPT_PATH, AGENT_CONTROL_LIST_PATH,
    AGENT_CONTROL_RELEASE_PATH, AGENT_CONTROL_RESULT_PATH, AGENT_CONTROL_SEND_PATH,
    AGENT_CONTROL_SPAWN_PATH, AGENT_CONTROL_WAIT_PATH, AgentControlError, AgentControlErrorCode,
    AgentControlResponse, AgentInterruptRequest, AgentSendRequest, AgentSpawnRequest,
    AgentTargetRequest, AgentWaitRequest,
};

use crate::agent_control_client::{PaneCredentials, WhoAmI};

/// The protocol revision answered when a client asks for none.
const PROTOCOL_VERSION: &str = "2025-06-18";
/// MCP clients often stop a tool call after a few minutes; waits default to
/// less than the Host's limit and say so.
const DEFAULT_WAIT_SECONDS: u64 = 120;

/// How the server reaches the Host; a test supplies its own.
pub trait AgentControlTransport {
    fn whoami(&self) -> WhoAmI;
    fn control(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<AgentControlResponse, Option<AgentControlError>>;
}

/// The calling process's pane, if it runs in one.
pub struct PaneTransport(Option<PaneCredentials>);

impl AgentControlTransport for PaneTransport {
    fn whoami(&self) -> WhoAmI {
        self.0
            .as_ref()
            .map_or(WhoAmI::External, PaneCredentials::whoami)
    }

    fn control(
        &self,
        path: &str,
        body: &Value,
    ) -> Result<AgentControlResponse, Option<AgentControlError>> {
        match self.0.as_ref().filter(|pane| pane.has_control_token()) {
            Some(pane) => pane.control(path, body),
            None => Err(Some(AgentControlError::new(
                AgentControlErrorCode::Forbidden,
                "these tools work only in an orchestrator pane; see yttt_whoami",
            ))),
        }
    }
}

/// Serves MCP on stdin and stdout until stdin closes.
pub fn run() -> i32 {
    let transport = PaneTransport(PaneCredentials::from_env());
    let stdin = std::io::stdin();
    let mut stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else {
            return 1;
        };
        if line.trim().is_empty() {
            continue;
        }
        let response = match serde_json::from_str::<Value>(&line) {
            Ok(message) => handle_message(&message, &transport),
            Err(error) => Some(error_response(Value::Null, -32700, &error.to_string())),
        };
        if let Some(response) = response
            && writeln!(stdout, "{response}")
                .and_then(|()| stdout.flush())
                .is_err()
        {
            return 1;
        }
    }
    0
}

/// Answers one JSON-RPC message; notifications get no answer.
pub fn handle_message(message: &Value, transport: &dyn AgentControlTransport) -> Option<Value> {
    let id = message.get("id").cloned()?;
    let method = message["method"].as_str().unwrap_or_default();
    let params = &message["params"];
    let result = match method {
        "initialize" => json!({
            "protocolVersion": params["protocolVersion"].as_str().unwrap_or(PROTOCOL_VERSION),
            "capabilities": { "tools": {} },
            "serverInfo": { "name": "yttt", "version": env!("CARGO_PKG_VERSION") },
            "instructions": INSTRUCTIONS,
        }),
        "ping" => json!({}),
        "tools/list" => json!({ "tools": tools() }),
        "tools/call" => call_tool(
            params["name"].as_str().unwrap_or_default(),
            params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({})),
            transport,
        ),
        _ => {
            return Some(error_response(
                id,
                -32601,
                &format!("unknown method {method}"),
            ));
        }
    };
    Some(json!({ "jsonrpc": "2.0", "id": id, "result": result }))
}

const INSTRUCTIONS: &str = "Drive other Agents in yttt terminals a person can watch. \
Start workers with yttt_spawn_agent and give each a self-contained task. After yttt_send, \
call yttt_wait with after_turn set to the turn_seq_before it returned, then yttt_result. \
A control_changed wait or a human_control error means a person took a worker over: stop \
sending to it and continue with other work until they hand it back. A worker waiting for \
an approval needs a person; you cannot answer it.";

fn tools() -> Value {
    let agent = json!({
        "type": "string",
        "description": "A worker handle from yttt_spawn_agent or yttt_list_agents",
    });
    json!([
        tool(
            "yttt_whoami",
            "Who this pane is and whether it may orchestrate.",
            json!({})
        ),
        tool(
            "yttt_spawn_agent",
            "Start a worker Agent in a new tab of this project and return its handle.",
            json!({
                "provider": {
                    "type": "string",
                    "enum": ["codex", "claude", "grok", "groky", "opencode", "pi", "omp"],
                },
                "title": { "type": "string" },
                "args": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Passed to the provider unchanged, such as an initial prompt",
                },
            }),
        )
        .with_required(&["provider"]),
        tool(
            "yttt_list_agents",
            "The workers this orchestrator drives, with their turn, controller and queue.",
            json!({}),
        ),
        tool(
            "yttt_send",
            "Paste text into a ready worker and press Enter. Returns turn_seq_before for \
             yttt_wait. With queue, the Host holds it while the worker is busy or a person \
             drives it.",
            json!({
                "agent": agent,
                "text": { "type": "string" },
                "queue": { "type": "boolean" },
            }),
        )
        .with_required(&["agent", "text"]),
        tool(
            "yttt_wait",
            "Block until one worker finishes a turn after after_turn (or meets `until`), ends, \
             is taken over by a person, or the timeout passes. Call again to keep waiting.",
            json!({
                "agents": { "type": "array", "items": agent, "minItems": 1, "maxItems": 16 },
                "until": { "type": "string", "enum": ["finished", "waiting", "ready", "change"] },
                "after_turn": { "type": "integer", "minimum": 0 },
                "timeout_seconds": {
                    "type": "integer",
                    "minimum": 0,
                    "maximum": 600,
                    "description": "Defaults to 120",
                },
            }),
        )
        .with_required(&["agents"]),
        tool(
            "yttt_result",
            "A worker's last reply, read from its provider's transcript.",
            json!({ "agent": agent }),
        )
        .with_required(&["agent"]),
        tool(
            "yttt_interrupt",
            "Stop a working Claude Code or Codex worker's turn.",
            json!({
                "agent": agent,
                "timeout_seconds": { "type": "integer", "minimum": 0, "maximum": 600 },
            }),
        )
        .with_required(&["agent"]),
        tool(
            "yttt_release",
            "Hand a worker to a person; you stop driving it until they hand it back.",
            json!({ "agent": agent }),
        )
        .with_required(&["agent"]),
        tool(
            "yttt_close",
            "Close a worker's pane, ending its process.",
            json!({ "agent": agent }),
        )
        .with_required(&["agent"]),
    ])
}

fn tool(name: &str, description: &str, properties: Value) -> Value {
    json!({
        "name": name,
        "description": description,
        "inputSchema": {
            "type": "object",
            "properties": properties,
            "additionalProperties": false,
        },
    })
}

trait RequiredFields {
    fn with_required(self, fields: &[&str]) -> Value;
}

impl RequiredFields for Value {
    fn with_required(mut self, fields: &[&str]) -> Value {
        self["inputSchema"]["required"] = json!(fields);
        self
    }
}

fn call_tool(name: &str, mut arguments: Value, transport: &dyn AgentControlTransport) -> Value {
    if name == "yttt_whoami" {
        return tool_result(
            &serde_json::to_value(transport.whoami()).unwrap_or_default(),
            false,
        );
    }
    if name == "yttt_wait"
        && let Some(arguments) = arguments.as_object_mut()
    {
        arguments
            .entry("until")
            .or_insert_with(|| json!("finished"));
        arguments
            .entry("timeout_seconds")
            .or_insert_with(|| json!(DEFAULT_WAIT_SECONDS));
    }
    let request = match name {
        "yttt_spawn_agent" => body::<AgentSpawnRequest>(arguments, AGENT_CONTROL_SPAWN_PATH),
        "yttt_list_agents" => Ok((AGENT_CONTROL_LIST_PATH, json!({}))),
        "yttt_send" => body::<AgentSendRequest>(arguments, AGENT_CONTROL_SEND_PATH),
        "yttt_wait" => body::<AgentWaitRequest>(arguments, AGENT_CONTROL_WAIT_PATH),
        "yttt_result" => body::<AgentTargetRequest>(arguments, AGENT_CONTROL_RESULT_PATH),
        "yttt_interrupt" => body::<AgentInterruptRequest>(arguments, AGENT_CONTROL_INTERRUPT_PATH),
        "yttt_release" => body::<AgentTargetRequest>(arguments, AGENT_CONTROL_RELEASE_PATH),
        "yttt_close" => body::<AgentTargetRequest>(arguments, AGENT_CONTROL_CLOSE_PATH),
        _ => Err(AgentControlError::new(
            AgentControlErrorCode::InvalidRequest,
            format!("unknown tool {name}"),
        )),
    };
    let outcome = request
        .map_err(Some)
        .and_then(|(path, body)| transport.control(path, &body));
    match outcome {
        Ok(response) => tool_result(&serde_json::to_value(response).unwrap_or_default(), false),
        Err(Some(error)) => tool_result(&serde_json::to_value(error).unwrap_or_default(), true),
        Err(None) => tool_result(
            &json!({
                "code": "outcome_unknown",
                "message": "the Host did not answer; check yttt_list_agents before retrying",
            }),
            true,
        ),
    }
}

/// Checks arguments against the request type the Host expects.
fn body<T: serde::de::DeserializeOwned + serde::Serialize>(
    arguments: Value,
    path: &'static str,
) -> Result<(&'static str, Value), AgentControlError> {
    let request = serde_json::from_value::<T>(arguments).map_err(|error| {
        AgentControlError::new(AgentControlErrorCode::InvalidRequest, error.to_string())
    })?;
    Ok((
        path,
        serde_json::to_value(request).expect("control requests serialize"),
    ))
}

fn tool_result(value: &Value, is_error: bool) -> Value {
    json!({
        "content": [{ "type": "text", "text": value.to_string() }],
        "structuredContent": value,
        "isError": is_error,
    })
}

fn error_response(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use yttt_core::model::{ids::TerminalSessionId, layout::PaneRole};

    use super::*;
    use crate::agent_control_client::ControlAccess;

    /// Records what the server asked the Host and answers with `reply`.
    struct FakeHost {
        sent: RefCell<Vec<(String, Value)>>,
        reply: Result<AgentControlResponse, Option<AgentControlError>>,
    }

    impl AgentControlTransport for FakeHost {
        fn whoami(&self) -> WhoAmI {
            WhoAmI::Pane {
                project_id: "project".into(),
                terminal_session_id: "project:lead:lead".into(),
                generation: 1,
                role: PaneRole::Orchestrator,
                control: ControlAccess::Verified,
            }
        }

        fn control(
            &self,
            path: &str,
            body: &Value,
        ) -> Result<AgentControlResponse, Option<AgentControlError>> {
            self.sent
                .borrow_mut()
                .push((path.to_string(), body.clone()));
            self.reply.clone()
        }
    }

    fn host(reply: Result<AgentControlResponse, Option<AgentControlError>>) -> FakeHost {
        FakeHost {
            sent: RefCell::new(Vec::new()),
            reply,
        }
    }

    fn closed() -> AgentControlResponse {
        AgentControlResponse::Closed {
            agent: TerminalSessionId::new("project:w:agent"),
        }
    }

    fn call(host: &FakeHost, name: &str, arguments: Value) -> Value {
        handle_message(
            &json!({
                "jsonrpc": "2.0",
                "id": 7,
                "method": "tools/call",
                "params": { "name": name, "arguments": arguments },
            }),
            host,
        )
        .unwrap()["result"]
            .clone()
    }

    #[test]
    fn the_server_introduces_itself_and_lists_a_schema_for_every_tool() {
        let host = host(Ok(closed()));
        let initialized = handle_message(
            &json!({"jsonrpc": "2.0", "id": 1, "method": "initialize",
                    "params": {"protocolVersion": "2025-03-26"}}),
            &host,
        )
        .unwrap();
        assert_eq!(initialized["result"]["protocolVersion"], "2025-03-26");
        assert_eq!(initialized["result"]["serverInfo"]["name"], "yttt");
        assert!(initialized["result"]["capabilities"]["tools"].is_object());

        assert!(
            handle_message(
                &json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
                &host
            )
            .is_none()
        );
        let unknown = handle_message(
            &json!({"jsonrpc": "2.0", "id": 2, "method": "resources/list"}),
            &host,
        )
        .unwrap();
        assert_eq!(unknown["error"]["code"], -32601);

        let listed = handle_message(
            &json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"}),
            &host,
        )
        .unwrap();
        let tools = listed["result"]["tools"].as_array().unwrap();
        let names = tools
            .iter()
            .map(|tool| tool["name"].as_str().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            names,
            [
                "yttt_whoami",
                "yttt_spawn_agent",
                "yttt_list_agents",
                "yttt_send",
                "yttt_wait",
                "yttt_result",
                "yttt_interrupt",
                "yttt_release",
                "yttt_close",
            ]
        );
        for tool in tools {
            assert_eq!(tool["inputSchema"]["type"], "object", "{tool}");
            assert!(!tool["description"].as_str().unwrap().is_empty());
        }
    }

    #[test]
    fn tool_calls_become_the_requests_the_cli_sends() {
        let host = host(Ok(closed()));
        let result = call(
            &host,
            "yttt_wait",
            json!({"agents": ["project:w:agent"], "after_turn": 2}),
        );
        assert_eq!(result["isError"], false);
        assert_eq!(
            result["structuredContent"]["closed"]["agent"],
            "project:w:agent"
        );
        call(
            &host,
            "yttt_send",
            json!({"agent": "project:w:agent", "text": "Add tests"}),
        );
        call(&host, "yttt_list_agents", json!({}));
        assert_eq!(
            *host.sent.borrow(),
            [
                (
                    AGENT_CONTROL_WAIT_PATH.to_string(),
                    json!({"agents": ["project:w:agent"], "until": "finished",
                           "after_turn": 2, "timeout_seconds": DEFAULT_WAIT_SECONDS}),
                ),
                (
                    AGENT_CONTROL_SEND_PATH.to_string(),
                    json!({"agent": "project:w:agent", "text": "Add tests", "queue": false}),
                ),
                (AGENT_CONTROL_LIST_PATH.to_string(), json!({})),
            ]
        );
        let whoami = call(&host, "yttt_whoami", json!({}));
        assert_eq!(whoami["structuredContent"]["role"], "orchestrator");
    }

    #[test]
    fn bad_arguments_refusals_and_lost_answers_are_tool_errors() {
        let host = host(Err(Some(AgentControlError::new(
            AgentControlErrorCode::HumanControl,
            "a person is driving this Agent",
        ))));
        let refused = call(&host, "yttt_send", json!({"agent": "a", "text": "x"}));
        assert_eq!(refused["isError"], true);
        assert_eq!(refused["structuredContent"]["code"], "human_control");

        for (name, arguments) in [
            ("yttt_send", json!({"agent": "a"})),
            (
                "yttt_spawn_agent",
                json!({"provider": "codex", "extra": 1, "args": "x"}),
            ),
            ("yttt_teleport", json!({})),
        ] {
            let result = call(&host, name, arguments);
            assert_eq!(result["isError"], true, "{name}");
            assert_eq!(
                result["structuredContent"]["code"], "invalid_request",
                "{name}"
            );
        }
        assert_eq!(
            host.sent.borrow().len(),
            1,
            "invalid calls never reach the Host"
        );

        let lost = call(&self::host(Err(None)), "yttt_list_agents", json!({}));
        assert_eq!(lost["structuredContent"]["code"], "outcome_unknown");

        let outside = PaneTransport(None);
        let result = call_tool("yttt_list_agents", json!({}), &outside);
        assert_eq!(result["structuredContent"]["code"], "forbidden");
    }
}
