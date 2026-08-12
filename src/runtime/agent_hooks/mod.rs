use std::{collections::BTreeMap, sync::Arc, time::Duration};

use serde_json::Value;
use yttt_client_core::ClientEvent;
use yttt_protocol::{
    Request, Response, ServerEvent,
    agent::{AgentHookEvent, AgentHookScope},
};

use crate::{
    config::default_layout::BuiltinAgent, host_runtime::DesktopHostRuntime,
    runtime::agent_manager::AgentPaneAddress,
};

pub mod installer;

pub const AGENT_HOOK_ENVIRONMENT_VARIABLES: [&str; 3] = [
    "YTTT_AGENT_HOOK_ENDPOINT",
    "YTTT_AGENT_HOOK_TOKEN",
    "YTTT_AGENT_HOOK_SCOPE",
];

#[derive(Clone)]
pub struct AgentHookClient {
    runtime: Arc<DesktopHostRuntime>,
    events: Arc<flume::Receiver<AgentHookRequest>>,
}

impl std::fmt::Debug for AgentHookClient {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentHookClient")
            .finish_non_exhaustive()
    }
}

impl AgentHookClient {
    pub fn new(runtime: Arc<DesktopHostRuntime>) -> Self {
        let source = runtime.events();
        let (sender, events) = flume::bounded(64);
        std::thread::Builder::new()
            .name("yttt-agent-hook-events".to_string())
            .spawn(move || {
                while let Ok(event) = source.recv() {
                    let ClientEvent::Server(event) = event else {
                        continue;
                    };
                    let ServerEvent::AgentHook(event) = event.body else {
                        continue;
                    };
                    let Some(event) = agent_hook_request(event) else {
                        continue;
                    };
                    if sender.try_send(event).is_err() && sender.is_disconnected() {
                        break;
                    }
                }
            })
            .expect("failed to spawn Agent hook Host event bridge");
        Self {
            runtime,
            events: Arc::new(events),
        }
    }

    pub fn environment(
        &self,
        address: &AgentPaneAddress,
        generation: u64,
    ) -> BTreeMap<String, String> {
        let response = self
            .runtime
            .request(Request::AgentHookEnvironment(AgentHookScope {
                project_id: address.project_id.clone(),
                tab_id: address.tab_id.clone(),
                pane_id: address.pane_id.clone(),
                generation,
            }));
        match response.recv_timeout(Duration::from_secs(2)) {
            Ok(Ok(Response::AgentHookEnvironment(environment))) => environment.variables,
            _ => BTreeMap::new(),
        }
    }

    pub fn drain(&self) -> Vec<AgentHookRequest> {
        self.events.try_iter().take(64).collect()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct AgentHookRequest {
    pub address: AgentPaneAddress,
    pub generation: u64,
    pub source: BuiltinAgent,
    pub event: String,
    pub payload: Value,
}

fn agent_hook_request(event: AgentHookEvent) -> Option<AgentHookRequest> {
    Some(AgentHookRequest {
        address: AgentPaneAddress::new(
            &event.scope.project_id,
            &event.scope.tab_id,
            &event.scope.pane_id,
        ),
        generation: event.scope.generation,
        source: BuiltinAgent::from_id(&event.source)?,
        event: event.event,
        payload: serde_json::from_slice(&event.payload_json).ok()?,
    })
}
