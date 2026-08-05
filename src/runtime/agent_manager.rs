use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use yttt_agent_core::{
    AgentExitReason, AgentInstanceId, AgentProcessExit, AgentProcessState, AgentProvider,
    AgentReducer, AgentSessionMetadata, AgentSnapshot, ProviderHookEvent, ProviderId,
    ProviderResumeCommand,
};
use yttt_agent_providers::{
    OMP_EXTENSION_FILE_NAME, OMP_EXTENSION_SOURCE, OMP_PROVIDER_ID, builtin_providers,
};
use yttt_agent_runtime::{
    AgentRuntime, AgentRuntimeError, AgentRuntimeUpdate, AgentScopeKey, PreparedAgentLaunch,
};

use crate::{
    config::{atomic_write, default_layout::BuiltinAgent, paths::AppConfigPaths},
    runtime::agent_hooks::{
        AgentHookClient, AgentHookRequest, AgentHookServer, installer::install_managed_hooks,
    },
};

const AGENT_STATE_VERSION: u32 = 1;
const AGENT_STATE_MAX_BYTES: usize = 1024 * 1024;
const AGENT_STATE_MAX_ENTRIES: usize = 256;

#[derive(Serialize, Deserialize)]
struct PersistedAgentState {
    version: u32,
    entries: Vec<PersistedAgentEntry>,
}

#[derive(Serialize, Deserialize)]
struct PersistedAgentEntry {
    address: AgentPaneAddress,
    snapshot: AgentSnapshot,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentPaneAddress {
    pub project_id: String,
    pub tab_id: String,
    pub pane_id: String,
}

impl AgentPaneAddress {
    pub fn new(project_id: &str, tab_id: &str, pane_id: &str) -> Self {
        Self {
            project_id: project_id.to_string(),
            tab_id: tab_id.to_string(),
            pane_id: pane_id.to_string(),
        }
    }

    fn scope_key(&self) -> AgentScopeKey {
        AgentScopeKey::new(format!(
            "{}\u{1f}{}\u{1f}{}",
            self.project_id, self.tab_id, self.pane_id
        ))
        .expect("pane identifiers form a valid agent scope")
    }
}

#[derive(Clone, Debug)]
pub struct AgentPaneLaunch {
    prepared: PreparedAgentLaunch,
    additional_args: Vec<String>,
    remote_extension_base64: Option<Arc<str>>,
}

impl AgentPaneLaunch {
    pub fn instance_id(&self) -> &AgentInstanceId {
        &self.prepared.instance_id
    }

    pub fn environment(&self, generation: u64) -> [(String, String); 3] {
        self.prepared.environment(generation)
    }

    pub fn additional_args(&self) -> &[String] {
        &self.additional_args
    }

    pub fn program_override(&self) -> Option<&str> {
        self.prepared.program_override()
    }

    pub fn restored_title_for(&self, default_title: &str) -> Option<&str> {
        let default_is_generic = default_title.eq_ignore_ascii_case("shell")
            || default_title.eq_ignore_ascii_case("terminal")
            || default_title.eq_ignore_ascii_case(self.prepared.provider_display_name());
        default_is_generic
            .then(|| self.prepared.restored_title())
            .flatten()
    }

    pub fn remote_command(&self, program: &str, args: &[String]) -> Option<String> {
        let encoded = self.remote_extension_base64.as_deref()?;
        let program = self.program_override().unwrap_or(program);
        let mut command = format!(
            "umask 077 && yttt_agent_dir=\"$HOME/.config/yttt/agent-providers/{OMP_PROVIDER_ID}\" && \\
             mkdir -p \"$yttt_agent_dir\" && yttt_agent_path=\"$yttt_agent_dir/{OMP_EXTENSION_FILE_NAME}\" && \\
             yttt_agent_tmp=\"$yttt_agent_path.$$\" && printf '%s' '{encoded}' | base64 -d > \"$yttt_agent_tmp\" && \\
             mv -f \"$yttt_agent_tmp\" \"$yttt_agent_path\" && exec {}",
            shell_quote(program)
        );
        if self.program_override().is_none() {
            for arg in args {
                command.push(' ');
                command.push_str(&shell_quote(arg));
            }
        }
        for arg in self.additional_args() {
            command.push(' ');
            command.push_str(&shell_quote(arg));
        }
        command.push_str(" --extension \"$yttt_agent_path\"");
        Some(command)
    }
}

struct DetectedAgentProcess {
    agent: BuiltinAgent,
    generation: u64,
    reducer: AgentReducer,
}

pub struct AgentManager {
    runtime: AgentRuntime,
    launches_by_address: HashMap<AgentPaneAddress, AgentPaneLaunch>,
    addresses_by_instance: HashMap<AgentInstanceId, AgentPaneAddress>,
    detected_by_address: HashMap<AgentPaneAddress, DetectedAgentProcess>,
    finished_detected_generations: HashMap<AgentPaneAddress, u64>,
    hook_providers: HashMap<String, Arc<dyn AgentProvider>>,
    hook_server: Option<AgentHookServer>,
    omp_extension_path: Option<PathBuf>,
    state_path: PathBuf,
    retained_snapshots: HashMap<AgentPaneAddress, AgentSnapshot>,
    omp_extension_base64: Arc<str>,
    last_error: Option<String>,
    setup_error: Option<String>,
}

impl AgentManager {
    pub fn new(config_paths: &AppConfigPaths) -> Self {
        let mut runtime = AgentRuntime::default();
        let providers = builtin_providers();
        let hook_providers = providers
            .iter()
            .map(|provider| (provider.descriptor().id.to_string(), Arc::clone(provider)))
            .collect();
        for provider in providers {
            runtime.register_provider(provider);
        }
        let (hook_server, hook_error) = match AgentHookServer::start() {
            Ok(server) => (Some(server), None),
            Err(error) => (None, Some(format!("agent hook listener: {error}"))),
        };
        let adapter_error = install_managed_hooks(config_paths)
            .err()
            .map(|error| format!("agent hook adapters: {error}"));
        let (omp_extension_path, extension_error) = match install_omp_extension(config_paths) {
            Ok(path) => (Some(path), None),
            Err(error) => (None, Some(error.to_string())),
        };
        let state_path = config_paths.agent_state_path();
        let (retained_snapshots, state_error) = match load_agent_state(&state_path) {
            Ok(state) => (state, None),
            Err(error) => (HashMap::new(), Some(error.to_string())),
        };
        let setup_error = combine_errors(
            combine_errors(combine_errors(extension_error, state_error), hook_error),
            adapter_error,
        );
        let omp_extension_base64 = Arc::<str>::from(STANDARD.encode(OMP_EXTENSION_SOURCE));
        Self {
            runtime,
            launches_by_address: HashMap::new(),
            addresses_by_instance: HashMap::new(),
            detected_by_address: HashMap::new(),
            finished_detected_generations: HashMap::new(),
            hook_providers,
            hook_server,
            omp_extension_path,
            state_path,
            retained_snapshots,
            omp_extension_base64,
            last_error: None,
            setup_error,
        }
    }

    pub fn setup_error(&self) -> Option<&str> {
        self.setup_error.as_deref()
    }

    pub fn resume_command(
        &self,
        provider_id: &str,
        session: &AgentSessionMetadata,
    ) -> Option<ProviderResumeCommand> {
        self.hook_providers
            .get(provider_id)?
            .resume_command(session)
    }

    pub fn retained_snapshots(&self) -> Vec<(AgentPaneAddress, AgentSnapshot)> {
        self.retained_snapshots
            .iter()
            .map(|(address, snapshot)| (address.clone(), snapshot.clone()))
            .collect()
    }

    pub fn has_retained_snapshot(&self, address: &AgentPaneAddress) -> bool {
        self.retained_snapshots.contains_key(address)
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }
    pub fn hook_client(&self) -> Option<AgentHookClient> {
        self.hook_server.as_ref().map(AgentHookServer::client)
    }

    pub fn drain_hook_requests(&self) -> Vec<AgentHookRequest> {
        self.hook_server
            .as_ref()
            .map(AgentHookServer::drain)
            .unwrap_or_default()
    }

    pub fn ingest_hook_request(
        &mut self,
        request: AgentHookRequest,
    ) -> Result<Option<(AgentPaneAddress, AgentSnapshot)>, AgentRuntimeError> {
        let provider_id = request.source.id();
        if let Some(launch) = self.launches_by_address.get(&request.address) {
            if launch.prepared.provider_id.as_str() != provider_id {
                return Ok(None);
            }
            let instance_id = launch.instance_id().clone();
            let update = self.runtime.ingest_hook(
                &instance_id,
                request.generation,
                &request.event,
                &request.payload,
            )?;
            return Ok(self.resolve_update(update));
        }
        if self
            .finished_detected_generations
            .get(&request.address)
            .is_some_and(|generation| *generation == request.generation)
        {
            return Ok(None);
        }

        if !self
            .detected_by_address
            .get(&request.address)
            .is_some_and(|detected| {
                detected.agent == request.source && detected.generation == request.generation
            })
        {
            self.detected_by_address.remove(&request.address);
            self.detected_process_started(
                request.address.clone(),
                request.source,
                request.generation,
            );
        }
        let Some(provider) = self.hook_providers.get(provider_id) else {
            return Ok(None);
        };
        let events = provider.normalize_hook(ProviderHookEvent {
            name: &request.event,
            payload: &request.payload,
        })?;
        let snapshot = {
            let Some(detected) = self.detected_by_address.get_mut(&request.address) else {
                return Ok(None);
            };
            let now = unix_timestamp_millis();
            for event in events {
                detected.reducer.apply(request.generation, event, now);
            }
            detected.reducer.snapshot().clone()
        };
        self.persist_snapshot(request.address.clone(), snapshot.clone());
        Ok(Some((request.address, snapshot)))
    }

    pub fn prepare_pane(
        &mut self,
        address: AgentPaneAddress,
        command: &str,
        remote: bool,
    ) -> Option<(AgentPaneLaunch, AgentSnapshot)> {
        if let Some(launch) = self.launches_by_address.get(&address) {
            let snapshot = self.runtime.snapshot(launch.instance_id())?.clone();
            return Some((launch.clone(), snapshot));
        }
        let restored = self.retained_snapshots.get(&address).cloned();
        let Some((prepared, snapshot)) = self.runtime.prepare_launch_with_snapshot(
            command,
            address.scope_key(),
            restored.as_ref(),
        ) else {
            if restored.is_some() {
                self.retained_snapshots.remove(&address);
                if let Err(error) = write_agent_state(&self.state_path, &self.retained_snapshots) {
                    self.last_error = Some(error.to_string());
                }
            }
            return None;
        };
        let is_omp = prepared.provider_id.as_str() == OMP_PROVIDER_ID;
        let mut additional_args = prepared.resume_arguments().to_vec();
        if is_omp
            && !remote
            && let Some(path) = &self.omp_extension_path
        {
            additional_args.push("--extension".to_string());
            additional_args.push(path.to_string_lossy().into_owned());
        }
        let launch = AgentPaneLaunch {
            prepared,
            additional_args,
            remote_extension_base64: (is_omp && remote).then(|| self.omp_extension_base64.clone()),
        };
        self.addresses_by_instance
            .insert(launch.instance_id().clone(), address.clone());
        self.launches_by_address
            .insert(address.clone(), launch.clone());
        self.persist_snapshot(address, snapshot.clone());
        Some((launch, snapshot))
    }

    pub fn process_started(
        &mut self,
        instance_id: &AgentInstanceId,
        generation: u64,
    ) -> Option<(AgentPaneAddress, AgentSnapshot)> {
        let update = self.runtime.process_started(instance_id, generation)?;
        self.resolve_update(update)
    }

    pub fn process_exited(
        &mut self,
        instance_id: &AgentInstanceId,
        generation: u64,
        exit: AgentProcessExit,
    ) -> Option<(AgentPaneAddress, AgentSnapshot)> {
        let update = self.runtime.process_exited(instance_id, generation, exit)?;
        self.resolve_update(update)
    }

    pub fn ingest_title(
        &mut self,
        title: &str,
    ) -> Result<Option<(AgentPaneAddress, AgentSnapshot)>, AgentRuntimeError> {
        let update = self.runtime.ingest_title(title)?;
        Ok(update.and_then(|update| self.resolve_update(update)))
    }

    pub fn detected_process_started(
        &mut self,
        address: AgentPaneAddress,
        agent: BuiltinAgent,
        generation: u64,
    ) -> Option<AgentSnapshot> {
        if self.launches_by_address.contains_key(&address)
            || self
                .detected_by_address
                .get(&address)
                .is_some_and(|detected| {
                    detected.agent == agent && detected.generation == generation
                })
        {
            return None;
        }
        self.finished_detected_generations.remove(&address);

        let now = unix_timestamp_millis();
        let mut reducer = AgentReducer::new(
            AgentInstanceId::random(),
            ProviderId::from_static(agent.id()),
            now,
        );
        reducer.process_starting(generation, now);
        reducer.process_started(generation, now);
        let snapshot = reducer.snapshot().clone();
        self.detected_by_address.insert(
            address.clone(),
            DetectedAgentProcess {
                agent,
                generation,
                reducer,
            },
        );
        self.persist_snapshot(address, snapshot.clone());
        Some(snapshot)
    }

    pub fn detected_process_exited(
        &mut self,
        address: &AgentPaneAddress,
        generation: u64,
        reason: AgentExitReason,
    ) -> Option<AgentSnapshot> {
        let detected = self.detected_by_address.remove(address)?;
        if detected.generation != generation {
            self.detected_by_address.insert(address.clone(), detected);
            return None;
        }
        self.finished_detected_generations
            .insert(address.clone(), generation);

        let mut reducer = detected.reducer;
        reducer.process_exited(
            generation,
            AgentProcessExit {
                code: (reason == AgentExitReason::Completed).then_some(0),
                reason,
            },
            unix_timestamp_millis(),
        );
        let snapshot = reducer.snapshot().clone();
        self.retained_snapshots.remove(address);
        if let Err(error) = write_agent_state(&self.state_path, &self.retained_snapshots) {
            self.last_error = Some(error.to_string());
        }
        Some(snapshot)
    }

    fn resolve_update(
        &mut self,
        update: AgentRuntimeUpdate,
    ) -> Option<(AgentPaneAddress, AgentSnapshot)> {
        let address = self
            .addresses_by_instance
            .get(&update.snapshot.instance_id)?
            .clone();
        self.persist_snapshot(address.clone(), update.snapshot.clone());
        Some((address, update.snapshot))
    }

    fn persist_snapshot(&mut self, address: AgentPaneAddress, snapshot: AgentSnapshot) {
        self.retained_snapshots.insert(address, snapshot);
        if self.retained_snapshots.len() > AGENT_STATE_MAX_ENTRIES {
            let mut oldest = self
                .retained_snapshots
                .iter()
                .map(|(address, snapshot)| (address.clone(), snapshot.updated_at))
                .collect::<Vec<_>>();
            oldest.sort_by_key(|(_, updated_at)| *updated_at);
            let remove_count = oldest.len() - AGENT_STATE_MAX_ENTRIES;
            for (address, _) in oldest.into_iter().take(remove_count) {
                self.retained_snapshots.remove(&address);
            }
        }
        if let Err(error) = write_agent_state(&self.state_path, &self.retained_snapshots) {
            self.last_error = Some(error.to_string());
        }
    }
}
fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

fn unix_timestamp_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn combine_errors(first: Option<String>, second: Option<String>) -> Option<String> {
    match (first, second) {
        (Some(first), Some(second)) => Some(format!("{first}; {second}")),
        (Some(error), None) | (None, Some(error)) => Some(error),
        (None, None) => None,
    }
}

fn load_agent_state(path: &Path) -> io::Result<HashMap<AgentPaneAddress, AgentSnapshot>> {
    let bytes = match fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(HashMap::new()),
        Err(error) => return Err(error),
    };
    if bytes.len() > AGENT_STATE_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent state exceeds the 1 MiB limit",
        ));
    }
    let mut persisted: PersistedAgentState = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if persisted.version != AGENT_STATE_VERSION {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("unsupported agent state version {}", persisted.version),
        ));
    }
    persisted
        .entries
        .sort_by_key(|entry| std::cmp::Reverse(entry.snapshot.updated_at));
    let mut snapshots = HashMap::new();
    for entry in persisted.entries.into_iter().take(AGENT_STATE_MAX_ENTRIES) {
        if valid_address(&entry.address)
            && entry.snapshot.process_state != AgentProcessState::Exited
        {
            snapshots.entry(entry.address).or_insert(entry.snapshot);
        }
    }
    Ok(snapshots)
}

fn valid_address(address: &AgentPaneAddress) -> bool {
    [&address.project_id, &address.tab_id, &address.pane_id]
        .into_iter()
        .all(|part| !part.trim().is_empty() && part.len() <= 512)
}

fn write_agent_state(
    path: &Path,
    snapshots: &HashMap<AgentPaneAddress, AgentSnapshot>,
) -> io::Result<()> {
    let mut entries = snapshots
        .iter()
        .map(|(address, snapshot)| PersistedAgentEntry {
            address: address.clone(),
            snapshot: snapshot.clone(),
        })
        .collect::<Vec<_>>();
    entries.sort_by(|left, right| {
        (
            &left.address.project_id,
            &left.address.tab_id,
            &left.address.pane_id,
        )
            .cmp(&(
                &right.address.project_id,
                &right.address.tab_id,
                &right.address.pane_id,
            ))
    });
    let source = serde_json::to_vec(&PersistedAgentState {
        version: AGENT_STATE_VERSION,
        entries,
    })
    .map_err(io::Error::other)?;
    if source.len() > AGENT_STATE_MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "agent state exceeds the 1 MiB limit",
        ));
    }
    atomic_write(path, &source)
}

fn install_omp_extension(config_paths: &AppConfigPaths) -> std::io::Result<PathBuf> {
    let directory = config_paths.agent_provider_dir(OMP_PROVIDER_ID);
    fs::create_dir_all(&directory)?;
    let path = directory.join(OMP_EXTENSION_FILE_NAME);
    let source = OMP_EXTENSION_SOURCE.as_bytes();
    if fs::read(&path).ok().as_deref() != Some(source) {
        atomic_write(&path, source)?;
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    #[test]
    fn prepares_omp_with_managed_extension_and_stable_identity() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        assert!(manager.setup_error().is_none());
        let address = AgentPaneAddress::new("project", "agent", "omp");
        let (first, snapshot) = manager.prepare_pane(address.clone(), "omp", false).unwrap();
        assert_eq!(snapshot.provider_id.as_str(), OMP_PROVIDER_ID);
        assert_eq!(first.additional_args()[0], "--extension");
        assert!(PathBuf::from(&first.additional_args()[1]).is_file());
        let (second, _) = manager.prepare_pane(address, "omp", false).unwrap();
        assert_eq!(first.instance_id(), second.instance_id());
    }

    #[test]
    fn remote_omp_uses_a_home_relative_managed_extension() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        let (launch, _) = manager
            .prepare_pane(
                AgentPaneAddress::new("project", "agent", "omp"),
                "omp",
                true,
            )
            .unwrap();
        assert!(launch.additional_args().is_empty());
        let command = launch
            .remote_command("omp", &["prompt with 'quote".to_string()])
            .unwrap();
        assert!(command.contains("$HOME/.config/yttt/agent-providers/omp"));
        assert!(command.contains("'prompt with '\"'\"'quote'"));
        assert!(command.contains("--extension \"$yttt_agent_path\""));
    }

    #[test]
    fn restores_the_latest_snapshot_from_disk() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let address = AgentPaneAddress::new("project", "agent", "omp");
        let expected_instance = {
            let mut manager = AgentManager::new(&paths);
            let (launch, _) = manager.prepare_pane(address.clone(), "omp", false).unwrap();
            assert!(manager.take_error().is_none());
            launch.instance_id().clone()
        };

        let restored = AgentManager::new(&paths).retained_snapshots();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored[0].0, address);
        assert_eq!(restored[0].1.instance_id, expected_instance);
    }

    #[test]
    fn detected_shell_agent_snapshot_is_removed_on_exit() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        let address = AgentPaneAddress::new("project", "shell", "terminal");

        let running = manager
            .detected_process_started(address.clone(), BuiltinAgent::Codex, 7)
            .unwrap();
        assert_eq!(running.provider_id.as_str(), "codex");
        assert_eq!(running.view_state(), yttt_agent_core::AgentViewState::Idle);
        assert!(
            manager
                .detected_process_started(address.clone(), BuiltinAgent::Codex, 7)
                .is_none()
        );

        let completed = manager
            .detected_process_exited(&address, 7, AgentExitReason::Completed)
            .unwrap();
        assert_eq!(
            completed.view_state(),
            yttt_agent_core::AgentViewState::Completed
        );
        assert!(manager.retained_snapshots().is_empty());
        drop(manager);
        assert!(AgentManager::new(&paths).retained_snapshots().is_empty());
    }

    #[test]
    fn late_omp_hook_does_not_restore_an_exited_shell_session() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        let address = AgentPaneAddress::new("project", "shell", "terminal");

        manager
            .detected_process_started(address.clone(), BuiltinAgent::OhMyPi, 7)
            .unwrap();
        manager
            .detected_process_exited(&address, 7, AgentExitReason::Completed)
            .unwrap();

        let late_hook = manager
            .ingest_hook_request(AgentHookRequest {
                address: address.clone(),
                generation: 7,
                source: BuiltinAgent::OhMyPi,
                event: "agent_end".to_string(),
                payload: json!({ "willContinue": false }),
            })
            .unwrap();
        assert!(late_hook.is_none());
        assert!(manager.retained_snapshots().is_empty());

        manager
            .detected_process_started(address.clone(), BuiltinAgent::OhMyPi, 7)
            .unwrap();
        assert!(
            manager
                .ingest_hook_request(AgentHookRequest {
                    address,
                    generation: 7,
                    source: BuiltinAgent::OhMyPi,
                    event: "session_start".to_string(),
                    payload: json!({ "sessionId": "omp-restarted" }),
                })
                .unwrap()
                .is_some()
        );
    }
    #[test]
    fn five_managed_agents_follow_prompt_working_and_completion_hooks() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        let cases = [
            (
                BuiltinAgent::Codex,
                "UserPromptSubmit",
                json!({ "prompt": "Codex task" }),
                "Stop",
                json!({}),
            ),
            (
                BuiltinAgent::Claude,
                "UserPromptSubmit",
                json!({ "prompt": "Claude task" }),
                "Stop",
                json!({}),
            ),
            (
                BuiltinAgent::OpenCode,
                "user_prompt",
                json!({ "prompt": "OpenCode task" }),
                "session_idle",
                json!({}),
            ),
            (
                BuiltinAgent::Pi,
                "before_agent_start",
                json!({ "prompt": "Pi task" }),
                "agent_end",
                json!({ "willContinue": false }),
            ),
            (
                BuiltinAgent::OhMyPi,
                "before_agent_start",
                json!({ "prompt": "Oh My Pi task" }),
                "agent_end",
                json!({ "willContinue": false }),
            ),
        ];

        for (agent, start_event, start_payload, finish_event, finish_payload) in cases {
            let address = AgentPaneAddress::new("project", "agent", agent.id());
            let (launch, _) = manager
                .prepare_pane(address.clone(), agent.command(), false)
                .unwrap();
            let (_, idle) = manager.process_started(launch.instance_id(), 1).unwrap();
            assert_eq!(idle.view_state(), yttt_agent_core::AgentViewState::Idle);

            let (_, working) = manager
                .ingest_hook_request(AgentHookRequest {
                    address: address.clone(),
                    generation: 1,
                    source: agent,
                    event: start_event.to_string(),
                    payload: start_payload,
                })
                .unwrap()
                .unwrap();
            assert_eq!(
                working.view_state(),
                yttt_agent_core::AgentViewState::Working
            );
            assert!(
                working.task.is_some(),
                "{} task missing",
                agent.display_name()
            );

            let (_, completed) = manager
                .ingest_hook_request(AgentHookRequest {
                    address,
                    generation: 1,
                    source: agent,
                    event: finish_event.to_string(),
                    payload: finish_payload,
                })
                .unwrap()
                .unwrap();
            assert_eq!(
                completed.view_state(),
                yttt_agent_core::AgentViewState::Completed
            );
        }
    }

    #[test]
    fn authenticated_omp_titles_update_parent_and_child_tasks() {
        use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
        use serde_json::{Value, json};

        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        let address = AgentPaneAddress::new("project", "agent", "omp");
        let (launch, _) = manager.prepare_pane(address.clone(), "omp", false).unwrap();
        manager.process_started(launch.instance_id(), 1).unwrap();
        let environment = launch.environment(1);
        let token = environment
            .iter()
            .find(|(name, _)| name == "YTTT_AGENT_TOKEN")
            .map(|(_, value)| value)
            .unwrap();
        let title_for = |event: &str, payload: Value| {
            let envelope = json!({
                "protocol": 1,
                "instanceId": launch.instance_id().as_str(),
                "token": token,
                "generation": 1,
                "event": event,
                "payload": payload,
            });
            format!(
                "{}{}",
                yttt_agent_runtime::AGENT_TITLE_PREFIX,
                URL_SAFE_NO_PAD.encode(serde_json::to_vec(&envelope).unwrap())
            )
        };

        let (updated_address, snapshot) = manager
            .ingest_title(&title_for(
                "before_agent_start",
                json!({ "prompt": "Implement authenticated OMP progress" }),
            ))
            .unwrap()
            .unwrap();
        assert_eq!(updated_address, address);
        assert_eq!(
            snapshot.task.as_ref().map(|task| task.title.as_str()),
            Some("Implement authenticated OMP progress")
        );
        assert_eq!(
            snapshot.view_state(),
            yttt_agent_core::AgentViewState::Working
        );

        manager
            .ingest_title(&title_for(
                "child_started",
                json!({
                    "childId": "child-1",
                    "name": "Reviewer",
                    "task": "Review runtime mapping",
                }),
            ))
            .unwrap();
        let (_, snapshot) = manager
            .ingest_title(&title_for(
                "child_updated",
                json!({
                    "childId": "child-1",
                    "task": "Review runtime mapping",
                    "action": "read",
                    "status": "running",
                }),
            ))
            .unwrap()
            .unwrap();
        assert_eq!(snapshot.children.len(), 1);
        assert_eq!(
            snapshot.children[0].primary_text(),
            "Review runtime mapping"
        );
        assert_eq!(
            snapshot.children[0].secondary_text().as_deref(),
            Some("read")
        );
    }
    #[test]
    fn persisted_configured_session_resumes_with_its_title() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let address = AgentPaneAddress::new("project", "agent", "opencode");
        let previous_instance = {
            let mut manager = AgentManager::new(&paths);
            let (launch, _) = manager
                .prepare_pane(address.clone(), "opencode", false)
                .unwrap();
            manager.process_started(launch.instance_id(), 1).unwrap();
            manager
                .ingest_hook_request(AgentHookRequest {
                    address: address.clone(),
                    generation: 1,
                    source: BuiltinAgent::OpenCode,
                    event: "session_start".to_string(),
                    payload: json!({
                        "id": "open-session-1",
                        "title": "Refactor authentication",
                    }),
                })
                .unwrap()
                .unwrap();
            launch.instance_id().clone()
        };

        let mut restored_manager = AgentManager::new(&paths);
        let (launch, snapshot) = restored_manager
            .prepare_pane(address, "opencode", false)
            .unwrap();
        assert_ne!(launch.instance_id(), &previous_instance);
        assert!(launch.program_override().is_none());
        assert_eq!(
            launch.additional_args(),
            &["--session".to_string(), "open-session-1".to_string()]
        );
        assert_eq!(
            launch.restored_title_for("OpenCode"),
            Some("Refactor authentication")
        );
        assert_eq!(snapshot.primary_text(), "Refactor authentication");
        assert_eq!(snapshot.process_state, AgentProcessState::Starting);
    }

    #[test]
    fn persisted_detected_shell_session_resumes_the_detected_provider() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let address = AgentPaneAddress::new("project", "dev", "shell");
        {
            let mut manager = AgentManager::new(&paths);
            manager
                .detected_process_started(address.clone(), BuiltinAgent::Codex, 7)
                .unwrap();
            manager
                .ingest_hook_request(AgentHookRequest {
                    address: address.clone(),
                    generation: 7,
                    source: BuiltinAgent::Codex,
                    event: "SessionStart".to_string(),
                    payload: json!({ "session_id": "codex-session-1" }),
                })
                .unwrap()
                .unwrap();
            manager
                .ingest_hook_request(AgentHookRequest {
                    address: address.clone(),
                    generation: 7,
                    source: BuiltinAgent::Codex,
                    event: "UserPromptSubmit".to_string(),
                    payload: json!({ "prompt": "Fix the flaky terminal test" }),
                })
                .unwrap()
                .unwrap();
        }

        let mut restored_manager = AgentManager::new(&paths);
        let (launch, snapshot) = restored_manager
            .prepare_pane(address, "zsh", false)
            .unwrap();
        assert_eq!(launch.program_override(), Some("codex"));
        assert_eq!(
            launch.additional_args(),
            &["resume".to_string(), "codex-session-1".to_string()]
        );
        assert_eq!(
            launch.restored_title_for("Shell"),
            Some("Fix the flaky terminal test")
        );
        assert_eq!(snapshot.primary_text(), "Fix the flaky terminal test");
    }
    #[test]
    fn remote_omp_resume_keeps_the_managed_extension_wrapper() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let address = AgentPaneAddress::new("project", "agent", "omp");
        {
            let mut manager = AgentManager::new(&paths);
            let (launch, _) = manager.prepare_pane(address.clone(), "omp", true).unwrap();
            manager.process_started(launch.instance_id(), 1).unwrap();
            manager
                .ingest_hook_request(AgentHookRequest {
                    address: address.clone(),
                    generation: 1,
                    source: BuiltinAgent::OhMyPi,
                    event: "session_start".to_string(),
                    payload: json!({ "sessionId": "omp-session-1" }),
                })
                .unwrap()
                .unwrap();
        }

        let mut manager = AgentManager::new(&paths);
        let (launch, _) = manager.prepare_pane(address, "omp", true).unwrap();
        assert_eq!(
            launch.additional_args(),
            &["--resume".to_string(), "omp-session-1".to_string()]
        );
        let command = launch.remote_command("omp", &[]).unwrap();
        assert!(command.contains("'--resume' 'omp-session-1'"));
        assert!(command.contains("--extension \"$yttt_agent_path\""));
    }
}
