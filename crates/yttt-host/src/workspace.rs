use std::{
    collections::{HashMap, HashSet},
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Component, Path, PathBuf},
    sync::Arc,
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use yttt_agent_core::AgentSnapshot;
use yttt_core::model::{
    editor::DocumentId,
    host_workspace::{
        HostWorkspace, HostWorkspaceState, IntentError, LegacyWorkspaceSnapshot, PaneAddress,
        PaneLaunchTarget, WorkspaceIntent,
    },
    ids::{ClientInstanceId, ProjectId},
    project::ProjectLocation,
    workspace::{Workspace, WorkspaceError, WorkspaceState},
};
use yttt_protocol::{
    FailureCode, HostPath, PathSegment, ProjectRelativePath, ProtocolFailure,
    workspace::{
        AgentSessionOperation, AgentStateCleared, AgentStateRef, DocumentLease, DocumentUpdate,
        DraftContentRevision, DraftRef, WorkspaceConfig, WorkspaceConfigRevision,
        WorkspaceDirectory, WorkspaceDirectoryEntry, WorkspaceDirectoryEntryKind,
        WorkspaceEnvironment, WorkspaceId, WorkspaceLayoutUpdate, WorkspaceProjectConfig,
        WorkspaceProjectConfigFile, WorkspaceRequest, WorkspaceResponse, WorkspaceSummary,
        encode_project_path,
    },
};

const WORKSPACE_SCHEMA_VERSION: u16 = 4;
/// Suffix of a pre-schema-4 record kept beside the record migrated from it.
const LEGACY_RECORD_SUFFIX: &str = ".schema3.json";
const MAX_WORKSPACES: usize = 128;
const MAX_WORKSPACE_STATE_BYTES: usize = 24 * 1024 * 1024;
const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_BROWSE_ENTRIES: usize = 4096;

/// Host-owned persistent remote workspace state for one profile.
///
/// All mutations run under one mutex. A successful mutation has first been serialized, fsynced,
/// atomically replaced, and directory-fsynced; only then is its in-memory state made visible.
pub struct WorkspaceService {
    state_root: PathBuf,
    config_root: PathBuf,
    project_config: WorkspaceProjectConfig,
    workspace_root: PathBuf,
    environment_id: String,
    drafts: crate::workspace_objects::WorkspaceObjects<DraftRef>,
    agent_states: crate::workspace_objects::WorkspaceObjects<AgentStateRef>,
    backups: crate::workspace_backups::WorkspaceBackups,
    state: Mutex<ServiceState>,
}

struct ServiceState {
    workspaces: HashMap<WorkspaceId, WorkspaceRecord>,
    next_temporary_file: u64,
    history_initialized: bool,
    subscribers: HashMap<ClientInstanceId, WorkspaceSubscriber>,
    next_subscriber: u64,
    /// Projects whose panes this Host run already restored; see
    /// [`HostWorkspace::claim_launches`].
    restored_projects: HashSet<(WorkspaceId, ProjectId)>,
    /// Edit leases of open documents. They are not stored: a lease belongs
    /// to a Client session, which does not outlive the Host.
    leases: HashMap<(WorkspaceId, DocumentId), DocumentLease>,
    /// Epoch of the next lease grant. Starts at the Host's start time, so a
    /// grant never repeats an epoch a Client holds from an earlier Host run.
    next_lease_epoch: u64,
}

/// A change sent to the Clients that opened a workspace.
#[derive(Clone, Copy)]
pub(crate) enum WorkspaceEvent<'a> {
    Layout(&'a WorkspaceLayoutUpdate),
    Document(&'a DocumentUpdate),
    AgentStateCleared(&'a AgentStateCleared),
}

/// Delivers a workspace change to one Client; returns `false` once the Client is gone.
pub(crate) type WorkspaceSink = Box<dyn Fn(WorkspaceEvent<'_>) -> bool + Send>;

struct WorkspaceSubscriber {
    generation: u64,
    sink: WorkspaceSink,
    workspaces: HashSet<WorkspaceId>,
}

/// Keeps a Client's workspace subscription; dropping it unsubscribes.
pub(crate) struct WorkspaceSubscription {
    service: Arc<WorkspaceService>,
    client_id: ClientInstanceId,
    generation: u64,
}

impl Drop for WorkspaceSubscription {
    fn drop(&mut self) {
        let mut state = self.service.state.lock();
        if state
            .subscribers
            .get(&self.client_id)
            .is_some_and(|subscriber| subscriber.generation == self.generation)
        {
            state.subscribers.remove(&self.client_id);
        }
    }
}

/// An accepted layout intent.
pub(crate) struct AppliedIntent {
    pub(crate) revision: u64,
    /// Panes the intent removed; their terminals must be terminated.
    pub(crate) removed_panes: Vec<PaneAddress>,
}

/// The uploaded draft of one open document.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct StoredDocument {
    document: DocumentId,
    draft: DraftRef,
}

#[derive(Clone)]
struct WorkspaceRecord {
    /// The layout; its panes carry the Agent snapshots the Host recorded.
    layout: HostWorkspace,
    /// Drafts of documents open in `layout`.
    documents: Vec<StoredDocument>,
    /// The stored Agent snapshots of `layout`'s panes.
    agent_states: Vec<AgentStateRef>,
    name: String,
    saved_millis: u64,
    restore_on_startup: bool,
    /// Agent snapshots in `layout` changed since `agent_states` was stored.
    agents_changed: bool,
}

#[derive(Serialize, Deserialize)]
struct PersistedWorkspaceRecord {
    schema_version: u16,
    layout: HostWorkspaceState,
    #[serde(default)]
    documents: Vec<StoredDocument>,
    #[serde(default)]
    agent_states: Vec<AgentStateRef>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    saved_millis: u64,
    restore_on_startup: bool,
}

impl WorkspaceRecord {
    fn empty() -> Self {
        Self {
            layout: HostWorkspace::new(),
            documents: Vec::new(),
            agent_states: Vec::new(),
            name: String::new(),
            saved_millis: 0,
            restore_on_startup: true,
            agents_changed: false,
        }
    }

    fn revision(&self) -> u64 {
        self.layout.revision()
    }

    fn draft_refs(&self) -> Vec<DraftRef> {
        self.documents
            .iter()
            .map(|stored| stored.draft.clone())
            .collect()
    }

    fn draft(&self, document: &DocumentId) -> Option<&DraftRef> {
        self.documents
            .iter()
            .find(|stored| &stored.document == document)
            .map(|stored| &stored.draft)
    }

    fn persisted(&self) -> PersistedWorkspaceRecord {
        PersistedWorkspaceRecord {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            layout: self.layout.state(),
            documents: self.documents.clone(),
            agent_states: self.agent_states.clone(),
            name: self.name.clone(),
            saved_millis: self.saved_millis,
            restore_on_startup: self.restore_on_startup,
        }
    }

    fn from_persisted(persisted: PersistedWorkspaceRecord) -> Result<Self, io::Error> {
        let layout = HostWorkspace::restore(persisted.layout)
            .map_err(|error| invalid_state(&format!("invalid workspace layout: {error}")))?;
        Ok(Self {
            layout,
            documents: persisted.documents,
            agent_states: persisted.agent_states,
            name: persisted.name,
            saved_millis: persisted.saved_millis,
            restore_on_startup: persisted.restore_on_startup,
            agents_changed: false,
        })
    }
}

impl WorkspaceService {
    /// Creates a workspace service rooted in the supplied profile-persistent state directory.
    ///
    /// Workspace files live in state_root; config_root is the existing environment config,
    /// supplied explicitly by the desktop or standalone Server bootstrap.
    pub fn new(
        state_root: impl AsRef<Path>,
        config_root: impl AsRef<Path>,
        project_config: WorkspaceProjectConfig,
    ) -> io::Result<Self> {
        let state_root = state_root.as_ref().to_path_buf();
        let config_root = config_root.as_ref().to_path_buf();
        let workspace_root = state_root.join("workspaces");
        fs::create_dir_all(&config_root)?;
        fs::create_dir_all(&workspace_root)?;
        ensure_directory_not_symlink(&config_root)?;
        ensure_directory_not_symlink(&workspace_root)?;
        let identity_path = state_root.join("environment-id");
        let environment_id = match fs::read_to_string(&identity_path) {
            Ok(identity) => {
                uuid::Uuid::parse_str(&identity)
                    .map_err(|_| invalid_state("invalid persisted environment identity"))?;
                identity
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let identity = uuid::Uuid::new_v4().to_string();
                atomic_write(&identity_path, identity.as_bytes(), random_temporary_file())?;
                identity
            }
            Err(error) => return Err(error),
        };

        let drafts = crate::workspace_objects::WorkspaceObjects::new(&state_root)?;
        let agent_states = crate::workspace_objects::WorkspaceObjects::new(&state_root)?;
        let backups = crate::workspace_backups::WorkspaceBackups::new(&state_root);
        let mut workspaces = HashMap::new();
        let history_marker = state_root.join("workspace-history-initialized");
        let mut history_initialized = history_marker.try_exists()?;
        for entry in fs::read_dir(&workspace_root)? {
            let entry = entry?;
            let path = entry.path();
            let file_type = entry.file_type()?;
            if !file_type.is_file() {
                return Err(invalid_state(
                    "workspace state directory contains a non-file entry",
                ));
            }
            let Some(name) = path.file_name().and_then(OsStr::to_str) else {
                return Err(invalid_state("workspace state file name is not UTF-8"));
            };
            if (name.starts_with('.') && name.ends_with(".tmp"))
                || name.ends_with(LEGACY_RECORD_SUFFIX)
            {
                continue;
            }
            let Some(identifier) = name.strip_suffix(".json") else {
                return Err(invalid_state(
                    "workspace state file does not use the .json suffix",
                ));
            };
            let workspace_id = WorkspaceId::new(identifier)
                .map_err(|_| invalid_state("workspace state file has an invalid identifier"))?;
            let legacy_path = workspace_root.join(format!("{identifier}{LEGACY_RECORD_SUFFIX}"));
            let load =
                |bytes: &[u8]| load_workspace_record(bytes, &workspace_id, &drafts, &agent_states);
            let loaded = read_bounded_file(&path, MAX_WORKSPACE_STATE_BYTES)
                .map_err(RecordLoadError::Unreadable)
                .and_then(|bytes| load(&bytes).map(|loaded| (loaded, bytes)));
            let (record, rewrite) = match loaded {
                Ok(((record, migrated), bytes)) => {
                    if migrated {
                        // The record it was migrated from stays beside it.
                        atomic_write(&legacy_path, &bytes, random_temporary_file())?;
                    }
                    (record, migrated)
                }
                Err(RecordLoadError::Fatal(error)) => return Err(error),
                Err(RecordLoadError::Migration(error)) => {
                    eprintln!("workspace {identifier} was not migrated and is kept aside: {error}");
                    fs::rename(&path, &legacy_path)?;
                    continue;
                }
                Err(RecordLoadError::Unreadable(error)) => {
                    eprintln!("workspace {identifier} state is unreadable: {error}");
                    let recovered =
                        backups.recover(identifier, &path, crate::now_millis(), |bytes| {
                            load(bytes).map_err(|error| match error {
                                RecordLoadError::Fatal(error)
                                | RecordLoadError::Migration(error)
                                | RecordLoadError::Unreadable(error) => error,
                            })
                        })?;
                    let Some((_, (record, _))) = recovered else {
                        eprintln!("workspace {identifier} has no readable backup");
                        continue;
                    };
                    (record, true)
                }
            };
            if rewrite {
                // All referenced objects are durable before replacing the record.
                let bytes = serialize_workspace_record(&record)
                    .map_err(|error| invalid_state(&error.message))?;
                atomic_write(&path, &bytes, random_temporary_file())?;
            }
            if !history_initialized {
                atomic_write(&history_marker, b"1", random_temporary_file())?;
                history_initialized = true;
            }
            if !record.restore_on_startup && workspace_is_empty(&record) {
                fs::remove_file(&path)?;
                #[cfg(unix)]
                File::open(&workspace_root)?.sync_all()?;
                drafts.remove_workspace(&workspace_id)?;
                agent_states.remove_workspace(&workspace_id)?;
                continue;
            }
            drafts.recover(&workspace_id, &record.draft_refs())?;
            agent_states.recover(&workspace_id, &record.agent_states)?;
            let mut record = record;
            load_agent_snapshots(
                &agent_states,
                &workspace_id,
                &mut record.layout,
                &record.agent_states,
            );
            if workspaces.insert(workspace_id, record).is_some() {
                return Err(invalid_state("duplicate workspace state identifier"));
            }
            if workspaces.len() > MAX_WORKSPACES {
                return Err(invalid_state("workspace state exceeds its workspace limit"));
            }
        }

        Ok(Self {
            state_root,
            config_root,
            project_config,
            workspace_root,
            environment_id,
            drafts,
            agent_states,
            backups,
            state: Mutex::new(ServiceState {
                workspaces,
                next_temporary_file: 0,
                history_initialized,
                subscribers: HashMap::new(),
                next_subscriber: 0,
                restored_projects: HashSet::new(),
                leases: HashMap::new(),
                next_lease_epoch: crate::now_millis(),
            }),
        })
    }

    pub(crate) fn referenced_projects(&self) -> Vec<(yttt_core::model::ids::ProjectId, HostPath)> {
        let state = self.state.lock();
        let mut projects = HashMap::new();
        for record in state.workspaces.values() {
            for project in record.layout.workspace().opened_projects() {
                if let Some(path) = project.location.local_path()
                    && let Ok(path) = HostPath::from_path(path)
                {
                    projects.insert(project.id.clone(), path);
                }
            }
        }
        projects.into_iter().collect()
    }

    /// Returns the profile-persistent state root supplied at construction.
    pub fn state_root(&self) -> &Path {
        &self.state_root
    }

    /// Returns the explicit environment configuration root.
    pub fn config_root(&self) -> &Path {
        &self.config_root
    }

    pub fn environment_id(&self) -> &str {
        &self.environment_id
    }

    /// Handles one request on behalf of a connected client.
    pub fn handle(
        &self,
        client_id: &ClientInstanceId,
        request: WorkspaceRequest,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        let request = match request {
            WorkspaceRequest::Apply { .. } => {
                return Err(failure(
                    FailureCode::Internal,
                    "layout intents are applied with WorkspaceService::apply",
                    false,
                ));
            }
            WorkspaceRequest::AgentSessions {
                providers,
                project_root,
            } => return scan_agent_history(providers, project_root),
            WorkspaceRequest::MutateAgentSession {
                provider,
                session_id,
                project_root,
                operation,
            } => return mutate_agent_history(provider, session_id, project_root, operation),
            WorkspaceRequest::OmpSessionExists { session_id } => {
                let roots = yttt_agent_providers::sessions::AgentSessionRoots::native()
                    .ok_or_else(|| {
                        failure(
                            FailureCode::Internal,
                            "Host home directory is unavailable",
                            false,
                        )
                    })?;
                let exists =
                    yttt_agent_providers::sessions::omp_session_exists(&roots.omp, &session_id)
                        .map_err(|error| {
                            failure(FailureCode::Internal, error.to_string(), false)
                        })?;
                return Ok(WorkspaceResponse::OmpSessionExists(exists));
            }
            request => request,
        };
        let mut state = self.state.lock();
        match request {
            WorkspaceRequest::Apply { .. } => unreachable!("handled before the state lock"),
            WorkspaceRequest::AgentSessions { .. }
            | WorkspaceRequest::MutateAgentSession { .. }
            | WorkspaceRequest::OmpSessionExists { .. } => {
                unreachable!("handled before the state lock")
            }
            WorkspaceRequest::InstallAgentHooks => {
                let home = environment(
                    &self.environment_id,
                    &self.config_root,
                    &self.project_config,
                    state.history_initialized || !state.workspaces.is_empty(),
                )?
                .home
                .to_path()
                .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?;
                yttt_agent_providers::installer::install_managed_hooks_at(&self.config_root, &home)
                    .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?;
                Ok(WorkspaceResponse::AgentHooksInstalled)
            }
            WorkspaceRequest::Environment => environment(
                &self.environment_id,
                &self.config_root,
                &self.project_config,
                state.history_initialized || !state.workspaces.is_empty(),
            )
            .map(WorkspaceResponse::Environment),
            WorkspaceRequest::Browse {
                path,
                include_hidden,
            } => browse(path, include_hidden).map(WorkspaceResponse::Directory),
            WorkspaceRequest::ReadConfig { relative_path } => self
                .read_config(&relative_path)
                .map(WorkspaceResponse::Config),
            WorkspaceRequest::WriteConfig {
                relative_path,
                expected_revision,
                bytes,
            } => self.write_config(relative_path, expected_revision, bytes),
            WorkspaceRequest::ReadProjectConfig { project_root, file } => self
                .read_project_config(project_root, file)
                .map(WorkspaceResponse::Config),
            WorkspaceRequest::WriteProjectConfig {
                project_root,
                file,
                expected_revision,
                bytes,
            } => self.write_project_config(project_root, file, expected_revision, bytes),
            WorkspaceRequest::DeleteProjectConfig {
                project_root,
                file,
                expected_revision,
            } => self.delete_project_config(project_root, file, expected_revision),
            WorkspaceRequest::ListConfig { relative_directory } => {
                let path = self.config_directory(&relative_directory, false)?;
                let mut entries = Vec::new();
                match fs::read_dir(path) {
                    Ok(directory) => {
                        for entry in directory {
                            let entry = entry
                                .map_err(|error| filesystem_failure("list configuration", error))?;
                            if entries.len() >= MAX_BROWSE_ENTRIES {
                                return Err(failure(
                                    FailureCode::ResourceLimit,
                                    "too many configuration entries",
                                    false,
                                ));
                            }
                            let mut relative = relative_directory.clone();
                            relative.segments.push(
                                PathSegment::from_os_str(entry.file_name()).map_err(|error| {
                                    failure(FailureCode::InvalidRequest, error.to_string(), false)
                                })?,
                            );
                            if !config_path_is_shared(&relative) {
                                continue;
                            }
                            entries.push(relative);
                        }
                    }
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => return Err(filesystem_failure("list configuration", error)),
                }
                Ok(WorkspaceResponse::ConfigDirectory(entries))
            }
            WorkspaceRequest::CreateConfigDirectory { relative_directory } => {
                self.config_directory(&relative_directory, true)?;
                Ok(WorkspaceResponse::ConfigDirectoryCreated)
            }
            WorkspaceRequest::RemoveConfigDirectory { relative_directory } => {
                validate_config_path(&relative_directory)?;
                let path = self.config_directory(&relative_directory, false)?;
                fs::remove_dir(&path).map_err(|error| {
                    filesystem_failure("remove empty configuration directory", error)
                })?;
                sync_config_parent(&path)?;
                Ok(WorkspaceResponse::ConfigDeleted)
            }
            WorkspaceRequest::RenameConfigDirectory {
                relative_from,
                relative_to,
            } => {
                validate_config_path(&relative_from)?;
                validate_config_path(&relative_to)?;
                let from = self.config_directory(&relative_from, false)?;
                let to = self.config_directory(&relative_to, false)?;
                match fs::symlink_metadata(&to) {
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(error) => {
                        return Err(filesystem_failure(
                            "inspect configuration destination",
                            error,
                        ));
                    }
                    Ok(_) => {
                        return Err(failure(
                            FailureCode::Conflict,
                            "configuration destination already exists",
                            false,
                        ));
                    }
                }
                fs::rename(&from, &to)
                    .map_err(|error| filesystem_failure("move configuration directory", error))?;
                sync_config_parent(&from)?;
                sync_config_parent(&to)?;
                Ok(WorkspaceResponse::ConfigDirectoryRenamed)
            }
            WorkspaceRequest::DeleteConfig {
                relative_path,
                expected_revision,
            } => self.delete_config(&relative_path, expected_revision),
            WorkspaceRequest::Open { workspace_id } => {
                Ok(self.open(&mut state, client_id, workspace_id))
            }
            WorkspaceRequest::OpenExisting { workspace_id } => {
                if !state.workspaces.contains_key(&workspace_id) {
                    return Err(failure(
                        FailureCode::NotFound,
                        "saved workspace no longer exists",
                        false,
                    ));
                }
                Ok(self.open(&mut state, client_id, workspace_id))
            }
            WorkspaceRequest::SetRestoreOnStartup {
                workspace_id,
                expected_revision,
                restore,
            } => {
                if !restore && !state.workspaces.contains_key(&workspace_id) {
                    return Ok(WorkspaceResponse::Forgotten { workspace_id });
                }
                let mut record = state
                    .workspaces
                    .get(&workspace_id)
                    .cloned()
                    .ok_or_else(|| failure(FailureCode::NotFound, "workspace not found", false))?;
                require_revision(&record, expected_revision)?;
                if record.restore_on_startup == restore {
                    return Ok(WorkspaceResponse::Registered(workspace_summary(
                        &workspace_id,
                        &record,
                    )));
                }
                record.restore_on_startup = restore;
                if !restore && workspace_is_empty(&record) {
                    self.forget_workspace(&mut state, &workspace_id)?;
                    return Ok(WorkspaceResponse::Forgotten { workspace_id });
                }
                self.persist_workspace(&mut state, &workspace_id, &record)?;
                let summary = workspace_summary(&workspace_id, &record);
                state.workspaces.insert(workspace_id, record);
                Ok(WorkspaceResponse::Registered(summary))
            }
            WorkspaceRequest::Forget {
                workspace_id,
                expected_revision,
            } => {
                let record = state
                    .workspaces
                    .get(&workspace_id)
                    .ok_or_else(|| failure(FailureCode::NotFound, "workspace not found", false))?;
                require_revision(record, expected_revision)?;
                if record.restore_on_startup {
                    return Err(failure(
                        FailureCode::Conflict,
                        "close the workspace before deleting its history",
                        false,
                    ));
                }
                self.forget_workspace(&mut state, &workspace_id)?;
                Ok(WorkspaceResponse::Forgotten { workspace_id })
            }
            WorkspaceRequest::List => {
                let mut entries = state
                    .workspaces
                    .iter()
                    .map(|(id, record)| workspace_summary(id, record))
                    .collect::<Vec<_>>();
                entries.sort_by(|a, b| a.workspace_id.as_str().cmp(b.workspace_id.as_str()));
                Ok(WorkspaceResponse::Workspaces(entries))
            }
            WorkspaceRequest::Register { workspace_id, name } => {
                if name.is_empty() || name.len() > 256 {
                    return Err(failure(
                        FailureCode::InvalidRequest,
                        "workspace name must be 1–256 bytes",
                        false,
                    ));
                }
                if let Some(record) = state.workspaces.get(&workspace_id) {
                    return Ok(WorkspaceResponse::Registered(workspace_summary(
                        &workspace_id,
                        record,
                    )));
                }
                let mut record = self.workspace_for_mut(&state, &workspace_id)?;
                record.name = name;
                self.persist_workspace(&mut state, &workspace_id, &record)?;
                let summary = workspace_summary(&workspace_id, &record);
                state.workspaces.insert(workspace_id, record);
                Ok(WorkspaceResponse::Registered(summary))
            }
            WorkspaceRequest::AcquireDocument {
                workspace_id,
                document,
                takeover,
            } => self.acquire_document(&mut state, client_id, workspace_id, document, takeover),
            WorkspaceRequest::PutDocumentDraft {
                workspace_id,
                document,
                epoch,
                revision,
                base,
                content,
            } => self.put_document_draft(
                &mut state,
                client_id,
                workspace_id,
                document,
                epoch,
                revision,
                base,
                content,
            ),
            WorkspaceRequest::GetDocumentDraft {
                workspace_id,
                document,
            } => self.get_document_draft(&state, &workspace_id, &document),
            WorkspaceRequest::ReleaseDocument {
                workspace_id,
                document,
                epoch,
            } => self.release_document(&mut state, client_id, workspace_id, document, epoch),
            WorkspaceRequest::GetAgentState {
                workspace_id,
                reference,
            } => {
                if !state
                    .workspaces
                    .get(&workspace_id)
                    .is_some_and(|record| record.agent_states.contains(&reference))
                {
                    return Err(failure(
                        FailureCode::NotFound,
                        "Agent state is not published by this workspace",
                        false,
                    ));
                }
                let content = self
                    .agent_states
                    .read(&workspace_id, &reference)
                    .map_err(|error| filesystem_failure("read Agent state", error))?;
                Ok(WorkspaceResponse::AgentState { reference, content })
            }
        }
    }

    /// Returns a workspace and subscribes the Client to its changes.
    fn open(
        &self,
        state: &mut ServiceState,
        client_id: &ClientInstanceId,
        workspace_id: WorkspaceId,
    ) -> WorkspaceResponse {
        // A Client only reads Agent states at open, so it gets the current ones.
        if state
            .workspaces
            .get(&workspace_id)
            .is_some_and(|record| record.agents_changed)
            && let Err(error) = self.store_agent_states(state, &workspace_id)
        {
            eprintln!(
                "Agent states of workspace {} were not stored: {}",
                workspace_id.as_str(),
                error.message
            );
        }
        let response = match state.workspaces.get(&workspace_id) {
            Some(record) => WorkspaceResponse::Opened {
                layout: record.layout.state(),
                documents: Self::document_states(state, &workspace_id, record),
                agent_states: record.agent_states.clone(),
            },
            None => WorkspaceResponse::Opened {
                layout: HostWorkspace::new().state(),
                documents: Vec::new(),
                agent_states: Vec::new(),
            },
        };
        if let Some(subscriber) = state.subscribers.get_mut(client_id) {
            subscriber.workspaces.insert(workspace_id);
        }
        response
    }

    /// Registers the sink that receives the changes of the workspaces this
    /// Client opens. A later registration for the same Client replaces it and
    /// starts with no opened workspaces.
    pub(crate) fn subscribe(
        self: &Arc<Self>,
        client_id: ClientInstanceId,
        sink: WorkspaceSink,
    ) -> WorkspaceSubscription {
        let mut state = self.state.lock();
        let generation = state.next_subscriber;
        state.next_subscriber += 1;
        state.subscribers.insert(
            client_id.clone(),
            WorkspaceSubscriber {
                generation,
                sink,
                workspaces: HashSet::new(),
            },
        );
        WorkspaceSubscription {
            service: self.clone(),
            client_id,
            generation,
        }
    }

    /// Sends a change to every Client that opened its workspace. Runs under
    /// the state lock, so each Client receives changes in the order they happen.
    fn notify(state: &mut ServiceState, workspace_id: &WorkspaceId, event: WorkspaceEvent<'_>) {
        state.subscribers.retain(|_, subscriber| {
            !subscriber.workspaces.contains(workspace_id) || (subscriber.sink)(event)
        });
    }

    fn notify_layout(state: &mut ServiceState, workspace_id: &WorkspaceId) {
        let Some(record) = state.workspaces.get(workspace_id) else {
            return;
        };
        let update = WorkspaceLayoutUpdate {
            workspace_id: workspace_id.clone(),
            layout: record.layout.state(),
        };
        Self::notify(state, workspace_id, WorkspaceEvent::Layout(&update));
    }

    /// Applies one layout intent in arrival order on behalf of any Client.
    /// A workspace that does not exist yet starts empty, as `Open` shows it.
    pub(crate) fn apply(
        &self,
        workspace_id: WorkspaceId,
        intent: WorkspaceIntent,
        expected_revision: Option<u64>,
    ) -> Result<AppliedIntent, ProtocolFailure> {
        let mut state = self.state.lock();
        let current = self.workspace_for_mut(&state, &workspace_id)?;
        let outcome = self.change_layout(&mut state, &workspace_id, current, |layout| {
            layout
                .apply(intent, expected_revision)
                .map_err(intent_failure)
        })?;
        Ok(AppliedIntent {
            revision: outcome.revision,
            removed_panes: outcome.panes.removed,
        })
    }

    /// Claims the panes the Host must start in a workspace that is open in
    /// some window; see [`HostWorkspace::claim_launches`]. Each project is
    /// restored once per Host run, when `reachable` first accepts its location.
    pub(crate) fn claim_launches(
        &self,
        workspace_id: &WorkspaceId,
        reachable: impl Fn(&ProjectLocation) -> bool,
    ) -> Result<Vec<PaneLaunchTarget>, ProtocolFailure> {
        let mut state = self.state.lock();
        let Some(current) = state
            .workspaces
            .get(workspace_id)
            .filter(|record| record.restore_on_startup)
            .cloned()
        else {
            return Ok(Vec::new());
        };
        let restored = state
            .restored_projects
            .iter()
            .filter(|(id, _)| id == workspace_id)
            .map(|(_, project_id)| project_id.clone())
            .collect::<HashSet<_>>();
        let mut restoring = Vec::new();
        let targets = self.change_layout(&mut state, workspace_id, current, |layout| {
            Ok(layout.claim_launches(|project_id, location| {
                if !reachable(location) {
                    return None;
                }
                let first = !restored.contains(project_id);
                if first {
                    restoring.push(project_id.clone());
                }
                Some(first)
            }))
        })?;
        state.restored_projects.extend(
            restoring
                .into_iter()
                .map(|project_id| (workspace_id.clone(), project_id)),
        );
        Ok(targets)
    }

    /// Workspaces open in some window, whose panes the Host runs.
    pub(crate) fn open_workspaces(&self) -> Vec<WorkspaceId> {
        self.state
            .lock()
            .workspaces
            .iter()
            .filter(|(_, record)| record.restore_on_startup)
            .map(|(id, _)| id.clone())
            .collect()
    }

    /// Every pane in a workspace's layout.
    pub(crate) fn pane_addresses(&self, workspace_id: &WorkspaceId) -> Vec<PaneAddress> {
        let state = self.state.lock();
        let Some(record) = state.workspaces.get(workspace_id) else {
            return Vec::new();
        };
        record
            .layout
            .workspace()
            .opened_projects()
            .iter()
            .flat_map(|project| {
                project.tab_states.iter().flat_map(move |tab| {
                    tab.pane_states.iter().map(move |pane| PaneAddress {
                        project_id: project.id.clone(),
                        tab_id: tab.tab_id.clone(),
                        pane_id: pane.pane_id.clone(),
                    })
                })
            })
            .collect()
    }

    /// Records that a pane's process exited or did not start. Returns the
    /// pane's config, or `None` when it was already exited or is gone.
    pub(crate) fn record_pane_exited(
        &self,
        workspace_id: &WorkspaceId,
        address: &PaneAddress,
    ) -> Result<Option<yttt_core::model::layout::PaneConfig>, ProtocolFailure> {
        let mut state = self.state.lock();
        let Some(current) = state.workspaces.get(workspace_id).cloned() else {
            return Ok(None);
        };
        self.change_layout(&mut state, workspace_id, current, |layout| {
            match layout.record_pane_exited(address) {
                Ok(true) => Ok(layout.pane(address).map(|(config, _)| config.clone())),
                Ok(false) | Err(_) => Ok(None),
            }
        })
    }

    /// Replaces a pane's Agent snapshot with `update(previous)`. It is stored
    /// by the next [`Self::flush_agent_states`]; the layout revision does not move.
    pub(crate) fn update_agent_snapshot(
        &self,
        workspace_id: &WorkspaceId,
        address: &PaneAddress,
        update: impl FnOnce(Option<&AgentSnapshot>) -> Option<AgentSnapshot>,
    ) {
        let mut state = self.state.lock();
        let Some(record) = state.workspaces.get_mut(workspace_id) else {
            return;
        };
        let Some((_, pane)) = record.layout.pane(address) else {
            return;
        };
        let snapshot = update(pane.agent_snapshot.as_ref());
        if snapshot.as_ref() == pane.agent_snapshot.as_ref() {
            return;
        }
        let cleared = snapshot.is_none();
        if record.layout.set_agent_snapshot(address, snapshot).is_ok() {
            record.agents_changed = true;
            if cleared {
                Self::notify_agent_state_cleared(&mut state, workspace_id, address);
            }
        }
    }

    fn notify_agent_state_cleared(
        state: &mut ServiceState,
        workspace_id: &WorkspaceId,
        address: &PaneAddress,
    ) {
        let cleared = AgentStateCleared {
            workspace_id: workspace_id.clone(),
            terminal_session_id: address.terminal_session_id(),
        };
        Self::notify(
            state,
            workspace_id,
            WorkspaceEvent::AgentStateCleared(&cleared),
        );
    }

    /// Stores the Agent snapshots that changed since the last flush.
    pub(crate) fn flush_agent_states(&self) {
        let mut state = self.state.lock();
        let changed = state
            .workspaces
            .iter()
            .filter(|(_, record)| record.agents_changed)
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>();
        for workspace_id in changed {
            if let Err(error) = self.store_agent_states(&mut state, &workspace_id) {
                eprintln!(
                    "Agent states of workspace {} were not stored: {}",
                    workspace_id.as_str(),
                    error.message
                );
            }
        }
    }

    fn store_agent_states(
        &self,
        state: &mut ServiceState,
        workspace_id: &WorkspaceId,
    ) -> Result<(), ProtocolFailure> {
        let mut updated = state.workspaces[workspace_id].clone();
        updated.agents_changed = false;
        let mut references = Vec::new();
        for (address, snapshot) in updated.layout.agent_snapshots() {
            let content = serde_json::to_vec(snapshot).map_err(layout_encoding_failure)?;
            let reference = AgentStateRef::for_content(
                address.project_id,
                address.tab_id,
                address.pane_id,
                &content,
            );
            if !updated.agent_states.contains(&reference) {
                self.agent_states
                    .put(workspace_id, &reference, &content)
                    .map_err(|error| filesystem_failure("persist Agent state", error))?;
            }
            references.push(reference);
        }
        let previous = std::mem::replace(&mut updated.agent_states, references);
        self.persist_workspace(state, workspace_id, &updated)?;
        self.agent_states
            .collect_obsolete(workspace_id, &previous, &updated.agent_states);
        state.workspaces.insert(workspace_id.clone(), updated);
        Ok(())
    }

    /// Changes a workspace's layout. When its revision moves, the layout is
    /// stored and sent to the Clients that opened it; drafts, leases, and
    /// Agent states of documents and panes it no longer holds are dropped.
    fn change_layout<T>(
        &self,
        state: &mut ServiceState,
        workspace_id: &WorkspaceId,
        current: WorkspaceRecord,
        change: impl FnOnce(&mut HostWorkspace) -> Result<T, ProtocolFailure>,
    ) -> Result<T, ProtocolFailure> {
        let mut layout = current.layout.clone();
        let result = change(&mut layout)?;
        if layout.revision() == current.revision() {
            return Ok(result);
        }
        // Agent states of panes the layout keeps but no longer holds a state for.
        let cleared_agents = current
            .layout
            .agent_snapshots()
            .into_iter()
            .map(|(address, _)| address)
            .filter(|address| {
                layout
                    .pane(address)
                    .is_some_and(|(_, pane)| pane.agent_snapshot.is_none())
            })
            .collect::<Vec<_>>();
        let mut updated = current;
        updated.layout = layout;
        let previous_drafts = updated.draft_refs();
        updated
            .documents
            .retain(|stored| updated.layout.contains_document(&stored.document));
        let previous_agents = updated.agent_states.clone();
        let workspace = updated.layout.state().workspace;
        updated
            .agent_states
            .retain(|reference| pane_exists(&workspace, reference));
        updated.saved_millis = crate::now_millis();
        self.persist_workspace(state, workspace_id, &updated)?;
        self.drafts
            .collect_obsolete(workspace_id, &previous_drafts, &updated.draft_refs());
        self.agent_states
            .collect_obsolete(workspace_id, &previous_agents, &updated.agent_states);
        state.leases.retain(|(id, document), _| {
            id != workspace_id || updated.layout.contains_document(document)
        });
        state.workspaces.insert(workspace_id.clone(), updated);
        Self::notify_layout(state, workspace_id);
        for address in &cleared_agents {
            Self::notify_agent_state_cleared(state, workspace_id, address);
        }
        Ok(result)
    }

    fn workspace_for_mut(
        &self,
        state: &ServiceState,
        workspace_id: &WorkspaceId,
    ) -> Result<WorkspaceRecord, ProtocolFailure> {
        if let Some(record) = state.workspaces.get(workspace_id) {
            return Ok(record.clone());
        }
        if state.workspaces.len() >= MAX_WORKSPACES {
            return Err(failure(
                FailureCode::ResourceLimit,
                "workspace limit reached",
                false,
            ));
        }
        Ok(WorkspaceRecord::empty())
    }

    fn forget_workspace(
        &self,
        state: &mut ServiceState,
        workspace_id: &WorkspaceId,
    ) -> Result<(), ProtocolFailure> {
        if !state.history_initialized {
            atomic_write(
                &self.state_root.join("workspace-history-initialized"),
                b"1",
                random_temporary_file(),
            )
            .map_err(|error| {
                filesystem_failure("preserve workspace history initialization", error)
            })?;
            state.history_initialized = true;
        }
        fs::remove_file(self.workspace_path(workspace_id))
            .map_err(|error| filesystem_failure("delete workspace", error))?;
        state.workspaces.remove(workspace_id);
        sync_config_parent(&self.workspace_path(workspace_id))?;
        self.drafts
            .remove_workspace(workspace_id)
            .map_err(|error| filesystem_failure("delete workspace drafts", error))?;
        self.agent_states
            .remove_workspace(workspace_id)
            .map_err(|error| filesystem_failure("delete workspace Agent states", error))?;
        self.backups.remove(workspace_id.as_str());
        Ok(())
    }

    fn persist_workspace(
        &self,
        state: &mut ServiceState,
        workspace_id: &WorkspaceId,
        record: &WorkspaceRecord,
    ) -> Result<(), ProtocolFailure> {
        let bytes = serialize_workspace_record(record)?;
        let path = self.workspace_path(workspace_id);
        self.backups
            .preserve(workspace_id.as_str(), &path, crate::now_millis());
        let temporary_file = state.next_temporary_file;
        state.next_temporary_file = state.next_temporary_file.wrapping_add(1);
        atomic_write(&path, &bytes, temporary_file).map_err(|error| {
            failure(
                FailureCode::Internal,
                format!("failed to persist workspace state: {error}"),
                true,
            )
        })
    }

    fn workspace_path(&self, workspace_id: &WorkspaceId) -> PathBuf {
        self.workspace_root
            .join(format!("{}.json", workspace_id.as_str()))
    }

    fn config_directory(
        &self,
        relative: &ProjectRelativePath,
        create: bool,
    ) -> Result<PathBuf, ProtocolFailure> {
        if !relative.segments.is_empty() {
            validate_config_path(relative)?;
            if !matches!(relative.segments.first(), Some(PathSegment::Utf8(name))
                if matches!(name.as_str(), "projects" | "themes" | "agent-providers"))
            {
                return Err(failure(
                    FailureCode::PermissionDenied,
                    "configuration directory is not shared",
                    false,
                ));
            }
        }
        let mut path = self.config_root.clone();
        existing_directory_not_symlink(&path)?;
        for segment in &relative.segments {
            path.push(segment.to_os_string());
            if create {
                ensure_directory_not_symlink(&path).map_err(config_directory_failure)?;
            } else if !existing_directory_not_symlink(&path)? {
                return Ok(relative.join_under(&self.config_root));
            }
        }
        Ok(path)
    }

    fn read_config(
        &self,
        relative_path: &ProjectRelativePath,
    ) -> Result<Option<WorkspaceConfig>, ProtocolFailure> {
        validate_config_path(relative_path)?;
        Self::read_config_at(&self.config_root, relative_path)
    }

    fn read_project_config(
        &self,
        project_root: HostPath,
        file: WorkspaceProjectConfigFile,
    ) -> Result<Option<WorkspaceConfig>, ProtocolFailure> {
        let (config_root, relative_path) = self.project_config_root(project_root, file)?;
        Self::read_config_at(&config_root, &relative_path)
    }

    fn write_config(
        &self,
        relative_path: ProjectRelativePath,
        expected_revision: Option<WorkspaceConfigRevision>,
        bytes: Vec<u8>,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        validate_config_path(&relative_path)?;
        Self::write_config_at(&self.config_root, relative_path, expected_revision, bytes)
    }

    fn write_project_config(
        &self,
        project_root: HostPath,
        file: WorkspaceProjectConfigFile,
        expected_revision: Option<WorkspaceConfigRevision>,
        bytes: Vec<u8>,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        self.ensure_project_config_mutable()?;
        let (config_root, relative_path) = self.project_config_root(project_root, file)?;
        Self::write_config_at(&config_root, relative_path, expected_revision, bytes)
    }

    fn delete_config(
        &self,
        relative_path: &ProjectRelativePath,
        expected_revision: WorkspaceConfigRevision,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        validate_config_path(relative_path)?;
        Self::delete_config_at(&self.config_root, relative_path, expected_revision)
    }

    fn delete_project_config(
        &self,
        project_root: HostPath,
        file: WorkspaceProjectConfigFile,
        expected_revision: WorkspaceConfigRevision,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        self.ensure_project_config_mutable()?;
        let (config_root, relative_path) = self.project_config_root(project_root, file)?;
        Self::delete_config_at(&config_root, &relative_path, expected_revision)
    }

    fn ensure_project_config_mutable(&self) -> Result<(), ProtocolFailure> {
        if matches!(
            &self.project_config,
            WorkspaceProjectConfig::ReadOnlyProject
        ) {
            return Err(failure(
                FailureCode::PermissionDenied,
                "project configuration is read-only",
                false,
            ));
        }
        Ok(())
    }

    fn project_config_root(
        &self,
        project_root: HostPath,
        file: WorkspaceProjectConfigFile,
    ) -> Result<(PathBuf, ProjectRelativePath), ProtocolFailure> {
        let project_root = project_root.to_path().map_err(|error| {
            failure(
                FailureCode::InvalidRequest,
                format!("invalid project root: {error}"),
                false,
            )
        })?;
        let project_root = fs::canonicalize(&project_root)
            .map_err(|error| filesystem_failure("resolve project root", error))?;
        if !project_root.is_dir() {
            return Err(failure(
                FailureCode::InvalidRequest,
                "project root is not a directory",
                false,
            ));
        }
        let config_root = match &self.project_config {
            WorkspaceProjectConfig::Project | WorkspaceProjectConfig::ReadOnlyProject => {
                project_root.join(".yttt")
            }
            WorkspaceProjectConfig::Overlay { root } => {
                let overlay_root = root.to_path().map_err(|error| {
                    failure(
                        FailureCode::Internal,
                        format!("invalid project configuration overlay root: {error}"),
                        false,
                    )
                })?;
                existing_directory_not_symlink(&overlay_root)?;
                overlay_root.join(encode_project_path(&project_root))
            }
        };
        let relative_path = ProjectRelativePath::from_utf8(file.file_name())
            .expect("project configuration file names are valid relative paths");
        Ok((config_root, relative_path))
    }

    fn read_config_at(
        config_root: &Path,
        relative_path: &ProjectRelativePath,
    ) -> Result<Option<WorkspaceConfig>, ProtocolFailure> {
        let Some(path) = Self::existing_config_path(config_root, relative_path)? else {
            return Ok(None);
        };
        let bytes = read_bounded_file(&path, MAX_CONFIG_BYTES).map_err(|error| {
            failure(
                FailureCode::Internal,
                format!("failed to read profile configuration: {error}"),
                true,
            )
        })?;
        Ok(Some(WorkspaceConfig {
            relative_path: relative_path.clone(),
            revision: config_revision(&bytes),
            bytes,
        }))
    }

    fn write_config_at(
        config_root: &Path,
        relative_path: ProjectRelativePath,
        expected_revision: Option<WorkspaceConfigRevision>,
        bytes: Vec<u8>,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(failure(
                FailureCode::ResourceLimit,
                "profile configuration exceeds its size limit",
                false,
            ));
        }
        let current = Self::read_config_at(config_root, &relative_path)?;
        if current.as_ref().map(|config| &config.revision) != expected_revision.as_ref() {
            return Err(failure(
                FailureCode::Conflict,
                "profile configuration revision does not match",
                false,
            ));
        }

        let path = Self::create_config_path(config_root, &relative_path)?;
        let temporary_file = random_temporary_file();
        atomic_write(&path, &bytes, temporary_file).map_err(|error| {
            failure(
                FailureCode::Internal,
                format!("failed to persist profile configuration: {error}"),
                true,
            )
        })?;
        Ok(WorkspaceResponse::ConfigWritten {
            relative_path,
            revision: config_revision(&bytes),
        })
    }

    fn delete_config_at(
        config_root: &Path,
        relative_path: &ProjectRelativePath,
        expected_revision: WorkspaceConfigRevision,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        let config = Self::read_config_at(config_root, relative_path)?;
        if config.as_ref().map(|value| &value.revision) != Some(&expected_revision) {
            return Err(failure(
                FailureCode::Conflict,
                "configuration changed before deletion",
                false,
            ));
        }
        let path = Self::existing_config_path(config_root, relative_path)?
            .ok_or_else(|| failure(FailureCode::Conflict, "configuration was removed", false))?;
        fs::remove_file(&path)
            .map_err(|error| filesystem_failure("delete configuration", error))?;
        sync_config_parent(&path)?;
        Ok(WorkspaceResponse::ConfigDeleted)
    }

    fn existing_config_path(
        config_root: &Path,
        relative_path: &ProjectRelativePath,
    ) -> Result<Option<PathBuf>, ProtocolFailure> {
        let mut current = config_root.to_path_buf();
        if !existing_directory_not_symlink(&current)? {
            return Ok(None);
        }
        let (name, parents) = relative_path.segments.split_last().ok_or_else(|| {
            failure(
                FailureCode::InvalidRequest,
                "configuration path is empty",
                false,
            )
        })?;
        for segment in parents {
            current.push(segment.to_os_string());
            if !existing_directory_not_symlink(&current)? {
                return Ok(None);
            }
        }
        current.push(name.to_os_string());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(failure(
                FailureCode::PermissionDenied,
                "configuration path must not be a symbolic link",
                false,
            )),
            Ok(metadata) if !metadata.is_file() => Err(failure(
                FailureCode::InvalidRequest,
                "configuration path is not a regular file",
                false,
            )),
            Ok(_) => Ok(Some(current)),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(failure(
                FailureCode::Internal,
                format!("failed to inspect profile configuration: {error}"),
                true,
            )),
        }
    }

    fn create_config_path(
        config_root: &Path,
        relative_path: &ProjectRelativePath,
    ) -> Result<PathBuf, ProtocolFailure> {
        let mut current = config_root.to_path_buf();
        ensure_directory_not_symlink(&current).map_err(config_directory_failure)?;
        let (name, parents) = relative_path.segments.split_last().ok_or_else(|| {
            failure(
                FailureCode::InvalidRequest,
                "configuration path is empty",
                false,
            )
        })?;
        for segment in parents {
            current.push(segment.to_os_string());
            ensure_directory_not_symlink(&current).map_err(config_directory_failure)?;
        }
        current.push(name.to_os_string());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => Err(failure(
                FailureCode::PermissionDenied,
                "configuration path must not be a symbolic link",
                false,
            )),
            Ok(metadata) if !metadata.is_file() => Err(failure(
                FailureCode::InvalidRequest,
                "configuration path is not a regular file",
                false,
            )),
            Ok(_) => Ok(current),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(current),
            Err(error) => Err(failure(
                FailureCode::Internal,
                format!("failed to inspect profile configuration: {error}"),
                true,
            )),
        }
    }
}

fn scan_agent_history(
    providers: Vec<String>,
    root: HostPath,
) -> Result<WorkspaceResponse, ProtocolFailure> {
    use yttt_agent_providers::sessions::{
        AgentSessionRoots, SessionProvider, scan_agent_sessions_for_agents_with_roots,
    };
    if providers.len() > 6 {
        return Err(failure(
            FailureCode::ResourceLimit,
            "too many Agent providers",
            false,
        ));
    }
    let providers = providers
        .into_iter()
        .map(|provider| {
            SessionProvider::from_id(&provider).ok_or_else(|| {
                failure(FailureCode::InvalidRequest, "unknown Agent provider", false)
            })
        })
        .collect::<Result<Vec<_>, _>>()?;
    let root = root
        .to_path()
        .map_err(|error| failure(FailureCode::InvalidRequest, error.to_string(), false))?;
    let roots = AgentSessionRoots::native().ok_or_else(|| {
        failure(
            FailureCode::Internal,
            "Host home directory is unavailable",
            false,
        )
    })?;
    let scan = scan_agent_sessions_for_agents_with_roots(&providers, &root, &roots, true)
        .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?;
    Ok(WorkspaceResponse::AgentSessions {
        sessions: scan
            .sessions
            .into_iter()
            .map(|session| yttt_protocol::workspace::AgentSessionSummary {
                provider: session.provider.id().to_string(),
                id: session.id,
                title: session.title,
                model: session.model,
                transcript_path: session
                    .transcript_path
                    .and_then(|path| HostPath::from_path(&path).ok()),
                updated_at_ms: session.updated_at_ms,
            })
            .collect(),
        warnings: scan.warnings,
    })
}

fn mutate_agent_history(
    provider: String,
    session_id: String,
    root: HostPath,
    operation: AgentSessionOperation,
) -> Result<WorkspaceResponse, ProtocolFailure> {
    use yttt_agent_providers::{
        session_management::{
            SessionOperation as ProviderSessionOperation,
            mutate_agent_session as mutate_provider_session,
        },
        sessions::{AgentSessionRoots, SessionProvider},
    };

    let provider = SessionProvider::from_id(&provider)
        .ok_or_else(|| failure(FailureCode::InvalidRequest, "unknown Agent provider", false))?;
    let root = root
        .to_path()
        .map_err(|error| failure(FailureCode::InvalidRequest, error.to_string(), false))?;
    let roots = AgentSessionRoots::native().ok_or_else(|| {
        failure(
            FailureCode::Internal,
            "Host home directory is unavailable",
            false,
        )
    })?;
    let operation = match operation {
        AgentSessionOperation::Rename { title } => ProviderSessionOperation::Rename { title },
        AgentSessionOperation::Delete => ProviderSessionOperation::Delete,
    };
    mutate_provider_session(provider, &session_id, &root, &roots, true, &operation)
        .map_err(|error| failure(FailureCode::Internal, error, false))?;
    Ok(WorkspaceResponse::AgentSessionMutated)
}

fn environment(
    environment_id: &str,
    config_root: &Path,
    project_config: &WorkspaceProjectConfig,
    workspace_history_initialized: bool,
) -> Result<WorkspaceEnvironment, ProtocolFailure> {
    let home = std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
        .map(PathBuf::from)
        .filter(|path| path.is_absolute())
        .ok_or_else(|| {
            failure(
                FailureCode::Internal,
                "Host home directory is unavailable",
                false,
            )
        })?;
    let home = HostPath::from_path(&home).map_err(|error| {
        failure(
            FailureCode::Internal,
            format!("Host home directory is invalid: {error}"),
            false,
        )
    })?;
    let (shell, shell_candidates) = crate::pane_launch::detect_shells();
    Ok(WorkspaceEnvironment {
        environment_id: environment_id.to_string(),
        config_root: HostPath::from_path(config_root)
            .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?,
        project_config: project_config.clone(),
        shell_candidates,
        home,
        platform: std::env::consts::OS.to_string(),
        shell,
        workspace_history_initialized,
    })
}

fn browse(path: HostPath, include_hidden: bool) -> Result<WorkspaceDirectory, ProtocolFailure> {
    let path = path.to_path().map_err(|error| {
        failure(
            FailureCode::InvalidRequest,
            format!("Host browse path is invalid: {error}"),
            false,
        )
    })?;
    let path = fs::canonicalize(path)
        .map_err(|error| filesystem_failure("resolve Host directory", error))?;
    let metadata =
        fs::metadata(&path).map_err(|error| filesystem_failure("open Host directory", error))?;
    if !metadata.is_dir() {
        return Err(failure(
            FailureCode::InvalidRequest,
            "Host browse path is not a directory",
            false,
        ));
    }

    let mut entries = Vec::new();
    for entry in
        fs::read_dir(&path).map_err(|error| filesystem_failure("read Host directory", error))?
    {
        let entry =
            entry.map_err(|error| filesystem_failure("read Host directory entry", error))?;
        let name = PathSegment::from_os_str(entry.file_name()).map_err(|error| {
            failure(
                FailureCode::Internal,
                format!("Host directory entry cannot be represented: {error}"),
                false,
            )
        })?;
        if !include_hidden && path_segment_is_hidden(&name) {
            continue;
        }
        if entries.len() == MAX_BROWSE_ENTRIES {
            return Err(failure(
                FailureCode::ResourceLimit,
                "Host directory has too many entries to browse",
                false,
            ));
        }
        let entry_path = entry.path();
        let file_type = entry
            .file_type()
            .map_err(|error| filesystem_failure("inspect Host directory entry", error))?;
        let kind = if file_type.is_dir() {
            WorkspaceDirectoryEntryKind::Directory
        } else if file_type.is_file() {
            WorkspaceDirectoryEntryKind::File
        } else if file_type.is_symlink() {
            if fs::metadata(&entry_path).is_ok_and(|target| target.is_dir()) {
                WorkspaceDirectoryEntryKind::SymlinkDirectory
            } else {
                WorkspaceDirectoryEntryKind::Symlink
            }
        } else {
            WorkspaceDirectoryEntryKind::Other
        };
        let entry_path = HostPath::from_path(&entry_path).map_err(|error| {
            failure(
                FailureCode::Internal,
                format!("Host directory entry path is invalid: {error}"),
                false,
            )
        })?;
        entries.push(WorkspaceDirectoryEntry {
            name,
            path: entry_path,
            kind,
        });
    }
    entries.sort_by(|left, right| {
        left.name
            .to_os_string()
            .to_string_lossy()
            .cmp(&right.name.to_os_string().to_string_lossy())
    });
    let path = HostPath::from_path(&path).map_err(|error| {
        failure(
            FailureCode::Internal,
            format!("Host browse path is invalid: {error}"),
            false,
        )
    })?;
    Ok(WorkspaceDirectory { path, entries })
}

fn workspace_summary(id: &WorkspaceId, record: &WorkspaceRecord) -> WorkspaceSummary {
    WorkspaceSummary {
        workspace_id: id.clone(),
        name: if record.name.is_empty() {
            id.as_str().to_string()
        } else {
            record.name.clone()
        },
        revision: record.revision(),
        saved_millis: record.saved_millis,
        restore_on_startup: record.restore_on_startup,
    }
}

fn workspace_is_empty(record: &WorkspaceRecord) -> bool {
    record.documents.is_empty()
        && record.agent_states.is_empty()
        && record.layout.workspace().opened_projects().is_empty()
}

enum RecordLoadError {
    /// The record is damaged; a backup may replace it.
    Unreadable(io::Error),
    /// A record from before schema 4 could not be migrated; it is kept aside.
    Migration(io::Error),
    /// The record is intact but cannot be loaded here.
    Fatal(io::Error),
}

#[derive(Deserialize)]
struct SchemaVersion {
    schema_version: u16,
}

/// A record of schema 1 to 3: the Client-published desktop snapshot as
/// opaque JSON, with drafts referenced from its documents.
#[derive(Deserialize)]
struct LegacyWorkspaceRecord {
    schema_version: u16,
    revision: u64,
    snapshot: serde_json::Value,
    #[serde(default)]
    drafts: Vec<DraftRef>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    saved_millis: u64,
    #[serde(default)]
    restore_on_startup: bool,
}

/// Parses one stored record; `true` when it was migrated and must be rewritten.
fn load_workspace_record(
    bytes: &[u8],
    workspace_id: &WorkspaceId,
    drafts: &crate::workspace_objects::WorkspaceObjects<DraftRef>,
    agent_states: &crate::workspace_objects::WorkspaceObjects<AgentStateRef>,
) -> Result<(WorkspaceRecord, bool), RecordLoadError> {
    let unreadable = |error: serde_json::Error| {
        RecordLoadError::Unreadable(invalid_state(&format!("invalid workspace state: {error}")))
    };
    let version: SchemaVersion = serde_json::from_slice(bytes).map_err(unreadable)?;
    match version.schema_version {
        WORKSPACE_SCHEMA_VERSION => {
            let persisted: PersistedWorkspaceRecord =
                serde_json::from_slice(bytes).map_err(unreadable)?;
            let record =
                WorkspaceRecord::from_persisted(persisted).map_err(RecordLoadError::Unreadable)?;
            Ok((record, false))
        }
        1..=3 => {
            let legacy: LegacyWorkspaceRecord =
                serde_json::from_slice(bytes).map_err(unreadable)?;
            migrate_legacy_record(legacy, workspace_id, drafts, agent_states)
                .map(|record| (record, true))
                .map_err(RecordLoadError::Migration)
        }
        _ => Err(RecordLoadError::Fatal(invalid_state(
            "unsupported workspace state schema",
        ))),
    }
}

/// Parses the layout of a schema 1 to 3 record; the Host keeps its documents'
/// drafts and its panes' Agent states, without leases.
fn migrate_legacy_record(
    mut legacy: LegacyWorkspaceRecord,
    workspace_id: &WorkspaceId,
    drafts: &crate::workspace_objects::WorkspaceObjects<DraftRef>,
    agent_states: &crate::workspace_objects::WorkspaceObjects<AgentStateRef>,
) -> io::Result<WorkspaceRecord> {
    #[derive(Deserialize)]
    struct LegacyDocument {
        project_id: ProjectId,
        relative_path: PathBuf,
        #[serde(default)]
        draft_ref: Option<DraftRef>,
    }

    if legacy.schema_version == 1 {
        migrate_inline_drafts(drafts, workspace_id, &mut legacy)?;
    }
    if legacy.schema_version <= 2 {
        migrate_inline_agent_states(agent_states, workspace_id, &mut legacy)?;
    }
    let layout = legacy_layout(&legacy.snapshot, legacy.revision)?;
    let workspace = layout.state().workspace;
    let agent_refs = legacy_agent_states(&legacy.snapshot)?
        .into_iter()
        .filter(|reference| pane_exists(&workspace, reference))
        .collect::<Vec<_>>();
    agent_states.validate_references(workspace_id, &agent_refs)?;
    let mut documents = Vec::new();
    if let Some(entries) = legacy
        .snapshot
        .get("documents")
        .and_then(serde_json::Value::as_array)
    {
        for entry in entries {
            let Ok(LegacyDocument {
                project_id,
                relative_path,
                draft_ref: Some(draft),
            }) = LegacyDocument::deserialize(entry)
            else {
                continue;
            };
            let Some(root) = layout
                .workspace()
                .opened_projects()
                .iter()
                .find(|project| project.id == project_id)
                .and_then(|project| project.location.local_path())
            else {
                continue;
            };
            let document = DocumentId {
                canonical_path: root.join(relative_path),
                project_id,
            };
            if legacy.drafts.contains(&draft) && layout.contains_document(&document) {
                documents.push(StoredDocument { document, draft });
            }
        }
    }
    let draft_refs = documents
        .iter()
        .map(|stored| stored.draft.clone())
        .collect::<Vec<_>>();
    drafts.validate_references(workspace_id, &draft_refs)?;
    Ok(WorkspaceRecord {
        layout,
        documents,
        agent_states: agent_refs,
        name: legacy.name,
        saved_millis: legacy.saved_millis,
        restore_on_startup: legacy.restore_on_startup,
        agents_changed: false,
    })
}

/// The layout of a Client-published snapshot; an empty snapshot is an empty workspace.
fn legacy_layout(snapshot: &serde_json::Value, revision: u64) -> io::Result<HostWorkspace> {
    let layout = if snapshot.as_object().is_some_and(|object| object.is_empty()) {
        HostWorkspace::restore(HostWorkspaceState {
            revision,
            workspace: Workspace::new().persisted_state(),
            editor: Default::default(),
        })
    } else {
        let legacy = LegacyWorkspaceSnapshot::deserialize(snapshot).map_err(io::Error::other)?;
        HostWorkspace::from_legacy(legacy, revision)
    };
    layout.map_err(io::Error::other)
}

fn legacy_agent_states(snapshot: &serde_json::Value) -> io::Result<Vec<AgentStateRef>> {
    match snapshot.get("agent_states") {
        None => Ok(Vec::new()),
        Some(value) => serde_json::from_value(value.clone()).map_err(io::Error::other),
    }
}

/// Puts the stored Agent states back on their panes. An unreadable state is
/// dropped from the layout; its pane keeps running without the old board.
fn load_agent_snapshots(
    objects: &crate::workspace_objects::WorkspaceObjects<AgentStateRef>,
    workspace_id: &WorkspaceId,
    layout: &mut HostWorkspace,
    references: &[AgentStateRef],
) {
    for reference in references {
        let snapshot = objects
            .read(workspace_id, reference)
            .and_then(|content| serde_json::from_slice(&content).map_err(io::Error::other));
        let address = PaneAddress {
            project_id: reference.project_id.clone(),
            tab_id: reference.tab_id.clone(),
            pane_id: reference.pane_id.clone(),
        };
        match snapshot {
            Ok(snapshot) => {
                let _ = layout.set_agent_snapshot(&address, Some(snapshot));
            }
            Err(error) => eprintln!(
                "Agent state of pane {} in workspace {} is unreadable: {error}",
                address.terminal_session_id().as_str(),
                workspace_id.as_str()
            ),
        }
    }
}

fn layout_encoding_failure(error: serde_json::Error) -> ProtocolFailure {
    failure(
        FailureCode::Internal,
        format!("failed to encode workspace layout: {error}"),
        false,
    )
}

fn pane_exists(workspace: &WorkspaceState, reference: &AgentStateRef) -> bool {
    workspace
        .opened_projects
        .iter()
        .find(|project| project.id == reference.project_id)
        .and_then(|project| {
            project
                .tab_states
                .iter()
                .find(|tab| tab.tab_id == reference.tab_id)
        })
        .is_some_and(|tab| {
            tab.pane_states
                .iter()
                .any(|pane| pane.pane_id == reference.pane_id)
        })
}

fn intent_failure(error: IntentError) -> ProtocolFailure {
    let code = match &error {
        IntentError::RevisionConflict { .. } | IntentError::RunningProcesses => {
            FailureCode::Conflict
        }
        IntentError::Workspace(
            WorkspaceError::ProjectNotFound(_)
            | WorkspaceError::TabNotFound(_)
            | WorkspaceError::PaneNotFound(_),
        )
        | IntentError::WorkItemNotFound(_)
        | IntentError::GroupNotFound(_)
        | IntentError::SplitNotFound(_) => FailureCode::NotFound,
        _ => FailureCode::InvalidRequest,
    };
    failure(code, error.to_string(), false)
}

/// Schema 1 kept draft bodies inline in the snapshot's documents.
fn migrate_inline_drafts(
    objects: &crate::workspace_objects::WorkspaceObjects<DraftRef>,
    workspace_id: &WorkspaceId,
    legacy: &mut LegacyWorkspaceRecord,
) -> io::Result<()> {
    if let Some(documents) = legacy
        .snapshot
        .get_mut("documents")
        .and_then(serde_json::Value::as_array_mut)
    {
        for document in documents {
            let Some(draft_value) = document
                .get("draft")
                .filter(|value| !value.is_null())
                .cloned()
            else {
                continue;
            };
            let draft: DraftContentRevision =
                serde_json::from_value(draft_value).map_err(io::Error::other)?;
            let key =
                serde_json::to_vec(&(document.get("project_id"), document.get("relative_path")))
                    .map_err(io::Error::other)?;
            let reference = DraftRef::for_document(&key, &draft);
            objects.put(workspace_id, &reference, draft.content.as_bytes())?;
            document["draft_ref"] = serde_json::to_value(&reference).map_err(io::Error::other)?;
            legacy.drafts.push(reference);
        }
    }
    Ok(())
}

/// Schemas 1 and 2 kept Agent snapshots inline in the snapshot's panes.
fn migrate_inline_agent_states(
    objects: &crate::workspace_objects::WorkspaceObjects<AgentStateRef>,
    workspace_id: &WorkspaceId,
    legacy: &mut LegacyWorkspaceRecord,
) -> io::Result<()> {
    let snapshot = &mut legacy.snapshot;
    let mut references = legacy_agent_states(snapshot)?;
    if let Some(projects) = snapshot
        .pointer_mut("/workspace/opened_projects")
        .and_then(serde_json::Value::as_array_mut)
    {
        for project in projects {
            let project_id: ProjectId =
                serde_json::from_value(project["id"].clone()).map_err(io::Error::other)?;
            let Some(tabs) = project
                .get_mut("tab_states")
                .and_then(serde_json::Value::as_array_mut)
            else {
                continue;
            };
            for tab in tabs {
                let tab_id = tab["tab_id"]
                    .as_str()
                    .ok_or_else(|| invalid_state("Agent state tab has no identity"))?
                    .to_string();
                let Some(panes) = tab
                    .get_mut("pane_states")
                    .and_then(serde_json::Value::as_array_mut)
                else {
                    continue;
                };
                for pane in panes {
                    let Some(agent) = pane.get("agent_snapshot").filter(|value| !value.is_null())
                    else {
                        continue;
                    };
                    let pane_id = pane["pane_id"]
                        .as_str()
                        .ok_or_else(|| invalid_state("Agent state pane has no identity"))?
                        .to_string();
                    let agent: AgentSnapshot =
                        serde_json::from_value(agent.clone()).map_err(io::Error::other)?;
                    let content = serde_json::to_vec(&agent).map_err(io::Error::other)?;
                    let reference = AgentStateRef::for_content(
                        project_id.clone(),
                        tab_id.clone(),
                        pane_id,
                        &content,
                    );
                    objects.put(workspace_id, &reference, &content)?;
                    references.push(reference);
                    pane["agent_snapshot"] = serde_json::Value::Null;
                }
            }
        }
        if snapshot["schema_version"] == 1 {
            snapshot["schema_version"] = serde_json::json!(2);
        }
    }
    if !references.is_empty() {
        snapshot["agent_states"] = serde_json::to_value(&references).map_err(io::Error::other)?;
    }
    Ok(())
}

fn serialize_workspace_record(record: &WorkspaceRecord) -> Result<Vec<u8>, ProtocolFailure> {
    let bytes = serde_json::to_vec(&record.persisted()).map_err(|error| {
        failure(
            FailureCode::Internal,
            format!("failed to encode workspace state: {error}"),
            false,
        )
    })?;
    if bytes.len() > MAX_WORKSPACE_STATE_BYTES {
        return Err(failure(
            FailureCode::ResourceLimit,
            "workspace state exceeds its size limit",
            false,
        ));
    }
    Ok(bytes)
}

fn require_revision(
    record: &WorkspaceRecord,
    expected_revision: u64,
) -> Result<(), ProtocolFailure> {
    if record.revision() == expected_revision {
        Ok(())
    } else {
        Err(failure(
            FailureCode::Conflict,
            "workspace revision does not match",
            false,
        ))
    }
}

fn config_revision(bytes: &[u8]) -> WorkspaceConfigRevision {
    WorkspaceConfigRevision {
        content_sha256: Sha256::digest(bytes).into(),
    }
}

fn sync_config_parent(path: &Path) -> Result<(), ProtocolFailure> {
    #[cfg(unix)]
    File::open(path.parent().unwrap())
        .and_then(|directory| directory.sync_all())
        .map_err(|error| filesystem_failure("persist configuration directory", error))?;
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

fn validate_config_path(relative_path: &ProjectRelativePath) -> Result<(), ProtocolFailure> {
    if relative_path.segments.is_empty()
        || relative_path.segments.iter().any(|segment| {
            let raw = segment.to_os_string();
            let mut components = Path::new(&raw).components();
            !matches!(components.next(), Some(Component::Normal(_)))
                || components.next().is_some()
                || match segment {
                    PathSegment::Utf8(value) => {
                        value.contains('\0') || value.contains('/') || value.contains('\\')
                    }
                    PathSegment::Bytes(value) => {
                        value.contains(&0) || value.contains(&b'/') || value.contains(&b'\\')
                    }
                }
        })
    {
        return Err(failure(
            FailureCode::InvalidRequest,
            "configuration path must be a non-empty relative path without traversal",
            false,
        ));
    }
    if !config_path_is_shared(relative_path) {
        return Err(failure(
            FailureCode::PermissionDenied,
            "device state and administration files are not shared configuration",
            false,
        ));
    }
    Ok(())
}

fn config_path_is_shared(path: &ProjectRelativePath) -> bool {
    let Some(PathSegment::Utf8(name)) = path.segments.first() else {
        return false;
    };
    matches!(name.as_str(), "projects" | "themes" | "agent-providers")
        || (path.segments.len() == 1
            && matches!(
                name.as_str(),
                "settings.toml"
                    | "keybindings.toml"
                    | "bars.toml"
                    | "default-layout.toml"
                    | "recent-projects.toml"
                    | "agent-state.json"
            ))
}

fn existing_directory_not_symlink(path: &Path) -> Result<bool, ProtocolFailure> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(failure(
            FailureCode::PermissionDenied,
            "configuration path must not traverse a symbolic link",
            false,
        )),
        Ok(metadata) if metadata.is_dir() => Ok(true),
        Ok(_) => Err(failure(
            FailureCode::InvalidRequest,
            "configuration path parent is not a directory",
            false,
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(failure(
            FailureCode::Internal,
            format!("failed to inspect configuration path: {error}"),
            true,
        )),
    }
}

fn ensure_directory_not_symlink(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "directory must not be a symbolic link",
        )),
        Ok(metadata) if metadata.is_dir() => Ok(()),
        Ok(_) => Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "path exists but is not a directory",
        )),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if let Some(parent) = path
                .parent()
                .filter(|parent| !parent.as_os_str().is_empty())
            {
                ensure_directory_not_symlink(parent)?;
            }
            fs::create_dir(path)?;
            ensure_directory_not_symlink(path)
        }
        Err(error) => Err(error),
    }
}

fn config_directory_failure(error: io::Error) -> ProtocolFailure {
    let code = if error.kind() == io::ErrorKind::PermissionDenied {
        FailureCode::PermissionDenied
    } else {
        FailureCode::Internal
    };
    failure(
        code,
        format!("failed to prepare profile configuration directory: {error}"),
        code == FailureCode::Internal,
    )
}

pub(crate) fn atomic_write(path: &Path, bytes: &[u8], sequence: u64) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write path does not have a parent directory",
        )
    })?;
    let file_name = path.file_name().and_then(OsStr::to_str).ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "atomic write path does not have a UTF-8 file name",
        )
    })?;
    let temporary = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));
    let write_result = (|| {
        let mut options = OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut file = options.open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        drop(file);
        fs::rename(&temporary, path)?;
        #[cfg(unix)]
        File::open(parent)?.sync_all()?;
        Ok(())
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result
}

pub(crate) fn read_bounded_file(path: &Path, maximum_bytes: usize) -> io::Result<Vec<u8>> {
    let file = File::open(path)?;
    let mut bytes = Vec::new();
    file.take((maximum_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() > maximum_bytes {
        return Err(io::Error::new(
            io::ErrorKind::FileTooLarge,
            "file exceeds its configured size limit",
        ));
    }
    Ok(bytes)
}

fn path_segment_is_hidden(segment: &PathSegment) -> bool {
    match segment {
        PathSegment::Utf8(text) => text.starts_with('.'),
        PathSegment::Bytes(bytes) => bytes.first() == Some(&b'.'),
    }
}

fn random_temporary_file() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEMPORARY_FILE: AtomicU64 = AtomicU64::new(0);
    NEXT_TEMPORARY_FILE.fetch_add(1, Ordering::Relaxed)
}

fn invalid_state(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.to_string())
}

fn filesystem_failure(action: &str, error: io::Error) -> ProtocolFailure {
    let code = if error.kind() == io::ErrorKind::NotFound {
        FailureCode::NotFound
    } else if error.kind() == io::ErrorKind::PermissionDenied {
        FailureCode::PermissionDenied
    } else {
        FailureCode::Internal
    };
    failure(
        code,
        format!("failed to {action}: {error}"),
        code == FailureCode::Internal,
    )
}

fn failure(code: FailureCode, message: impl Into<String>, retryable: bool) -> ProtocolFailure {
    ProtocolFailure::new(code, message, retryable)
}

mod documents;

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use tempfile::tempdir;
    use yttt_core::model::{
        host_workspace::{HostWorkspaceState, ProjectPanel, WorkspaceIntent},
        ids::{ClientInstanceId, ProjectId},
        layout::ProjectLayout,
        project::{ProjectDescriptor, ProjectLocation},
    };
    use yttt_protocol::{
        FailureCode, HostPath, ProjectRelativePath,
        workspace::{
            WorkspaceConfigRevision, WorkspaceDirectoryEntryKind, WorkspaceId,
            WorkspaceProjectConfig, WorkspaceProjectConfigFile, WorkspaceRequest,
            WorkspaceResponse, encode_project_path,
        },
    };

    use super::{WorkspaceService, browse};

    fn client(name: &str) -> ClientInstanceId {
        ClientInstanceId::new(name)
    }

    fn workspace() -> WorkspaceId {
        WorkspaceId::new("default").unwrap()
    }

    fn service(root: &std::path::Path) -> WorkspaceService {
        WorkspaceService::new(root, root.join("config"), WorkspaceProjectConfig::Project).unwrap()
    }

    fn open_project_intent(root: &std::path::Path) -> WorkspaceIntent {
        let project_root = root.join("project");
        fs::create_dir_all(&project_root).unwrap();
        let layout: ProjectLayout = toml::from_str(
            r#"
            [project]
            name = "Saved"

            [[tabs]]
            id = "first"
            title = "First"
            layout = { type = "pane", id = "shell", title = "Shell", command = "$SHELL" }
            "#,
        )
        .unwrap();
        WorkspaceIntent::OpenProject {
            descriptor: ProjectDescriptor::new(
                ProjectId::new("project"),
                ProjectLocation::local(&project_root),
            ),
            layout,
            panel: ProjectPanel {
                visible: true,
                width: 240.0,
            },
        }
    }

    /// Opens one project in the default workspace, moving it to revision 1.
    fn open_project(host: &WorkspaceService, root: &std::path::Path) {
        host.apply(workspace(), open_project_intent(root), None)
            .unwrap();
    }

    fn opened_layout(host: &WorkspaceService, reader: &ClientInstanceId) -> HostWorkspaceState {
        let WorkspaceResponse::Opened { layout, .. } = host
            .handle(
                reader,
                WorkspaceRequest::Open {
                    workspace_id: workspace(),
                },
            )
            .unwrap()
        else {
            panic!("open response")
        };
        layout
    }

    #[test]
    fn clearing_an_agent_state_tells_the_clients_that_opened_the_workspace() {
        let root = tempdir().unwrap();
        let host = std::sync::Arc::new(service(root.path()));
        open_project(&host, root.path());
        let address = yttt_core::model::host_workspace::PaneAddress {
            project_id: ProjectId::new("project"),
            tab_id: "first".into(),
            pane_id: "shell".into(),
        };
        let saved = yttt_agent_core::AgentReducer::new(
            yttt_agent_core::AgentInstanceId::random(),
            yttt_agent_core::ProviderId::from_static("claude"),
            1,
        )
        .snapshot()
        .clone();
        host.update_agent_snapshot(&workspace(), &address, |_| Some(saved));

        let cleared = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let reader = client("reader");
        let _subscription = host.subscribe(reader.clone(), {
            let cleared = cleared.clone();
            Box::new(move |event| {
                if let super::WorkspaceEvent::AgentStateCleared(update) = event {
                    cleared
                        .lock()
                        .unwrap()
                        .push(update.terminal_session_id.clone());
                }
                true
            })
        });
        opened_layout(&host, &reader);

        host.update_agent_snapshot(&workspace(), &address, |_| None);
        host.update_agent_snapshot(&workspace(), &address, |_| None);

        assert_eq!(
            *cleared.lock().unwrap(),
            vec![address.terminal_session_id()],
            "only the change that dropped the state is announced"
        );
    }

    #[test]
    fn open_does_not_publish_an_agent_state_the_launch_cleared() {
        let root = tempdir().unwrap();
        let host = service(root.path());
        open_project(&host, root.path());
        let address = yttt_core::model::host_workspace::PaneAddress {
            project_id: ProjectId::new("project"),
            tab_id: "first".into(),
            pane_id: "shell".into(),
        };
        let saved = yttt_agent_core::AgentReducer::new(
            yttt_agent_core::AgentInstanceId::random(),
            yttt_agent_core::ProviderId::from_static("claude"),
            1,
        )
        .snapshot()
        .clone();
        host.update_agent_snapshot(&workspace(), &address, |_| Some(saved));
        host.flush_agent_states();

        // PaneLauncher reports clears_agent_snapshot for a shell pane with no resumable session.
        host.update_agent_snapshot(&workspace(), &address, |_| None);

        let WorkspaceResponse::Opened { agent_states, .. } = host
            .handle(
                &client("reader"),
                WorkspaceRequest::Open {
                    workspace_id: workspace(),
                },
            )
            .unwrap()
        else {
            panic!("open response")
        };
        assert!(
            agent_states.is_empty(),
            "Client restored a snapshot the Host already cleared: {agent_states:?}"
        );
    }

    #[test]
    fn closing_retains_history_but_removes_startup_membership() {
        let root = tempdir().unwrap();
        let host = service(root.path());
        let owner = client("owner");
        open_project(&host, root.path());
        assert_eq!(
            host.handle(
                &owner,
                WorkspaceRequest::Forget {
                    workspace_id: workspace(),
                    expected_revision: 1,
                }
            )
            .unwrap_err()
            .code,
            FailureCode::Conflict
        );
        host.handle(
            &owner,
            WorkspaceRequest::SetRestoreOnStartup {
                workspace_id: workspace(),
                expected_revision: 1,
                restore: false,
            },
        )
        .unwrap();
        drop(host);
        let host = service(root.path());
        let WorkspaceResponse::Workspaces(index) =
            host.handle(&owner, WorkspaceRequest::List).unwrap()
        else {
            panic!("index")
        };
        assert_eq!(index.len(), 1);
        assert!(!index[0].restore_on_startup);
        let layout = opened_layout(&host, &owner);
        assert_eq!(layout.revision, 1);
        assert_eq!(layout.workspace.opened_projects.len(), 1);
        assert_eq!(
            host.handle(
                &owner,
                WorkspaceRequest::Forget {
                    workspace_id: workspace(),
                    expected_revision: 0,
                }
            )
            .unwrap_err()
            .code,
            FailureCode::Conflict
        );
        host.handle(
            &owner,
            WorkspaceRequest::Forget {
                workspace_id: workspace(),
                expected_revision: 1,
            },
        )
        .unwrap();
        drop(host);
        let host = service(root.path());
        assert_eq!(
            host.handle(&owner, WorkspaceRequest::List).unwrap(),
            WorkspaceResponse::Workspaces(vec![])
        );
        assert_eq!(
            host.handle(
                &owner,
                WorkspaceRequest::OpenExisting {
                    workspace_id: workspace(),
                }
            )
            .unwrap_err()
            .code,
            FailureCode::NotFound
        );
    }

    #[test]
    fn closing_empty_windows_does_not_exhaust_workspace_capacity() {
        let root = tempdir().unwrap();
        let host = service(root.path());
        let owner = client("owner");
        let WorkspaceResponse::Environment(environment) =
            host.handle(&owner, WorkspaceRequest::Environment).unwrap()
        else {
            panic!("environment")
        };
        assert!(!environment.workspace_history_initialized);
        for index in 0..=super::MAX_WORKSPACES {
            let id = WorkspaceId::new(format!("empty-{index}")).unwrap();
            host.handle(
                &owner,
                WorkspaceRequest::Register {
                    workspace_id: id.clone(),
                    name: format!("Window {index}"),
                },
            )
            .unwrap();
            assert_eq!(
                host.handle(
                    &owner,
                    WorkspaceRequest::SetRestoreOnStartup {
                        workspace_id: id.clone(),
                        expected_revision: 0,
                        restore: false,
                    }
                )
                .unwrap(),
                WorkspaceResponse::Forgotten { workspace_id: id }
            );
        }
        assert_eq!(
            host.handle(&owner, WorkspaceRequest::List).unwrap(),
            WorkspaceResponse::Workspaces(vec![])
        );
        drop(host);
        let host = service(root.path());
        let WorkspaceResponse::Environment(environment) =
            host.handle(&owner, WorkspaceRequest::Environment).unwrap()
        else {
            panic!("environment")
        };
        assert!(
            environment.workspace_history_initialized,
            "closing all windows must not reimport legacy state"
        );
    }

    #[test]
    fn environment_reports_explicit_project_configuration_routing() {
        let root = tempdir().unwrap();
        let project_config = WorkspaceProjectConfig::Overlay {
            root: HostPath::from_path(&root.path().join("project-config-overlay")).unwrap(),
        };
        let service = WorkspaceService::new(
            root.path(),
            root.path().join("config"),
            project_config.clone(),
        )
        .unwrap();

        let WorkspaceResponse::Environment(environment) = service
            .handle(&client("reader"), WorkspaceRequest::Environment)
            .unwrap()
        else {
            panic!("workspace environment")
        };

        assert_eq!(environment.project_config, project_config);
    }

    #[test]
    fn observers_read_normal_project_configuration_without_control_or_creation() {
        let root = tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir_all(project.join(".yttt")).unwrap();
        fs::write(project.join(".yttt/settings.toml"), b"settings").unwrap();
        fs::write(project.join(".yttt/layout.toml"), b"layout").unwrap();
        let service = service(root.path());
        let reader = client("reader");

        for (file, bytes) in [
            (WorkspaceProjectConfigFile::Settings, b"settings".as_slice()),
            (WorkspaceProjectConfigFile::Layout, b"layout".as_slice()),
        ] {
            let WorkspaceResponse::Config(Some(config)) = service
                .handle(
                    &reader,
                    WorkspaceRequest::ReadProjectConfig {
                        project_root: HostPath::from_path(&project).unwrap(),
                        file,
                    },
                )
                .unwrap()
            else {
                panic!("project configuration");
            };
            assert_eq!(config.bytes, bytes);
        }

        let absent_project = root.path().join("absent-project");
        fs::create_dir(&absent_project).unwrap();
        assert!(matches!(
            service
                .handle(
                    &reader,
                    WorkspaceRequest::ReadProjectConfig {
                        project_root: HostPath::from_path(&absent_project).unwrap(),
                        file: WorkspaceProjectConfigFile::Settings,
                    },
                )
                .unwrap(),
            WorkspaceResponse::Config(None)
        ));
        assert!(!absent_project.join(".yttt").exists());
    }

    #[test]
    fn observers_read_project_configuration_from_the_overlay_root() {
        let root = tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let overlay = root.path().join("overlay");
        let overlay_file = overlay
            .join(encode_project_path(&project.canonicalize().unwrap()))
            .join("layout.toml");
        fs::create_dir_all(overlay_file.parent().unwrap()).unwrap();
        fs::write(&overlay_file, b"overlay-layout").unwrap();
        let service = WorkspaceService::new(
            root.path(),
            root.path().join("config"),
            WorkspaceProjectConfig::Overlay {
                root: HostPath::from_path(&overlay).unwrap(),
            },
        )
        .unwrap();

        let WorkspaceResponse::Config(Some(config)) = service
            .handle(
                &client("reader"),
                WorkspaceRequest::ReadProjectConfig {
                    project_root: HostPath::from_path(&project).unwrap(),
                    file: WorkspaceProjectConfigFile::Layout,
                },
            )
            .unwrap()
        else {
            panic!("project configuration");
        };

        assert_eq!(config.bytes, b"overlay-layout");
    }

    #[test]
    fn read_only_project_configuration_rejects_controller_mutations() {
        let root = tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let service = WorkspaceService::new(
            root.path(),
            root.path().join("config"),
            WorkspaceProjectConfig::ReadOnlyProject,
        )
        .unwrap();
        let owner = client("owner");
        let project_root = HostPath::from_path(&project).unwrap();

        assert_eq!(
            service
                .handle(
                    &owner,
                    WorkspaceRequest::WriteProjectConfig {
                        project_root: project_root.clone(),
                        file: WorkspaceProjectConfigFile::Settings,
                        expected_revision: None,
                        bytes: b"settings".to_vec(),
                    },
                )
                .unwrap_err()
                .code,
            FailureCode::PermissionDenied
        );
        assert_eq!(
            service
                .handle(
                    &owner,
                    WorkspaceRequest::DeleteProjectConfig {
                        project_root,
                        file: WorkspaceProjectConfigFile::Settings,
                        expected_revision: WorkspaceConfigRevision {
                            content_sha256: [0; 32],
                        },
                    },
                )
                .unwrap_err()
                .code,
            FailureCode::PermissionDenied
        );
        assert!(!project.join(".yttt").exists());
    }

    #[test]
    fn project_configuration_write_uses_compare_and_swap() {
        let root = tempdir().unwrap();
        let project = root.path().join("project");
        fs::create_dir(&project).unwrap();
        let service = service(root.path());
        let owner = client("owner");
        let project_root = HostPath::from_path(&project).unwrap();

        let WorkspaceResponse::ConfigWritten { revision, .. } = service
            .handle(
                &owner,
                WorkspaceRequest::WriteProjectConfig {
                    project_root: project_root.clone(),
                    file: WorkspaceProjectConfigFile::Settings,
                    expected_revision: None,
                    bytes: b"original".to_vec(),
                },
            )
            .unwrap()
        else {
            panic!("project configuration write");
        };
        assert_eq!(
            service
                .handle(
                    &owner,
                    WorkspaceRequest::WriteProjectConfig {
                        project_root: project_root.clone(),
                        file: WorkspaceProjectConfigFile::Settings,
                        expected_revision: None,
                        bytes: b"stale".to_vec(),
                    },
                )
                .unwrap_err()
                .code,
            FailureCode::Conflict
        );
        let WorkspaceResponse::ConfigWritten { .. } = service
            .handle(
                &owner,
                WorkspaceRequest::WriteProjectConfig {
                    project_root,
                    file: WorkspaceProjectConfigFile::Settings,
                    expected_revision: Some(revision),
                    bytes: b"updated".to_vec(),
                },
            )
            .unwrap()
        else {
            panic!("project configuration compare-and-swap");
        };
        assert_eq!(
            fs::read(project.join(".yttt/settings.toml")).unwrap(),
            b"updated"
        );
    }

    #[cfg(unix)]
    #[test]
    fn browse_classifies_directory_symlinks_without_following_other_links() {
        let root = tempdir().unwrap();
        let directory = root.path().join("directory");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("nested"), b"nested").unwrap();
        let file = root.path().join("file");
        fs::write(&file, b"file").unwrap();
        let directory_link = root.path().join("directory-link");
        let file_link = root.path().join("file-link");
        let broken_link = root.path().join("broken-link");
        symlink(&directory, &directory_link).unwrap();
        symlink(&file, &file_link).unwrap();
        symlink(root.path().join("missing"), &broken_link).unwrap();
        symlink("cycle-b", root.path().join("cycle-a")).unwrap();
        symlink("cycle-a", root.path().join("cycle-b")).unwrap();

        let snapshot = browse(HostPath::from_path(root.path()).unwrap(), true).unwrap();
        let entry = |name| {
            snapshot
                .entries
                .iter()
                .find(|entry| entry.name.to_os_string() == name)
                .unwrap()
        };
        let directory_entry = entry("directory-link");
        assert_eq!(
            directory_entry.kind,
            WorkspaceDirectoryEntryKind::SymlinkDirectory
        );
        assert_eq!(
            directory_entry.path.to_path().unwrap(),
            root.path().canonicalize().unwrap().join("directory-link")
        );
        assert_eq!(
            entry("file-link").kind,
            WorkspaceDirectoryEntryKind::Symlink
        );
        assert_eq!(
            entry("broken-link").kind,
            WorkspaceDirectoryEntryKind::Symlink
        );
        assert_eq!(entry("cycle-a").kind, WorkspaceDirectoryEntryKind::Symlink);
        assert_eq!(entry("cycle-b").kind, WorkspaceDirectoryEntryKind::Symlink);

        let linked_directory = browse(directory_entry.path.clone(), true).unwrap();
        assert_eq!(linked_directory.entries.len(), 1);
        assert_eq!(
            linked_directory.entries[0].name.to_os_string(),
            std::ffi::OsString::from("nested")
        );
    }

    #[cfg(unix)]
    #[test]
    fn profile_config_rejects_symbolic_link_escape() {
        let root = tempdir().unwrap();
        let outside = root.path().join("outside-settings");
        fs::write(&outside, b"outside").unwrap();
        let service = service(root.path());
        symlink(&outside, root.path().join("config").join("settings")).unwrap();

        let failure = service
            .handle(
                &client("reader"),
                WorkspaceRequest::ReadConfig {
                    relative_path: ProjectRelativePath::from_utf8("settings").unwrap(),
                },
            )
            .unwrap_err();
        assert_eq!(failure.code, FailureCode::PermissionDenied);
    }

    #[cfg(unix)]
    #[test]
    fn project_configuration_rejects_symbolic_link_escape() {
        let root = tempdir().unwrap();
        let project = root.path().join("project");
        let outside = root.path().join("private-settings");
        fs::create_dir_all(project.join(".yttt")).unwrap();
        fs::write(&outside, b"private").unwrap();
        symlink(&outside, project.join(".yttt/settings.toml")).unwrap();
        let service = service(root.path());
        let project_root = HostPath::from_path(&project).unwrap();

        assert_eq!(
            service
                .handle(
                    &client("reader"),
                    WorkspaceRequest::ReadProjectConfig {
                        project_root: project_root.clone(),
                        file: WorkspaceProjectConfigFile::Settings,
                    },
                )
                .unwrap_err()
                .code,
            FailureCode::PermissionDenied
        );

        let owner = client("owner");
        assert_eq!(
            service
                .handle(
                    &owner,
                    WorkspaceRequest::WriteProjectConfig {
                        project_root,
                        file: WorkspaceProjectConfigFile::Settings,
                        expected_revision: None,
                        bytes: b"attempted-write".to_vec(),
                    },
                )
                .unwrap_err()
                .code,
            FailureCode::PermissionDenied
        );
        assert_eq!(fs::read(outside).unwrap(), b"private");
    }

    #[test]
    fn persisted_layout_reopens_after_a_restart() {
        let root = tempdir().unwrap();
        {
            let service = service(root.path());
            open_project(&service, root.path());
        }

        let reloaded = service(root.path());
        let layout = opened_layout(&reloaded, &client("reader"));
        assert_eq!(layout.revision, 1);
        assert_eq!(layout.workspace.opened_projects.len(), 1);
    }

    #[test]
    fn failed_atomic_replace_keeps_the_old_revision_and_can_be_retried() {
        let root = tempdir().unwrap();
        let service = service(root.path());
        let state_path = root.path().join("workspaces").join("default.json");
        fs::create_dir_all(&state_path).unwrap();

        let Err(failed) = service.apply(workspace(), open_project_intent(root.path()), Some(0))
        else {
            panic!("the record cannot be replaced by a directory")
        };
        assert_eq!(failed.code, FailureCode::Internal);
        fs::remove_dir(&state_path).unwrap();

        let applied = service
            .apply(workspace(), open_project_intent(root.path()), Some(0))
            .unwrap();
        assert_eq!(applied.revision, 1);
    }

    #[test]
    fn an_applied_layout_is_restored_after_a_host_restart() {
        let root = tempdir().unwrap();
        {
            let host = service(root.path());
            open_project(&host, root.path());
            host.apply(
                workspace(),
                WorkspaceIntent::RenameTab {
                    project_id: ProjectId::new("project"),
                    tab_id: "first".to_string(),
                    title: "Renamed".to_string(),
                },
                None,
            )
            .unwrap();
        }

        let layout = opened_layout(&service(root.path()), &client("observer"));
        assert_eq!(layout.revision, 2);
        assert_eq!(
            layout.workspace.opened_projects[0].layout.tabs[0].title,
            "Renamed"
        );
    }
}

#[cfg(test)]
mod agent_state_tests;

#[cfg(test)]
mod publication_tests {
    use super::*;
    use yttt_protocol::workspace::WorkspaceProjectConfig;

    pub(super) fn setup(root: &Path) -> (WorkspaceService, ClientInstanceId) {
        let service =
            WorkspaceService::new(root, root.join("config"), WorkspaceProjectConfig::Project)
                .unwrap();
        let client = ClientInstanceId::new("publisher");
        (service, client)
    }
    pub(super) fn register(
        service: &WorkspaceService,
        client: &ClientInstanceId,
        name: &str,
    ) -> WorkspaceId {
        let id = WorkspaceId::new(name).unwrap();
        service
            .handle(
                client,
                WorkspaceRequest::Register {
                    workspace_id: id.clone(),
                    name: name.to_string(),
                },
            )
            .unwrap();
        id
    }

    #[test]
    fn config_directory_moves_preserve_files_without_replacing_existing_trees() {
        let root = tempfile::tempdir().unwrap();
        let (service, client) = setup(root.path());
        let path = |value| ProjectRelativePath::from_utf8(value).unwrap();
        for (name, bytes) in [
            ("themes/staged/icon.svg", b"original".to_vec()),
            ("themes/occupied/icon.svg", b"keep".to_vec()),
        ] {
            service
                .handle(
                    &client,
                    WorkspaceRequest::WriteConfig {
                        relative_path: path(name),
                        expected_revision: None,
                        bytes,
                    },
                )
                .unwrap();
        }
        assert!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::RenameConfigDirectory {
                        relative_from: path("themes/staged"),
                        relative_to: path("themes/occupied")
                    }
                )
                .is_err()
        );
        assert_eq!(
            fs::read(root.path().join("config/themes/occupied/icon.svg")).unwrap(),
            b"keep"
        );
        service
            .handle(
                &client,
                WorkspaceRequest::RenameConfigDirectory {
                    relative_from: path("themes/staged"),
                    relative_to: path("themes/installed"),
                },
            )
            .unwrap();
        assert_eq!(
            fs::read(root.path().join("config/themes/installed/icon.svg")).unwrap(),
            b"original"
        );
        assert!(!root.path().join("config/themes/staged").exists());
        assert!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::RemoveConfigDirectory {
                        relative_directory: path("themes/installed")
                    }
                )
                .is_err()
        );
        assert!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::RemoveConfigDirectory {
                        relative_directory: ProjectRelativePath::root()
                    }
                )
                .is_err()
        );
        assert!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::RenameConfigDirectory {
                        relative_from: path("themes/installed"),
                        relative_to: path("state/moved")
                    }
                )
                .is_err()
        );
        assert!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::RenameConfigDirectory {
                        relative_from: path("themes/installed"),
                        relative_to: path("themes/missing/nested")
                    }
                )
                .is_err()
        );
        assert!(
            root.path()
                .join("config/themes/installed/icon.svg")
                .is_file()
        );
    }

    #[cfg(unix)]
    #[test]
    fn config_cas_and_symlink_checks_cannot_expose_device_state() {
        let root = tempfile::tempdir().unwrap();
        let (service, client) = setup(root.path());
        let path = |value| ProjectRelativePath::from_utf8(value).unwrap();
        service
            .handle(
                &client,
                WorkspaceRequest::WriteConfig {
                    relative_path: path("settings.toml"),
                    expected_revision: None,
                    bytes: b"original".to_vec(),
                },
            )
            .unwrap();
        assert_eq!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::WriteConfig {
                        relative_path: path("settings.toml"),
                        expected_revision: None,
                        bytes: b"stale".to_vec()
                    }
                )
                .unwrap_err()
                .code,
            FailureCode::Conflict
        );
        assert_eq!(
            fs::read(root.path().join("config/settings.toml")).unwrap(),
            b"original"
        );
        fs::create_dir_all(root.path().join("config/themes")).unwrap();
        fs::write(root.path().join("device-secret"), "private").unwrap();
        std::os::unix::fs::symlink(
            root.path().join("device-secret"),
            root.path().join("config/themes/escape"),
        )
        .unwrap();
        assert_eq!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::ReadConfig {
                        relative_path: path("themes/escape")
                    }
                )
                .unwrap_err()
                .code,
            FailureCode::PermissionDenied
        );
        assert_eq!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::ReadConfig {
                        relative_path: path("remote-access/settings.json")
                    }
                )
                .unwrap_err()
                .code,
            FailureCode::PermissionDenied
        );
        for request in [
            WorkspaceRequest::ReadConfig {
                relative_path: path("terminal-placements.json"),
            },
            WorkspaceRequest::WriteConfig {
                relative_path: path("terminal-placements.json"),
                expected_revision: None,
                bytes: b"obsolete".to_vec(),
            },
        ] {
            assert_eq!(
                service.handle(&client, request).unwrap_err().code,
                FailureCode::PermissionDenied
            );
        }
        fs::rename(
            root.path().join("config"),
            root.path().join("original-config"),
        )
        .unwrap();
        std::os::unix::fs::symlink(
            root.path().join("original-config"),
            root.path().join("config"),
        )
        .unwrap();
        assert_eq!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::ListConfig {
                        relative_directory: ProjectRelativePath::root()
                    }
                )
                .unwrap_err()
                .code,
            FailureCode::PermissionDenied
        );
    }
}
