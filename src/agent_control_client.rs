//! The CLI side of Agent control: who the calling process is, read from the
//! variables the Host injected into its terminal, and verified with the Host
//! when the process claims an orchestrator credential.

use std::{
    io::{Read as _, Write as _},
    net::{SocketAddr, TcpStream},
    time::Duration,
};

use serde::Serialize;
use yttt_core::model::layout::PaneRole;
use yttt_protocol::agent::{
    AGENT_CONTROL_SCOPE_HEADER, AGENT_CONTROL_TOKEN_ENV, AGENT_CONTROL_TOKEN_HEADER,
    AGENT_CONTROL_WHOAMI_PATH, AGENT_HOOK_ENDPOINT_ENV, AGENT_SELF_ENV, AgentControlError,
    AgentControlErrorCode, AgentControlIdentity, AgentControlResponse, AgentHookScope,
    MAX_AGENT_WAIT_SECONDS, agent_control_blocks,
};

const CONTROL_TIMEOUT: Duration = Duration::from_secs(2);
const MAX_RESPONSE_BYTES: usize = 1024 * 1024;

/// What the Host made of an orchestrator credential.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlAccess {
    /// The pane was not started as an orchestrator.
    NotGranted,
    /// The Host accepted the credential.
    Verified,
    /// The Host refused it: the pane restarted or its role changed.
    Rejected,
    /// The Host could not be reached from this process.
    Unavailable,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WhoAmI {
    /// Not running inside a yttt terminal.
    External,
    Pane {
        project_id: String,
        terminal_session_id: String,
        generation: u64,
        role: PaneRole,
        control: ControlAccess,
    },
}

/// The identity a yttt terminal hands to everything running in it.
pub struct PaneCredentials {
    pub scope: AgentHookScope,
    encoded_scope: String,
    endpoint: Option<String>,
    control_token: Option<String>,
}

impl PaneCredentials {
    pub fn from_env() -> Option<Self> {
        Self::from_lookup(|name| std::env::var(name).ok())
    }

    pub fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Option<Self> {
        let encoded_scope = lookup(AGENT_SELF_ENV)?;
        let scope = AgentHookScope::decode(&encoded_scope)?;
        let non_empty = |name| lookup(name).filter(|value| !value.is_empty());
        Some(Self {
            scope,
            encoded_scope,
            endpoint: non_empty(AGENT_HOOK_ENDPOINT_ENV),
            control_token: non_empty(AGENT_CONTROL_TOKEN_ENV),
        })
    }

    pub fn whoami(&self) -> WhoAmI {
        let (role, control) = match &self.control_token {
            None => (PaneRole::Worker, ControlAccess::NotGranted),
            Some(_) => match self.post(AGENT_CONTROL_WHOAMI_PATH, &[]) {
                Ok((200, body)) => match serde_json::from_slice::<AgentControlIdentity>(&body) {
                    Ok(identity) => (identity.role, ControlAccess::Verified),
                    Err(_) => (PaneRole::Orchestrator, ControlAccess::Unavailable),
                },
                Ok((403, _)) => (PaneRole::Orchestrator, ControlAccess::Rejected),
                Ok(_) | Err(()) => (PaneRole::Orchestrator, ControlAccess::Unavailable),
            },
        };
        WhoAmI::Pane {
            project_id: self.scope.project_id.clone(),
            terminal_session_id: self.scope.pane_id.clone(),
            generation: self.scope.generation,
            role,
            control,
        }
    }

    /// Whether this pane holds an orchestrator credential at all; only the
    /// Host can say whether it is still valid.
    pub fn has_control_token(&self) -> bool {
        self.control_token.is_some()
    }

    /// Sends one control request. `Err(None)` means no answer arrived, so the
    /// request may or may not have been carried out.
    pub fn control(
        &self,
        path: &str,
        body: &serde_json::Value,
    ) -> Result<AgentControlResponse, Option<AgentControlError>> {
        let body = serde_json::to_vec(body).expect("control request serializes");
        let (status, body) = self.post(path, &body).map_err(|()| None)?;
        if status == 200 {
            return serde_json::from_slice(&body).map_err(|_| None);
        }
        Err(Some(
            serde_json::from_slice::<AgentControlError>(&body).unwrap_or_else(|_| {
                AgentControlError::new(
                    if status == 403 {
                        AgentControlErrorCode::Forbidden
                    } else {
                        AgentControlErrorCode::Failed
                    },
                    format!("the Host answered {status}"),
                )
            }),
        ))
    }

    fn post(&self, path: &str, body: &[u8]) -> Result<(u16, Vec<u8>), ()> {
        let address = self
            .endpoint
            .as_deref()
            .and_then(|endpoint| endpoint.strip_prefix("http://"))
            .and_then(|address| address.parse::<SocketAddr>().ok())
            .filter(|address| address.ip().is_loopback())
            .ok_or(())?;
        let token = self.control_token.as_deref().ok_or(())?;
        let mut stream = TcpStream::connect_timeout(&address, CONTROL_TIMEOUT).map_err(|_| ())?;
        // The Host answers a wait or interrupt only when it ends.
        let read_timeout = if agent_control_blocks(path) {
            Duration::from_secs(u64::from(MAX_AGENT_WAIT_SECONDS) + 15)
        } else {
            CONTROL_TIMEOUT
        };
        stream
            .set_read_timeout(Some(read_timeout))
            .map_err(|_| ())?;
        stream
            .set_write_timeout(Some(CONTROL_TIMEOUT))
            .map_err(|_| ())?;
        write!(
            stream,
            "POST {path} HTTP/1.1\r\nHost: {address}\r\n{AGENT_CONTROL_SCOPE_HEADER}: {}\r\n{AGENT_CONTROL_TOKEN_HEADER}: {token}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            self.encoded_scope,
            body.len()
        )
        .map_err(|_| ())?;
        stream.write_all(body).map_err(|_| ())?;
        let mut response = Vec::new();
        stream
            .take(MAX_RESPONSE_BYTES as u64 + 1)
            .read_to_end(&mut response)
            .map_err(|_| ())?;
        parse_response(&response)
    }
}

fn parse_response(response: &[u8]) -> Result<(u16, Vec<u8>), ()> {
    if response.len() > MAX_RESPONSE_BYTES {
        return Err(());
    }
    let header_end = response
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or(())?;
    let status = std::str::from_utf8(&response[..header_end])
        .map_err(|_| ())?
        .split_whitespace()
        .nth(1)
        .and_then(|status| status.parse::<u16>().ok())
        .ok_or(())?;
    Ok((status, response[header_end + 4..].to_vec()))
}

#[cfg(test)]
mod tests {
    use std::{collections::HashMap, net::TcpListener, thread};

    use yttt_core::model::ids::TerminalSessionId;

    use super::*;

    fn scope() -> AgentHookScope {
        AgentHookScope {
            project_id: "project".into(),
            tab_id: "project:tab:pane".into(),
            pane_id: "project:tab:pane".into(),
            generation: 3,
        }
    }

    fn credentials(variables: &[(&str, String)]) -> Option<PaneCredentials> {
        let variables: HashMap<_, _> = variables
            .iter()
            .map(|(name, value)| (name.to_string(), value.clone()))
            .collect();
        PaneCredentials::from_lookup(|name| variables.get(name).cloned())
    }

    /// Serves one request with `status` and `body`, returning its endpoint.
    fn one_shot_host(status: &'static str, body: String) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            // Closing before the whole request is read would reset the connection.
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !request.windows(4).any(|window| window == b"\r\n\r\n") {
                let count = stream.read(&mut chunk).unwrap();
                if count == 0 {
                    break;
                }
                request.extend_from_slice(&chunk[..count]);
            }
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        format!("http://{address}")
    }

    fn pane(role: PaneRole, control: ControlAccess) -> WhoAmI {
        WhoAmI::Pane {
            project_id: "project".into(),
            terminal_session_id: "project:tab:pane".into(),
            generation: 3,
            role,
            control,
        }
    }

    #[test]
    fn a_process_outside_yttt_has_no_pane_identity() {
        assert!(credentials(&[]).is_none());
        assert!(credentials(&[(AGENT_SELF_ENV, "not-a-scope".into())]).is_none());
    }

    #[test]
    fn a_worker_pane_knows_itself_without_asking_the_host() {
        let whoami = credentials(&[(AGENT_SELF_ENV, scope().encode())])
            .unwrap()
            .whoami();
        assert_eq!(whoami, pane(PaneRole::Worker, ControlAccess::NotGranted));
    }

    #[test]
    fn an_orchestrator_credential_is_verified_by_the_host() {
        let identity = AgentControlIdentity {
            project_id: "project".into(),
            terminal_session_id: TerminalSessionId::new("project:tab:pane"),
            generation: 3,
            role: PaneRole::Orchestrator,
        };
        let endpoint = one_shot_host("200 OK", serde_json::to_string(&identity).unwrap());
        let whoami = credentials(&[
            (AGENT_SELF_ENV, scope().encode()),
            (AGENT_HOOK_ENDPOINT_ENV, endpoint),
            (AGENT_CONTROL_TOKEN_ENV, "token".into()),
        ])
        .unwrap()
        .whoami();
        assert_eq!(
            whoami,
            pane(PaneRole::Orchestrator, ControlAccess::Verified)
        );
    }

    #[test]
    fn a_refused_or_unreachable_credential_is_reported_as_such() {
        let refused = one_shot_host("403 Forbidden", String::new());
        let whoami = credentials(&[
            (AGENT_SELF_ENV, scope().encode()),
            (AGENT_HOOK_ENDPOINT_ENV, refused),
            (AGENT_CONTROL_TOKEN_ENV, "token".into()),
        ])
        .unwrap()
        .whoami();
        assert_eq!(
            whoami,
            pane(PaneRole::Orchestrator, ControlAccess::Rejected)
        );

        let unused = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let closed = format!("http://{}", unused.local_addr().unwrap());
        drop(unused);
        for endpoint in [closed, "http://192.0.2.1:9".into()] {
            let whoami = credentials(&[
                (AGENT_SELF_ENV, scope().encode()),
                (AGENT_HOOK_ENDPOINT_ENV, endpoint),
                (AGENT_CONTROL_TOKEN_ENV, "token".into()),
            ])
            .unwrap()
            .whoami();
            assert_eq!(
                whoami,
                pane(PaneRole::Orchestrator, ControlAccess::Unavailable)
            );
        }
    }
}

#[cfg(test)]
mod control_tests {
    use std::{net::TcpListener, thread};

    use yttt_core::model::ids::TerminalSessionId;

    use super::*;

    /// Answers one request with `status` and `body` after checking that the
    /// request carried `expected_body`.
    fn host(status: &'static str, body: String, expected_body: &'static str) -> String {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0_u8; 1024];
            while !String::from_utf8_lossy(&request).ends_with(expected_body) {
                let count = stream.read(&mut chunk).unwrap();
                assert!(count > 0, "request ended before its body");
                request.extend_from_slice(&chunk[..count]);
            }
            let _ = write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        });
        format!("http://{address}")
    }

    fn orchestrator(endpoint: String) -> PaneCredentials {
        let scope = AgentHookScope {
            project_id: "project".into(),
            tab_id: "project:lead:lead".into(),
            pane_id: "project:lead:lead".into(),
            generation: 1,
        };
        let variables = [
            (AGENT_SELF_ENV, scope.encode()),
            (AGENT_HOOK_ENDPOINT_ENV, endpoint),
            (AGENT_CONTROL_TOKEN_ENV, "token".to_string()),
        ];
        PaneCredentials::from_lookup(|name| {
            variables
                .iter()
                .find(|(candidate, _)| *candidate == name)
                .map(|(_, value)| value.clone())
        })
        .unwrap()
    }

    #[test]
    fn a_control_request_returns_the_hosts_typed_answer_or_refusal() {
        let closed = AgentControlResponse::Closed {
            agent: TerminalSessionId::new("project:w:agent"),
        };
        let pane = orchestrator(host(
            "200 OK",
            serde_json::to_string(&closed).unwrap(),
            r#"{"agent":"project:w:agent"}"#,
        ));
        assert_eq!(
            pane.control(
                "/control/v1/agents/close",
                &serde_json::json!({"agent": "project:w:agent"})
            )
            .unwrap(),
            closed
        );

        let refusal = AgentControlError::new(AgentControlErrorCode::Forbidden, "not yours");
        let pane = orchestrator(host(
            "403 Forbidden",
            serde_json::to_string(&refusal).unwrap(),
            "{}",
        ));
        assert_eq!(
            pane.control("/control/v1/agents/list", &serde_json::json!({}))
                .unwrap_err(),
            Some(refusal)
        );

        let pane = orchestrator(host("503 Service Unavailable", String::new(), "{}"));
        let error = pane
            .control("/control/v1/agents/list", &serde_json::json!({}))
            .unwrap_err()
            .unwrap();
        assert_eq!(error.code, AgentControlErrorCode::Failed);
    }
}
