#![forbid(unsafe_code)]

use std::{
    collections::HashMap,
    fmt,
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use serde::Deserialize;
use serde_json::Value;
use uuid::Uuid;
use yttt_agent_core::{
    AgentExitReason, AgentInstanceId, AgentProcessExit, AgentProvider, AgentReducer,
    AgentSessionMetadata, AgentSnapshot, ProviderHookEvent, ProviderId, ProviderResumeCommand,
};

pub const AGENT_TITLE_PREFIX: &str = "yttt-agent-v1:";
pub const AGENT_PROTOCOL_VERSION: u8 = 1;
const MAX_ENCODED_FRAME_BYTES: usize = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AgentScopeKey(String);

impl AgentScopeKey {
    pub fn new(value: impl Into<String>) -> Result<Self, AgentRuntimeError> {
        let value = value.into();
        if value.trim().is_empty() || value.len() > 512 {
            return Err(AgentRuntimeError::InvalidScope);
        }
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct AgentLaunchToken(String);

impl AgentLaunchToken {
    fn random() -> Self {
        Self(format!(
            "{}{}",
            Uuid::new_v4().simple(),
            Uuid::new_v4().simple()
        ))
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for AgentLaunchToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AgentLaunchToken([redacted])")
    }
}

#[derive(Clone, Debug)]
pub struct PreparedAgentLaunch {
    pub instance_id: AgentInstanceId,
    pub provider_id: ProviderId,
    pub token: AgentLaunchToken,
    pub scope: AgentScopeKey,
    resume_arguments: Vec<String>,
    program_override: Option<&'static str>,
    restored_title: Option<String>,
    provider_display_name: &'static str,
}

impl PreparedAgentLaunch {
    pub fn environment(&self, generation: u64) -> [(String, String); 3] {
        [
            (
                "YTTT_AGENT_INSTANCE_ID".to_string(),
                self.instance_id.to_string(),
            ),
            (
                "YTTT_AGENT_TOKEN".to_string(),
                self.token.expose().to_string(),
            ),
            ("YTTT_AGENT_GENERATION".to_string(), generation.to_string()),
        ]
    }

    pub fn resume_arguments(&self) -> &[String] {
        &self.resume_arguments
    }

    pub fn program_override(&self) -> Option<&'static str> {
        self.program_override
    }

    pub fn restored_title(&self) -> Option<&str> {
        self.restored_title.as_deref()
    }

    pub fn provider_display_name(&self) -> &'static str {
        self.provider_display_name
    }
}

#[derive(Clone, Debug)]
pub struct AgentRuntimeUpdate {
    pub scope: AgentScopeKey,
    pub snapshot: AgentSnapshot,
}

struct AgentRecord {
    scope: AgentScopeKey,
    token: AgentLaunchToken,
    provider: Arc<dyn AgentProvider>,
    reducer: AgentReducer,
}

#[derive(Default)]
pub struct AgentRuntime {
    providers: Vec<Arc<dyn AgentProvider>>,
    records: HashMap<AgentInstanceId, AgentRecord>,
}

impl AgentRuntime {
    pub fn register_provider(&mut self, provider: Arc<dyn AgentProvider>) {
        let provider_id = provider.descriptor().id;
        self.providers
            .retain(|existing| existing.descriptor().id != provider_id);
        self.providers.push(provider);
    }

    pub fn resume_command(
        &self,
        provider_id: &str,
        session: &AgentSessionMetadata,
    ) -> Option<ProviderResumeCommand> {
        self.providers
            .iter()
            .find(|provider| provider.descriptor().id.as_str() == provider_id)?
            .resume_command(session)
    }

    pub fn prepare_launch(
        &mut self,
        command: &str,
        scope: AgentScopeKey,
    ) -> Option<(PreparedAgentLaunch, AgentSnapshot)> {
        self.prepare_launch_with_snapshot(command, scope, None)
    }

    pub fn prepare_launch_with_snapshot(
        &mut self,
        command: &str,
        scope: AgentScopeKey,
        restored: Option<&AgentSnapshot>,
    ) -> Option<(PreparedAgentLaunch, AgentSnapshot)> {
        let command_provider = self
            .providers
            .iter()
            .find(|provider| provider.matches_command(command))
            .cloned();
        let restored_provider = restored.and_then(|snapshot| {
            self.providers
                .iter()
                .find(|provider| provider.descriptor().id == snapshot.provider_id)
                .cloned()
        });
        let provider = command_provider.or(restored_provider)?;
        let descriptor = provider.descriptor();
        let command_matches = provider.matches_command(command);
        let resume_command = restored
            .filter(|snapshot| snapshot.provider_id == descriptor.id)
            .and_then(|snapshot| snapshot.session.as_ref())
            .and_then(|session| provider.resume_command(session));
        if !command_matches && resume_command.is_none() {
            return None;
        }
        let restored = resume_command.as_ref().and(restored);
        let program_override = resume_command
            .as_ref()
            .filter(|_| !command_matches)
            .map(|command| command.program);
        let resume_arguments = resume_command
            .as_ref()
            .map(|command| command.arguments.clone())
            .unwrap_or_default();
        let restored_title = restored
            .and_then(|snapshot| snapshot.session.as_ref())
            .and_then(|session| session.title.clone());
        let instance_id = AgentInstanceId::random();
        let token = AgentLaunchToken::random();
        let now = now_millis();
        let mut reducer = match restored {
            Some(snapshot) => AgentReducer::from_restored(
                instance_id.clone(),
                descriptor.id.clone(),
                snapshot,
                now,
            ),
            None => AgentReducer::new(instance_id.clone(), descriptor.id.clone(), now),
        };
        reducer.process_starting(1, now);
        let snapshot = reducer.snapshot().clone();
        self.records.insert(
            instance_id.clone(),
            AgentRecord {
                scope: scope.clone(),
                token: token.clone(),
                provider,
                reducer,
            },
        );
        Some((
            PreparedAgentLaunch {
                instance_id,
                provider_id: descriptor.id,
                token,
                scope,
                resume_arguments,
                program_override,
                restored_title,
                provider_display_name: descriptor.display_name,
            },
            snapshot,
        ))
    }

    pub fn process_started(
        &mut self,
        instance_id: &AgentInstanceId,
        generation: u64,
    ) -> Option<AgentRuntimeUpdate> {
        let record = self.records.get_mut(instance_id)?;
        if generation != record.reducer.snapshot().generation {
            record.reducer.process_starting(generation, now_millis());
        }
        record.reducer.process_started(generation, now_millis());
        Some(update_from_record(record))
    }

    pub fn process_exited(
        &mut self,
        instance_id: &AgentInstanceId,
        generation: u64,
        exit: AgentProcessExit,
    ) -> Option<AgentRuntimeUpdate> {
        let record = self.records.get_mut(instance_id)?;
        record
            .reducer
            .process_exited(generation, exit, now_millis());
        Some(update_from_record(record))
    }

    pub fn ingest_title(
        &mut self,
        title: &str,
    ) -> Result<Option<AgentRuntimeUpdate>, AgentRuntimeError> {
        let Some(encoded) = title.strip_prefix(AGENT_TITLE_PREFIX) else {
            return Ok(None);
        };
        if encoded.len() > MAX_ENCODED_FRAME_BYTES {
            return Err(AgentRuntimeError::FrameTooLarge);
        }
        let bytes = URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|_| AgentRuntimeError::InvalidEncoding)?;
        let envelope: HookEnvelope =
            serde_json::from_slice(&bytes).map_err(|_| AgentRuntimeError::InvalidEnvelope)?;
        if envelope.protocol != AGENT_PROTOCOL_VERSION {
            return Err(AgentRuntimeError::UnsupportedProtocol(envelope.protocol));
        }
        let instance_id = AgentInstanceId::new(envelope.instance_id)
            .map_err(|_| AgentRuntimeError::InvalidEnvelope)?;
        let record = self
            .records
            .get(&instance_id)
            .ok_or(AgentRuntimeError::UnknownInstance)?;
        if record.token.expose().as_bytes() != envelope.token.as_bytes() {
            return Err(AgentRuntimeError::InvalidToken);
        }
        self.ingest_hook(
            &instance_id,
            envelope.generation,
            &envelope.event,
            &envelope.payload,
        )
        .map(Some)
    }

    pub fn ingest_hook(
        &mut self,
        instance_id: &AgentInstanceId,
        generation: u64,
        event: &str,
        payload: &Value,
    ) -> Result<AgentRuntimeUpdate, AgentRuntimeError> {
        let record = self
            .records
            .get_mut(instance_id)
            .ok_or(AgentRuntimeError::UnknownInstance)?;
        if record.reducer.snapshot().generation != generation {
            return Err(AgentRuntimeError::StaleGeneration);
        }
        let normalized = record.provider.normalize_hook(ProviderHookEvent {
            name: event,
            payload,
        })?;
        let now = now_millis();
        for event in normalized {
            record.reducer.apply(generation, event, now);
        }
        Ok(update_from_record(record))
    }

    pub fn snapshot(&self, instance_id: &AgentInstanceId) -> Option<&AgentSnapshot> {
        self.records
            .get(instance_id)
            .map(|record| record.reducer.snapshot())
    }

    pub fn remove(&mut self, instance_id: &AgentInstanceId) {
        self.records.remove(instance_id);
    }
}

fn update_from_record(record: &AgentRecord) -> AgentRuntimeUpdate {
    AgentRuntimeUpdate {
        scope: record.scope.clone(),
        snapshot: record.reducer.snapshot().clone(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookEnvelope {
    protocol: u8,
    instance_id: String,
    token: String,
    generation: u64,
    event: String,
    payload: Value,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

#[derive(Debug, thiserror::Error)]
pub enum AgentRuntimeError {
    #[error("agent scope is empty or too long")]
    InvalidScope,
    #[error("agent event frame exceeds the transport limit")]
    FrameTooLarge,
    #[error("agent event frame is not valid base64url")]
    InvalidEncoding,
    #[error("agent event frame is not a valid envelope")]
    InvalidEnvelope,
    #[error("unsupported agent event protocol {0}")]
    UnsupportedProtocol(u8),
    #[error("agent event references an unknown instance")]
    UnknownInstance,
    #[error("agent event token is invalid")]
    InvalidToken,
    #[error("agent event belongs to a stale process generation")]
    StaleGeneration,
    #[error(transparent)]
    Provider(#[from] yttt_agent_core::ProviderError),
}

pub fn completed_exit(code: Option<i32>) -> AgentProcessExit {
    AgentProcessExit {
        code,
        reason: AgentExitReason::Completed,
    }
}

#[cfg(test)]
mod tests {
    use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
    use serde_json::json;
    use yttt_agent_core::{AgentProvider, AgentTurnState};

    use super::*;

    #[derive(Default)]
    struct TestProvider;

    impl AgentProvider for TestProvider {
        fn descriptor(&self) -> yttt_agent_core::ProviderDescriptor {
            yttt_agent_core::ProviderDescriptor {
                id: ProviderId::from_static("test"),
                display_name: "Test",
            }
        }

        fn matches_command(&self, command: &str) -> bool {
            command == "test-agent"
        }

        fn normalize_hook(
            &self,
            event: ProviderHookEvent<'_>,
        ) -> Result<Vec<yttt_agent_core::AgentEventKind>, yttt_agent_core::ProviderError> {
            assert_eq!(event.name, "working");
            Ok(vec![yttt_agent_core::AgentEventKind::Working])
        }
    }

    #[test]
    fn authenticates_and_routes_title_frames() {
        let mut runtime = AgentRuntime::default();
        runtime.register_provider(Arc::new(TestProvider));
        let (launch, _) = runtime
            .prepare_launch(
                "test-agent",
                AgentScopeKey::new("project/tab/pane").unwrap(),
            )
            .unwrap();
        runtime.process_started(&launch.instance_id, 1).unwrap();
        let frame = json!({
            "protocol": 1,
            "instanceId": launch.instance_id.as_str(),
            "token": launch.token.expose(),
            "generation": 1,
            "event": "working",
            "payload": {}
        });
        let title = format!(
            "{AGENT_TITLE_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&frame).unwrap())
        );
        let update = runtime.ingest_title(&title).unwrap().unwrap();
        assert_eq!(update.snapshot.turn_state, AgentTurnState::Working);
    }

    #[test]
    fn rejects_bad_tokens_without_mutating_state() {
        let mut runtime = AgentRuntime::default();
        runtime.register_provider(Arc::new(TestProvider));
        let (launch, _) = runtime
            .prepare_launch("test-agent", AgentScopeKey::new("scope").unwrap())
            .unwrap();
        let frame = json!({
            "protocol": 1,
            "instanceId": launch.instance_id.as_str(),
            "token": "wrong",
            "generation": 1,
            "event": "working",
            "payload": {}
        });
        let title = format!(
            "{AGENT_TITLE_PREFIX}{}",
            URL_SAFE_NO_PAD.encode(serde_json::to_vec(&frame).unwrap())
        );
        assert!(matches!(
            runtime.ingest_title(&title),
            Err(AgentRuntimeError::InvalidToken)
        ));
        assert_eq!(
            runtime.snapshot(&launch.instance_id).unwrap().turn_state,
            AgentTurnState::Idle
        );
    }
}
