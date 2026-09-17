use std::{
    collections::{HashMap, HashSet},
    io,
    path::{Path, PathBuf},
    sync::{Arc, LazyLock, Mutex},
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
             if [ ! -d \"$yttt_agent_dir\" ]; then mkdir -p \"$yttt_agent_dir\"; fi && \\
             yttt_agent_path=\"$yttt_agent_dir/{OMP_EXTENSION_FILE_NAME}\" && \\
             if [ ! -f \"$yttt_agent_path\" ] || ! printf '%s' '{encoded}' | base64 -d | cmp -s - \"$yttt_agent_path\"; then \\
             yttt_agent_tmp=\"$yttt_agent_path.$$\" && printf '%s' '{encoded}' | base64 -d > \"$yttt_agent_tmp\" && \\
             mv -f \"$yttt_agent_tmp\" \"$yttt_agent_path\" || exit $?; fi && exec {}",
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
        snapshot: AgentSnapshot,
    },
}

#[derive(Clone, Debug)]
pub struct AgentRuntimeProvisioning {
    key: AgentProvisioningKey,
    omp_extension_path: PathBuf,
}

#[derive(Debug, thiserror::Error)]
pub enum AgentInitializationError {
    #[error("shared Host control is required before provisioning agents")]
    ControlRequired,
    #[error("agent runtime provisioning failed: {0}")]
    Installation(String),
    #[error("agent provisioning does not match this workbench configuration")]
    MismatchedConfiguration,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct AgentProvisioningKey {
    environment_id: Option<String>,
    profile_id: Option<String>,
    config_dir: PathBuf,
}

impl AgentProvisioningKey {
    fn for_paths(config_paths: &AppConfigPaths) -> Self {
        let environment_id = crate::config::storage::environment_storage()
            .map(|storage| storage.environment().environment_id.clone());
        let profile_id = config_paths
            .profile()
            .map(|profile| profile.id().as_str().to_string());
        Self {
            environment_id,
            profile_id,
            config_dir: config_paths.config_dir().to_path_buf(),
        }
    }
}

static PROVISIONED_AGENT_RUNTIMES: LazyLock<Mutex<HashSet<AgentProvisioningKey>>> =
    LazyLock::new(|| Mutex::new(HashSet::new()));

pub struct AgentManager {
    runtime: AgentRuntime,
    launches_by_address: HashMap<AgentPaneAddress, AgentPaneLaunch>,
    addresses_by_instance: HashMap<AgentInstanceId, AgentPaneAddress>,
    snapshot_client: Option<AgentSnapshotClient>,
    host_snapshot_sequences: HashMap<AgentPaneAddress, (u64, u64, u64)>,
    provisioning_key: AgentProvisioningKey,
    omp_extension_path: PathBuf,
    agent_runtime_initialized: bool,
    retained_snapshots: HashMap<AgentPaneAddress, AgentSnapshot>,
    restorable_projects: HashSet<String>,
    omp_extension_base64: Arc<str>,
    state_load_error: Option<String>,
}

impl AgentManager {
    pub fn new(config_paths: &AppConfigPaths) -> Self {
        let mut runtime = AgentRuntime::default();
        for provider in builtin_providers() {
            runtime.register_provider(provider);
        }
        let state_path = config_paths.agent_state_path();
        let (retained_snapshots, state_load_error) = match load_agent_state(&state_path) {
            Ok(state) => (state, None),
            Err(error) => (HashMap::new(), Some(error.to_string())),
        };
        let omp_extension_path = omp_extension_path(config_paths);
        let provisioning_key = AgentProvisioningKey::for_paths(config_paths);
        let omp_extension_base64 = Arc::<str>::from(STANDARD.encode(OMP_EXTENSION_SOURCE));
        Self {
            runtime,
            launches_by_address: HashMap::new(),
            addresses_by_instance: HashMap::new(),
            snapshot_client: None,
            host_snapshot_sequences: HashMap::new(),
            omp_extension_path,
            provisioning_key,
            agent_runtime_initialized: false,
            retained_snapshots,
            restorable_projects: HashSet::new(),
            omp_extension_base64,
            state_load_error,
        }
    }

    /// Performs Host-authorized provisioning and is intended for a background executor.
    pub fn provision(
        config_paths: AppConfigPaths,
        runtime: Arc<crate::host_runtime::DesktopHostRuntime>,
    ) -> Result<AgentRuntimeProvisioning, AgentInitializationError> {
        Self::provision_with_authority(&config_paths, || runtime.shared_editing_enabled())
    }

    fn provision_with_authority(
        config_paths: &AppConfigPaths,
        mut shared_editing_enabled: impl FnMut() -> bool,
    ) -> Result<AgentRuntimeProvisioning, AgentInitializationError> {
        if !shared_editing_enabled() {
            return Err(AgentInitializationError::ControlRequired);
        }

        let key = AgentProvisioningKey::for_paths(config_paths);
        let mut provisioned = PROVISIONED_AGENT_RUNTIMES
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if provisioned.contains(&key) {
            return Ok(AgentRuntimeProvisioning {
                key,
                omp_extension_path: omp_extension_path(config_paths),
            });
        }

        let mut errors = Vec::new();
        if let Err(error) = install_managed_hooks(config_paths) {
            errors.push(format!("agent hook adapters: {error}"));
        }
        if !shared_editing_enabled() {
            return Err(AgentInitializationError::ControlRequired);
        }
        if let Err(error) = install_omp_extension(config_paths) {
            errors.push(format!("OMP extension: {error}"));
        }
        if !shared_editing_enabled() {
            return Err(AgentInitializationError::ControlRequired);
        }
        if !errors.is_empty() {
            return Err(AgentInitializationError::Installation(errors.join("; ")));
        }

        provisioned.insert(key.clone());
        Ok(AgentRuntimeProvisioning {
            key,
            omp_extension_path: omp_extension_path(config_paths),
        })
    }

    /// Applies successful background provisioning to this workbench without I/O.
    pub fn ensure_initialized(
        &mut self,
        provisioning: AgentRuntimeProvisioning,
    ) -> Result<(), AgentInitializationError> {
        if provisioning.key != self.provisioning_key
            || provisioning.omp_extension_path != self.omp_extension_path
        {
            return Err(AgentInitializationError::MismatchedConfiguration);
        }
        self.agent_runtime_initialized = true;
        Ok(())
    }

    pub fn is_initialized(&self) -> bool {
        self.agent_runtime_initialized
    }
    /// Whether a fresh command needs Host agent provisioning before it can launch.
    ///
    /// Existing Host terminal attachments must bypass this check: observation never provisions.
    pub fn requires_initialization(&self, command: &str) -> bool {
        self.runtime.matches_command(command)
    }

    pub fn state_load_error(&self) -> Option<&str> {
        self.state_load_error.as_deref()
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
        self.restorable_projects = snapshots
            .iter()
            .map(|(address, _)| address.project_id.clone())
            .collect();
        self.retained_snapshots = snapshots.into_iter().collect();
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

        self.retained_snapshots
            .retain(|address, _| !matches(address));
        self.host_snapshot_sequences
            .retain(|address, _| !matches(address));
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
        mut update: AgentSnapshotUpdate,
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
        let session_confirmed = update.snapshot.session.is_some();
        let resuming = self
            .launches_by_address
            .get(&address)
            .is_some_and(AgentPaneLaunch::is_resuming_session);
        if resuming && !session_confirmed {
            update.snapshot.session = self
                .retained_snapshots
                .get(&address)
                .and_then(|previous| previous.session.clone());
        }

        let snapshot_failed =
            update.snapshot.view_state() == yttt_agent_core::AgentViewState::Failed;
        let resume_failed = snapshot_failed && resuming;
        if resume_failed {
            let snapshot = update.snapshot;
            self.retain_snapshot(address.clone(), snapshot.clone());
            return Some(AgentPaneExitOutcome::ResumeFailed { address, snapshot });
        }
        if !snapshot_failed
            && session_confirmed
            && let Some(launch) = self.launches_by_address.get_mut(&address)
        {
            launch.resuming_session = false;
        }

        self.retain_snapshot(address.clone(), update.snapshot.clone());
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
        let restored = self
            .restorable_projects
            .contains(&address.project_id)
            .then(|| self.retained_snapshots.get(&address).cloned())
            .flatten();
        let (prepared, _) = self.runtime.prepare_launch_with_snapshot(
            command,
            address.scope_key(),
            restored.as_ref(),
        )?;
        let restored_for_view = restored.as_ref().map(disconnected_snapshot);
        let mut additional_args = prepared.resume_arguments().to_vec();
        let resuming_session = !prepared.resume_arguments().is_empty();
        let is_omp = prepared.provider_id.as_str() == OMP_PROVIDER_ID;
        if is_omp && !remote {
            additional_args.push("--extension".to_string());
            additional_args.push(self.omp_extension_path.to_string_lossy().into_owned());
        }
        let launch = AgentPaneLaunch {
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

    fn retain_snapshot(&mut self, address: AgentPaneAddress, snapshot: AgentSnapshot) {
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

fn omp_extension_path(config_paths: &AppConfigPaths) -> PathBuf {
    config_paths
        .agent_provider_dir(OMP_PROVIDER_ID)
        .join(OMP_EXTENSION_FILE_NAME)
}

fn install_omp_extension(config_paths: &AppConfigPaths) -> std::io::Result<()> {
    let path = omp_extension_path(config_paths);
    let directory = path
        .parent()
        .expect("OMP extension path has an agent provider directory");
    fs::create_dir_all(directory)?;
    let source = OMP_EXTENSION_SOURCE.as_bytes();
    if fs::read(&path).ok().as_deref() != Some(source) {
        atomic_write(&path, source)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[cfg(unix)]
    use std::{os::unix::fs::MetadataExt as _, process::Command};

    use tempfile::TempDir;
    use yttt_agent_core::{AgentReducer, AgentViewState, ProviderId};
    use yttt_core::model::ids::TerminalSessionId;
    use yttt_protocol::agent::{AgentHookScope, AgentSnapshotUpdate};

    use super::*;

    #[test]
    fn host_restore_resumes_saved_agent_and_keeps_unresumable_session() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);
        let address = AgentPaneAddress::new("project", "tab", "agent");
        let reducer = AgentReducer::new(
            AgentInstanceId::random(),
            ProviderId::from_static("codex"),
            1,
        );
        let mut snapshot = reducer.snapshot().clone();
        snapshot.process_state = AgentProcessState::Exited;
        snapshot.session = Some(yttt_agent_core::AgentSessionMetadata {
            session_id: Some("saved-session".into()),
            ..Default::default()
        });
        manager.reset_for_host_restore(vec![(address.clone(), snapshot.clone())]);
        let (launch, _) = manager
            .prepare_pane(address.clone(), "codex", false)
            .unwrap();
        assert_eq!(launch.program_override(), Some("codex"));
        assert_eq!(launch.additional_args(), ["resume", "saved-session"]);

        snapshot.session.as_mut().unwrap().session_id = None;
        manager.reset_for_host_restore(vec![(address.clone(), snapshot)]);
        assert!(
            manager
                .prepare_pane(address.clone(), "codex", false)
                .is_none()
        );
        assert!(manager.has_retained_snapshot(&address));
    }

    #[test]
    fn constructor_does_not_provision_agent_files() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());

        let manager = AgentManager::new(&paths);

        assert!(!manager.is_initialized());
        assert!(manager.state_load_error().is_none());
        assert!(!paths.agent_provider_dir(OMP_PROVIDER_ID).exists());
    }

    #[test]
    fn provisioning_requires_control_without_creating_extension_files() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());

        assert!(matches!(
            AgentManager::provision_with_authority(&paths, || false),
            Err(AgentInitializationError::ControlRequired)
        ));
        assert!(!paths.agent_provider_dir(OMP_PROVIDER_ID).exists());
    }

    #[test]
    fn prepares_omp_after_explicit_idempotent_provisioning() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut manager = AgentManager::new(&paths);

        let provisioning = AgentManager::provision_with_authority(&paths, || true).unwrap();
        manager.ensure_initialized(provisioning).unwrap();
        assert!(manager.is_initialized());

        let mut second_manager = AgentManager::new(&paths);
        let repeat = AgentManager::provision_with_authority(&paths, || true).unwrap();
        second_manager.ensure_initialized(repeat).unwrap();
        assert!(second_manager.is_initialized());

        let address = AgentPaneAddress::new("project", "agent", "omp");
        let (first, _) = manager.prepare_pane(address.clone(), "omp", false).unwrap();
        assert_eq!(first.additional_args()[0], "--extension");
        assert_eq!(
            PathBuf::from(&first.additional_args()[1]),
            omp_extension_path(&paths)
        );
        assert_eq!(
            fs::read(omp_extension_path(&paths)).unwrap(),
            OMP_EXTENSION_SOURCE.as_bytes()
        );
        let (second, _) = manager.prepare_pane(address, "omp", false).unwrap();
        assert_eq!(first.instance_id(), second.instance_id());
    }

    #[test]
    fn requires_initialization_only_for_matching_fresh_agent_commands() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let manager = AgentManager::new(&paths);

        assert!(manager.requires_initialization("omp"));
        assert!(!manager.requires_initialization("zsh"));
    }

    #[cfg(unix)]
    #[test]
    fn remote_omp_command_reuses_matching_extension_content() {
        let temp = TempDir::new().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let mut manager = AgentManager::new(&paths);
        let address = AgentPaneAddress::new("project", "agent", "omp");
        let (launch, _) = manager.prepare_pane(address, "omp", true).unwrap();
        let command = launch.remote_command("true", &[]).unwrap();

        let first = Command::new("/bin/sh")
            .arg("-c")
            .arg(&command)
            .env("HOME", temp.path())
            .status()
            .unwrap();
        assert!(first.success());
        let extension = temp
            .path()
            .join(".config/yttt/agent-providers/omp")
            .join(OMP_EXTENSION_FILE_NAME);
        let inode = std::fs::metadata(&extension).unwrap().ino();

        let second = Command::new("/bin/sh")
            .arg("-c")
            .arg(&command)
            .env("HOME", temp.path())
            .status()
            .unwrap();
        assert!(second.success());
        assert_eq!(std::fs::metadata(extension).unwrap().ino(), inode);
    }
    #[test]
    fn applies_monotonic_host_snapshots_without_writing_client_mirrors() {
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

        assert!(
            manager
                .retained_snapshots()
                .iter()
                .any(|(retained_address, _)| retained_address == &address)
        );
        assert!(!paths.agent_state_path().exists());
    }
}
