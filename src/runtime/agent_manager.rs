use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::Arc,
};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use yttt_agent_core::{
    AgentInstanceId, AgentProcessState, AgentSessionMetadata, AgentSnapshot, ProviderResumeCommand,
};
use yttt_agent_providers::{
    OMP_EXTENSION_FILE_NAME, OMP_EXTENSION_SOURCE, OMP_PROVIDER_ID, builtin_providers,
};
use yttt_agent_runtime::{AgentRuntime, AgentScopeKey, PreparedAgentLaunch};
use yttt_protocol::agent::AgentSnapshotUpdate;

use crate::config::storage as fs;
use crate::{
    config::{atomic_write, paths::AppConfigPaths},
    runtime::agent_hooks::{AgentSnapshotClient, installer::install_managed_hooks},
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
    forced_program_override: Option<&'static str>,
    additional_args: Vec<String>,
    remote_extension_base64: Option<Arc<str>>,
    resuming_session: bool,
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

    pub fn is_resuming_session(&self) -> bool {
        self.resuming_session
    }

    pub fn program_override(&self) -> Option<&str> {
        self.static_program_override()
    }

    fn static_program_override(&self) -> Option<&'static str> {
        self.forced_program_override
            .or_else(|| self.prepared.program_override())
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

#[allow(clippy::large_enum_variant)]
#[derive(Clone, Debug)]
pub enum AgentPaneExitOutcome {
    Snapshot {
        address: AgentPaneAddress,
        snapshot: AgentSnapshot,
    },
    ResumeFailed {
        address: AgentPaneAddress,
    },
}

pub struct AgentManager {
    runtime: AgentRuntime,
    launches_by_address: HashMap<AgentPaneAddress, AgentPaneLaunch>,
    addresses_by_instance: HashMap<AgentInstanceId, AgentPaneAddress>,
    snapshot_client: Option<AgentSnapshotClient>,
    host_snapshot_sequences: HashMap<AgentPaneAddress, (u64, u64, u64)>,
    omp_extension_path: Option<PathBuf>,
    state_path: PathBuf,
    retained_snapshots: HashMap<AgentPaneAddress, AgentSnapshot>,
    restorable_projects: HashSet<String>,
    fresh_program_overrides: HashMap<AgentPaneAddress, &'static str>,
    omp_extension_base64: Arc<str>,
    last_error: Option<String>,
    setup_error: Option<String>,
}

impl AgentManager {
    pub fn new(config_paths: &AppConfigPaths) -> Self {
        let mut runtime = AgentRuntime::default();
        let providers = builtin_providers();
        for provider in providers {
            runtime.register_provider(provider);
        }
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
        let setup_error =
            combine_errors(combine_errors(extension_error, state_error), adapter_error);
        let omp_extension_base64 = Arc::<str>::from(STANDARD.encode(OMP_EXTENSION_SOURCE));
        Self {
            runtime,
            launches_by_address: HashMap::new(),
            addresses_by_instance: HashMap::new(),
            snapshot_client: None,
            host_snapshot_sequences: HashMap::new(),
            omp_extension_path,
            state_path,
            retained_snapshots,
            restorable_projects: HashSet::new(),
            fresh_program_overrides: HashMap::new(),
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
        self.runtime.resume_command(provider_id, session)
    }

    pub fn retained_snapshots(&self) -> Vec<(AgentPaneAddress, AgentSnapshot)> {
        self.retained_snapshots
            .iter()
            .map(|(address, snapshot)| (address.clone(), disconnected_snapshot(snapshot)))
            .collect()
    }

    pub fn disconnected_snapshot(&self, address: &AgentPaneAddress) -> Option<AgentSnapshot> {
        self.retained_snapshots
            .get(address)
            .map(disconnected_snapshot)
    }

    pub fn has_retained_snapshot(&self, address: &AgentPaneAddress) -> bool {
        self.restorable_projects.contains(&address.project_id)
            && self.retained_snapshots.contains_key(address)
    }

    pub fn enable_project_session_restore(&mut self, project_id: &str) {
        self.restorable_projects.insert(project_id.to_string());
    }

    pub fn reset_project_sessions(&mut self, project_id: &str) {
        self.restorable_projects.remove(project_id);
        self.forget_matching(|address| address.project_id == project_id);
    }

    pub(crate) fn reset_for_host_restore(
        &mut self,
        snapshots: Vec<(AgentPaneAddress, AgentSnapshot)>,
    ) {
        self.runtime = AgentRuntime::default();
        for provider in builtin_providers() {
            self.runtime.register_provider(provider);
        }
        self.launches_by_address.clear();
        self.addresses_by_instance.clear();
        self.host_snapshot_sequences.clear();
        self.restorable_projects.clear();
        self.fresh_program_overrides.clear();
        self.retained_snapshots = snapshots.into_iter().collect();
        self.last_error = None;
    }

    pub fn forget_tabs(&mut self, project_id: &str, tab_ids: &[String]) {
        self.forget_matching(|address| {
            address.project_id == project_id
                && tab_ids.iter().any(|tab_id| tab_id == &address.tab_id)
        });
    }

    pub fn forget_pane(&mut self, address: &AgentPaneAddress) {
        self.forget_matching(|candidate| candidate == address);
    }

    fn forget_matching(&mut self, mut matches: impl FnMut(&AgentPaneAddress) -> bool) {
        self.fresh_program_overrides
            .retain(|address, _| !matches(address));
        self.launches_by_address
            .retain(|address, _| !matches(address));

        let mut removed_instances = Vec::new();
        self.addresses_by_instance.retain(|instance_id, address| {
            if matches(address) {
                removed_instances.push(instance_id.clone());
                false
            } else {
                true
            }
        });
        for instance_id in removed_instances {
            self.runtime.remove(&instance_id);
        }

        let retained_count = self.retained_snapshots.len();
        self.retained_snapshots
            .retain(|address, _| !matches(address));
        self.host_snapshot_sequences
            .retain(|address, _| !matches(address));
        if self.retained_snapshots.len() != retained_count
            && let Err(error) = write_agent_state(&self.state_path, &self.retained_snapshots)
        {
            self.last_error = Some(error.to_string());
        }
    }

    pub fn take_error(&mut self) -> Option<String> {
        self.last_error.take()
    }
    pub fn set_snapshot_client(&mut self, client: Option<AgentSnapshotClient>) {
        self.snapshot_client = client;
        self.host_snapshot_sequences.clear();
    }

    pub fn snapshot_client(&self) -> Option<AgentSnapshotClient> {
        self.snapshot_client.clone()
    }

    pub fn apply_host_snapshot(
        &mut self,
        address: AgentPaneAddress,
        update: AgentSnapshotUpdate,
    ) -> Option<AgentPaneExitOutcome> {
        let cursor = (update.host_epoch, update.scope.generation, update.sequence);
        if self
            .host_snapshot_sequences
            .get(&address)
            .is_some_and(|current| *current >= cursor)
        {
            return None;
        }
        self.host_snapshot_sequences.insert(address.clone(), cursor);

        let snapshot_failed =
            update.snapshot.view_state() == yttt_agent_core::AgentViewState::Failed;
        let resume_failed = snapshot_failed
            && self
                .launches_by_address
                .get(&address)
                .is_some_and(AgentPaneLaunch::is_resuming_session);
        if resume_failed {
            let launch = self.launches_by_address.remove(&address)?;
            self.addresses_by_instance.remove(launch.instance_id());
            self.runtime.remove(launch.instance_id());
            self.retained_snapshots.remove(&address);
            if let Some(program) = launch.static_program_override() {
                self.fresh_program_overrides
                    .insert(address.clone(), program);
            }
            if let Err(error) = write_agent_state(&self.state_path, &self.retained_snapshots) {
                self.last_error = Some(error.to_string());
            }
            return Some(AgentPaneExitOutcome::ResumeFailed { address });
        }
        if !snapshot_failed && let Some(launch) = self.launches_by_address.get_mut(&address) {
            launch.resuming_session = false;
        }

        self.persist_snapshot(address.clone(), update.snapshot.clone());
        Some(AgentPaneExitOutcome::Snapshot {
            address,
            snapshot: update.snapshot,
        })
    }

    pub fn prepare_pane(
        &mut self,
        address: AgentPaneAddress,
        command: &str,
        remote: bool,
    ) -> Option<(AgentPaneLaunch, Option<AgentSnapshot>)> {
        if let Some(launch) = self.launches_by_address.get(&address) {
            let restored = self
                .restorable_projects
                .contains(&address.project_id)
                .then(|| {
                    self.retained_snapshots
                        .get(&address)
                        .map(disconnected_snapshot)
                })
                .flatten();
            return Some((launch.clone(), restored));
        }
        let forced_program_override = self.fresh_program_overrides.remove(&address);
        let provider_command = forced_program_override.unwrap_or(command);
        let restored = self
            .restorable_projects
            .contains(&address.project_id)
            .then(|| self.retained_snapshots.get(&address).cloned())
            .flatten();
        let Some((prepared, _)) = self.runtime.prepare_launch_with_snapshot(
            provider_command,
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
        let restored_for_view = restored.as_ref().map(disconnected_snapshot);
        let resuming_session = !prepared.resume_arguments().is_empty();
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
            forced_program_override,
            prepared,
            additional_args,
            remote_extension_base64: (is_omp && remote).then(|| self.omp_extension_base64.clone()),
            resuming_session,
        };
        self.addresses_by_instance
            .insert(launch.instance_id().clone(), address.clone());
        self.launches_by_address.insert(address, launch.clone());
        Some((launch, restored_for_view))
    }

    pub fn process_start_failed(
        &mut self,
        instance_id: &AgentInstanceId,
        _generation: u64,
    ) -> Option<AgentPaneAddress> {
        let address = self.addresses_by_instance.get(instance_id)?.clone();
        if !self
            .launches_by_address
            .get(&address)
            .is_some_and(|launch| launch.instance_id() == instance_id)
        {
            return None;
        }
        self.retained_snapshots.remove(&address);
        if let Err(error) = write_agent_state(&self.state_path, &self.retained_snapshots) {
            self.last_error = Some(error.to_string());
        }
        Some(address)
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

fn disconnected_snapshot(snapshot: &AgentSnapshot) -> AgentSnapshot {
    let mut snapshot = snapshot.clone();
    snapshot.mark_disconnected();
    snapshot
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
    use tempfile::TempDir;
    use yttt_agent_core::{AgentReducer, AgentViewState, ProviderId};
    use yttt_core::model::ids::TerminalSessionId;
    use yttt_protocol::agent::{AgentHookScope, AgentSnapshotUpdate};

    use super::*;

    #[test]
    fn prepares_omp_with_managed_extension_and_stable_identity() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        assert!(manager.setup_error().is_none());
        let address = AgentPaneAddress::new("project", "agent", "omp");
        let (first, _) = manager.prepare_pane(address.clone(), "omp", false).unwrap();
        assert_eq!(first.additional_args()[0], "--extension");
        assert!(PathBuf::from(&first.additional_args()[1]).is_file());
        let (second, _) = manager.prepare_pane(address, "omp", false).unwrap();
        assert_eq!(first.instance_id(), second.instance_id());
    }

    #[test]
    fn applies_only_monotonic_host_snapshots_and_persists_them() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        let scope = AgentHookScope {
            project_id: "project".to_string(),
            tab_id: "tab".to_string(),
            pane_id: "pane".to_string(),
            generation: 3,
        };
        let address = AgentPaneAddress::new("project", "tab", "pane");
        let mut reducer = AgentReducer::new(
            AgentInstanceId::new("agent").unwrap(),
            ProviderId::new("codex").unwrap(),
            1,
        );
        reducer.process_starting(3, 1);
        reducer.process_started(3, 2);
        let update = AgentSnapshotUpdate {
            scope,
            terminal_session_id: TerminalSessionId::new("terminal"),
            host_epoch: 4,
            sequence: 2,
            snapshot: reducer.snapshot().clone(),
        };

        let Some(AgentPaneExitOutcome::Snapshot { address, snapshot }) =
            manager.apply_host_snapshot(address.clone(), update.clone())
        else {
            panic!("new Host snapshot was not applied");
        };
        assert_eq!(address, AgentPaneAddress::new("project", "tab", "pane"));
        assert_eq!(snapshot.view_state(), AgentViewState::Idle);
        assert!(
            manager
                .apply_host_snapshot(address.clone(), update)
                .is_none()
        );

        let restored = load_agent_state(&paths.agent_state_path()).unwrap();
        assert_eq!(restored.get(&address), Some(&snapshot));
    }
}
