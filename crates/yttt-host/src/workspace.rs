use std::{
    collections::{HashMap, VecDeque},
    ffi::OsStr,
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Component, Path, PathBuf},
};

use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use yttt_core::model::ids::ClientInstanceId;
use yttt_protocol::{
    FailureCode, HostPath, PathSegment, ProjectRelativePath, ProtocolFailure,
    workspace::{
        DraftRef, MAX_WORKSPACE_MANIFEST_BYTES, WorkspaceConfig, WorkspaceConfigRevision,
        WorkspaceDirectory, WorkspaceDirectoryEntry, WorkspaceDirectoryEntryKind, WorkspaceDraft,
        WorkspaceEnvironment, WorkspaceId, WorkspaceOperationId, WorkspaceRequest,
        WorkspaceResponse, WorkspaceSnapshot, WorkspaceSummary,
    },
};

const WORKSPACE_SCHEMA_VERSION: u16 = 2;
const MAX_WORKSPACES: usize = 128;
const MAX_COMPLETED_OPERATIONS: usize = 128;
const MAX_WORKSPACE_STATE_BYTES: usize = 2 * 1024 * 1024;
const MAX_CONFIG_BYTES: usize = 1024 * 1024;
const MAX_BROWSE_ENTRIES: usize = 4096;

/// Host-owned persistent remote workspace state for one profile.
///
/// All mutations run under one mutex. A successful mutation has first been serialized, fsynced,
/// atomically replaced, and directory-fsynced; only then is its in-memory state made visible.
pub struct WorkspaceService {
    state_root: PathBuf,
    config_root: PathBuf,
    workspace_root: PathBuf,
    environment_id: String,
    drafts: crate::drafts::DraftObjects,
    state: Mutex<ServiceState>,
    pub(crate) control: crate::control::ProfileControl,
}

struct ServiceState {
    workspaces: HashMap<WorkspaceId, WorkspaceRecord>,
    next_temporary_file: u64,
}

#[derive(Clone)]
struct WorkspaceRecord {
    revision: u64,
    snapshot: WorkspaceSnapshot,
    drafts: Vec<DraftRef>,
    name: String,
    saved_millis: u64,
    completed_operations: VecDeque<CompletedOperation>,
}

#[derive(Clone, Serialize, Deserialize)]
struct PersistedWorkspaceRecord {
    schema_version: u16,
    revision: u64,
    snapshot: WorkspaceSnapshot,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    draft: Option<WorkspaceDraft>,
    #[serde(default)]
    drafts: Vec<DraftRef>,
    #[serde(default)]
    name: String,
    #[serde(default)]
    saved_millis: u64,
    #[serde(default)]
    completed_operations: VecDeque<CompletedOperation>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CompletedOperation {
    operation_id: WorkspaceOperationId,
    request_digest: [u8; 32],
    resulting_revision: u64,
}

impl WorkspaceRecord {
    fn empty() -> Self {
        Self {
            revision: 0,
            snapshot: empty_snapshot(),
            drafts: Vec::new(),
            name: String::new(),
            saved_millis: 0,
            completed_operations: VecDeque::new(),
        }
    }

    fn persisted(&self) -> PersistedWorkspaceRecord {
        PersistedWorkspaceRecord {
            schema_version: WORKSPACE_SCHEMA_VERSION,
            revision: self.revision,
            snapshot: self.snapshot.clone(),
            draft: None,
            drafts: self.drafts.clone(),
            name: self.name.clone(),
            saved_millis: self.saved_millis,
            completed_operations: self.completed_operations.clone(),
        }
    }

    fn from_persisted(persisted: PersistedWorkspaceRecord) -> Result<Self, io::Error> {
        if persisted.schema_version != WORKSPACE_SCHEMA_VERSION {
            return Err(invalid_state("unsupported workspace state schema"));
        }
        if persisted.completed_operations.len() > MAX_COMPLETED_OPERATIONS {
            return Err(invalid_state(
                "workspace operation journal exceeds its limit",
            ));
        }
        validate_workspace_payload(&persisted.snapshot, &persisted.drafts)
            .map_err(|failure| invalid_state(&failure.message))?;
        Ok(Self {
            revision: persisted.revision,
            snapshot: persisted.snapshot,
            drafts: persisted.drafts,
            name: persisted.name,
            saved_millis: persisted.saved_millis,
            completed_operations: persisted.completed_operations,
        })
    }
}

impl WorkspaceService {
    /// Creates a workspace service rooted in the supplied profile-persistent state directory.
    ///
    /// Workspace files live in state_root; config_root is the existing environment config,
    /// supplied explicitly by the desktop or standalone Server bootstrap.
    pub fn new(state_root: impl AsRef<Path>, config_root: impl AsRef<Path>) -> io::Result<Self> {
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

        let drafts = crate::drafts::DraftObjects::new(&state_root)?;
        let mut workspaces = HashMap::new();
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
            if name.starts_with('.') && name.ends_with(".tmp") {
                continue;
            }
            let Some(identifier) = name.strip_suffix(".json") else {
                return Err(invalid_state(
                    "workspace state file does not use the .json suffix",
                ));
            };
            let workspace_id = WorkspaceId::new(identifier)
                .map_err(|_| invalid_state("workspace state file has an invalid identifier"))?;
            let metadata = entry.metadata()?;
            if metadata.len() > MAX_WORKSPACE_STATE_BYTES as u64 {
                return Err(invalid_state("workspace state file exceeds its size limit"));
            }
            let bytes = read_bounded_file(&path, MAX_WORKSPACE_STATE_BYTES)?;
            let mut persisted: PersistedWorkspaceRecord = serde_json::from_slice(&bytes)
                .map_err(|error| invalid_state(&format!("invalid workspace state: {error}")))?;
            if persisted.schema_version == 1 {
                migrate_inline_drafts(&drafts, &workspace_id, &mut persisted)?;
                // Bodies are durable first. If publication fails, the original manifest is intact.
                let migrated = serde_json::to_vec(&persisted).map_err(io::Error::other)?;
                atomic_write(&path, &migrated, random_temporary_file())?;
            }
            let record = WorkspaceRecord::from_persisted(persisted)?;
            drafts.recover(&workspace_id, &record.drafts)?;
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
            workspace_root,
            environment_id,
            drafts,
            control: crate::control::ProfileControl::new(),
            state: Mutex::new(ServiceState {
                workspaces,
                next_temporary_file: 0,
            }),
        })
    }

    pub(crate) fn referenced_projects(&self) -> Vec<(yttt_core::model::ids::ProjectId, HostPath)> {
        let state = self.state.lock();
        let mut projects = HashMap::new();
        for record in state.workspaces.values() {
            let Some(value) = record.snapshot.as_value().get("workspace") else {
                continue;
            };
            let Ok(workspace) = serde_json::from_value::<yttt_core::model::workspace::WorkspaceState>(
                value.clone(),
            ) else {
                continue;
            };
            for project in workspace.opened_projects {
                if let Some(path) = project.location.local_path()
                    && let Ok(path) = HostPath::from_path(path)
                {
                    projects.insert(project.id, path);
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

    pub(crate) fn sharing_ready(&self) -> bool {
        let state = self.state.lock();
        !state.workspaces.is_empty()
            && state
                .workspaces
                .values()
                .all(|workspace| workspace.revision > 0)
    }

    pub fn authorize_mutation(&self, client_id: &ClientInstanceId) -> Result<(), ProtocolFailure> {
        self.control.authorize_owner(client_id)
    }

    pub(crate) fn verify_revisions(
        &self,
        revisions: &[yttt_protocol::session::WorkspaceRevision],
    ) -> bool {
        let state = self.state.lock();
        let ids = revisions
            .iter()
            .map(|revision| &revision.workspace_id)
            .collect::<std::collections::HashSet<_>>();
        ids.len() == revisions.len()
            && revisions.len() == state.workspaces.len()
            && revisions.iter().all(|revision| {
                state
                    .workspaces
                    .get(&revision.workspace_id)
                    .is_some_and(|record| record.revision == revision.revision)
            })
    }

    /// Handles one request on behalf of a connected client.
    pub fn handle(
        &self,
        client_id: &ClientInstanceId,
        request: WorkspaceRequest,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        if request.is_mutation() {
            self.control.authorize_owner(client_id)?;
        }
        let request = match request {
            WorkspaceRequest::AgentSessions {
                providers,
                project_root,
            } => return scan_agent_history(providers, project_root),
            request => request,
        };
        let mut state = self.state.lock();
        match request {
            WorkspaceRequest::AgentSessions { .. } => unreachable!("handled before the state lock"),
            WorkspaceRequest::InstallAgentHooks => {
                let home = environment(&self.environment_id, &self.config_root)?
                    .home
                    .to_path()
                    .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?;
                yttt_agent_providers::installer::install_managed_hooks_at(&self.config_root, &home)
                    .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?;
                Ok(WorkspaceResponse::AgentHooksInstalled)
            }
            WorkspaceRequest::Environment => environment(&self.environment_id, &self.config_root)
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
            } => {
                let config = self.read_config(&relative_path)?;
                if config.as_ref().map(|value| &value.revision) != Some(&expected_revision) {
                    return Err(failure(
                        FailureCode::Conflict,
                        "configuration changed before deletion",
                        false,
                    ));
                }
                let path = self.existing_config_path(&relative_path)?.ok_or_else(|| {
                    failure(FailureCode::Conflict, "configuration was removed", false)
                })?;
                fs::remove_file(&path)
                    .map_err(|error| filesystem_failure("delete configuration", error))?;
                #[cfg(unix)]
                File::open(path.parent().unwrap())
                    .and_then(|directory| directory.sync_all())
                    .map_err(|error| filesystem_failure("persist configuration deletion", error))?;
                Ok(WorkspaceResponse::ConfigDeleted)
            }
            WorkspaceRequest::Open { workspace_id } => Ok(self.open(&state, workspace_id)),
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
            WorkspaceRequest::Commit {
                workspace_id,
                expected_revision,
                operation_id,
                snapshot,
                drafts,
            } => self.commit(
                &mut state,
                workspace_id,
                expected_revision,
                operation_id,
                snapshot,
                drafts,
            ),
            WorkspaceRequest::PutDraft {
                workspace_id,
                reference,
                content,
            } => {
                if !state.workspaces.contains_key(&workspace_id) {
                    return Err(failure(
                        FailureCode::NotFound,
                        "register the workspace before uploading drafts",
                        false,
                    ));
                }
                self.drafts
                    .put(&workspace_id, &reference, &content)
                    .map_err(|error| filesystem_failure("persist draft body", error))?;
                Ok(WorkspaceResponse::DraftStored { reference })
            }
            WorkspaceRequest::GetDraft {
                workspace_id,
                reference,
            } => {
                if !state
                    .workspaces
                    .get(&workspace_id)
                    .is_some_and(|record| record.drafts.contains(&reference))
                {
                    return Err(failure(
                        FailureCode::NotFound,
                        "draft is not published by this workspace",
                        false,
                    ));
                }
                let content = self
                    .drafts
                    .read(&workspace_id, &reference)
                    .map_err(|error| filesystem_failure("read draft body", error))?;
                Ok(WorkspaceResponse::Draft { reference, content })
            }
        }
    }

    fn open(&self, state: &ServiceState, workspace_id: WorkspaceId) -> WorkspaceResponse {
        let record = state.workspaces.get(&workspace_id);
        WorkspaceResponse::Opened {
            snapshot: record
                .map(|record| record.snapshot.clone())
                .unwrap_or_else(empty_snapshot),
            revision: record.map_or(0, |record| record.revision),
            drafts: record
                .map(|record| record.drafts.clone())
                .unwrap_or_default(),
        }
    }

    fn commit(
        &self,
        state: &mut ServiceState,
        workspace_id: WorkspaceId,
        expected_revision: u64,
        operation_id: WorkspaceOperationId,
        snapshot: WorkspaceSnapshot,
        drafts: Vec<DraftRef>,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        validate_workspace_payload(&snapshot, &drafts)?;
        let request_digest = commit_digest(expected_revision, &snapshot, &drafts)?;
        let current = {
            let record = self.workspace_for_mut(state, &workspace_id)?;

            if let Some(completed) = record
                .completed_operations
                .iter()
                .find(|completed| completed.operation_id == operation_id)
            {
                if completed.request_digest != request_digest {
                    return Err(failure(
                        FailureCode::Conflict,
                        "workspace operation ID was already used for different content",
                        false,
                    ));
                }
                return Ok(WorkspaceResponse::Committed {
                    workspace_id,
                    revision: completed.resulting_revision,
                    snapshot,
                });
            }
            require_revision(&record, expected_revision)?;
            record
        };

        self.drafts
            .validate_references(&workspace_id, &drafts)
            .map_err(|error| filesystem_failure("verify draft references", error))?;
        let resulting_revision = next_revision(current.revision)?;
        let mut updated = current;
        updated.revision = resulting_revision;
        updated.snapshot = snapshot.clone();
        let previous_drafts = std::mem::replace(&mut updated.drafts, drafts);
        updated.saved_millis = crate::now_millis();
        updated.completed_operations.push_back(CompletedOperation {
            operation_id,
            request_digest,
            resulting_revision,
        });
        if updated.completed_operations.len() > MAX_COMPLETED_OPERATIONS {
            updated.completed_operations.pop_front();
        }
        self.persist_workspace(state, &workspace_id, &updated)?;
        self.drafts
            .collect_obsolete(&workspace_id, &previous_drafts, &updated.drafts);
        state.workspaces.insert(workspace_id.clone(), updated);

        Ok(WorkspaceResponse::Committed {
            workspace_id,
            revision: resulting_revision,
            snapshot,
        })
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

    fn persist_workspace(
        &self,
        state: &mut ServiceState,
        workspace_id: &WorkspaceId,
        record: &WorkspaceRecord,
    ) -> Result<(), ProtocolFailure> {
        let bytes = serialize_workspace_record(record)?;
        let path = self.workspace_path(workspace_id);
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
        let Some(path) = self.existing_config_path(relative_path)? else {
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

    fn write_config(
        &self,
        relative_path: ProjectRelativePath,
        expected_revision: Option<WorkspaceConfigRevision>,
        bytes: Vec<u8>,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        validate_config_path(&relative_path)?;
        if bytes.len() > MAX_CONFIG_BYTES {
            return Err(failure(
                FailureCode::ResourceLimit,
                "profile configuration exceeds its size limit",
                false,
            ));
        }
        let current = self.read_config(&relative_path)?;
        if current.as_ref().map(|config| &config.revision) != expected_revision.as_ref() {
            return Err(failure(
                FailureCode::Conflict,
                "profile configuration revision does not match",
                false,
            ));
        }

        let path = self.create_config_path(&relative_path)?;
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

    fn existing_config_path(
        &self,
        relative_path: &ProjectRelativePath,
    ) -> Result<Option<PathBuf>, ProtocolFailure> {
        let mut current = self.config_root.clone();
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
        &self,
        relative_path: &ProjectRelativePath,
    ) -> Result<PathBuf, ProtocolFailure> {
        let mut current = self.config_root.clone();
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
    let sessions = scan_agent_sessions_for_agents_with_roots(&providers, &root, &roots, true)
        .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?;
    Ok(WorkspaceResponse::AgentSessions(
        sessions
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
    ))
}

fn environment(
    environment_id: &str,
    config_root: &Path,
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
    let shell = std::env::var(if cfg!(windows) { "COMSPEC" } else { "SHELL" }).ok();
    let mut shell_candidates = shell.iter().cloned().collect::<Vec<_>>();
    if let Ok(shells) = fs::read_to_string("/etc/shells") {
        for line in shells
            .lines()
            .map(str::trim)
            .filter(|line| line.starts_with('/'))
            .take(64)
        {
            if Path::new(line).is_file() && !shell_candidates.iter().any(|shell| shell == line) {
                shell_candidates.push(line.to_string());
            }
        }
    }
    Ok(WorkspaceEnvironment {
        environment_id: environment_id.to_string(),
        config_root: HostPath::from_path(config_root)
            .map_err(|error| failure(FailureCode::Internal, error.to_string(), false))?,
        shell_candidates,
        home,
        platform: std::env::consts::OS.to_string(),
        shell,
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
        let file_type = entry
            .file_type()
            .map_err(|error| filesystem_failure("inspect Host directory entry", error))?;
        let kind = if file_type.is_dir() {
            WorkspaceDirectoryEntryKind::Directory
        } else if file_type.is_file() {
            WorkspaceDirectoryEntryKind::File
        } else if file_type.is_symlink() {
            WorkspaceDirectoryEntryKind::Symlink
        } else {
            WorkspaceDirectoryEntryKind::Other
        };
        let entry_path = HostPath::from_path(&entry.path()).map_err(|error| {
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
        revision: record.revision,
        saved_millis: record.saved_millis,
    }
}

fn migrate_inline_drafts(
    objects: &crate::drafts::DraftObjects,
    workspace_id: &WorkspaceId,
    persisted: &mut PersistedWorkspaceRecord,
) -> io::Result<()> {
    let mut snapshot = persisted.snapshot.clone().into_value();
    if let Some(documents) = snapshot
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
            let draft: yttt_protocol::workspace::DraftContentRevision =
                serde_json::from_value(draft_value).map_err(io::Error::other)?;
            let key =
                serde_json::to_vec(&(document.get("project_id"), document.get("relative_path")))
                    .map_err(io::Error::other)?;
            let reference = DraftRef::for_document(&key, &draft);
            objects.put(workspace_id, &reference, draft.content.as_bytes())?;
            document
                .as_object_mut()
                .ok_or_else(|| invalid_state("document must be an object"))?
                .remove("draft");
            document["draft_ref"] = serde_json::to_value(&reference).map_err(io::Error::other)?;
            persisted.drafts.push(reference);
        }
    }
    if let Some(legacy) = persisted.draft.take() {
        for draft in legacy.contents {
            let key = serde_json::to_vec(&draft.base).map_err(io::Error::other)?;
            let reference = DraftRef::for_document(&key, &draft);
            objects.put(workspace_id, &reference, draft.content.as_bytes())?;
            if !persisted
                .drafts
                .iter()
                .any(|item| item.document_id == reference.document_id)
            {
                persisted.drafts.push(reference);
            }
        }
    }
    persisted.snapshot = WorkspaceSnapshot::new(snapshot).map_err(io::Error::other)?;
    validate_workspace_payload(&persisted.snapshot, &persisted.drafts)
        .map_err(|error| invalid_state(&error.message))?;
    persisted.schema_version = WORKSPACE_SCHEMA_VERSION;
    Ok(())
}

fn validate_workspace_payload(
    snapshot: &WorkspaceSnapshot,
    drafts: &[DraftRef],
) -> Result<(), ProtocolFailure> {
    let snapshot_bytes = serde_json::to_vec(&(snapshot.as_value(), drafts)).map_err(|error| {
        failure(
            FailureCode::Internal,
            format!("failed to encode workspace snapshot: {error}"),
            false,
        )
    })?;
    if snapshot_bytes.len() > MAX_WORKSPACE_MANIFEST_BYTES {
        return Err(failure(
            FailureCode::ResourceLimit,
            "workspace snapshot exceeds its size limit",
            false,
        ));
    }
    Ok(())
}

fn serialize_workspace_record(record: &WorkspaceRecord) -> Result<Vec<u8>, ProtocolFailure> {
    validate_workspace_payload(&record.snapshot, &record.drafts)?;
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

fn commit_digest(
    expected_revision: u64,
    snapshot: &WorkspaceSnapshot,
    drafts: &[DraftRef],
) -> Result<[u8; 32], ProtocolFailure> {
    let snapshot = serde_json::to_vec(&(snapshot.as_value(), drafts)).map_err(|error| {
        failure(
            FailureCode::Internal,
            format!("failed to encode workspace commit: {error}"),
            false,
        )
    })?;
    let mut digest = Sha256::new();
    digest.update(expected_revision.to_le_bytes());
    digest.update(snapshot);
    Ok(digest.finalize().into())
}

fn require_revision(
    record: &WorkspaceRecord,
    expected_revision: u64,
) -> Result<(), ProtocolFailure> {
    if record.revision == expected_revision {
        Ok(())
    } else {
        Err(failure(
            FailureCode::Conflict,
            "workspace revision does not match",
            false,
        ))
    }
}

fn next_revision(revision: u64) -> Result<u64, ProtocolFailure> {
    revision.checked_add(1).ok_or_else(|| {
        failure(
            FailureCode::Internal,
            "workspace revision is exhausted",
            false,
        )
    })
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
                    | "terminal-placements.json"
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

fn empty_snapshot() -> WorkspaceSnapshot {
    WorkspaceSnapshot::new(serde_json::json!({}))
        .expect("an empty JSON object is always a valid workspace snapshot")
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

#[cfg(test)]
mod tests {
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::symlink;

    use serde_json::json;
    use tempfile::tempdir;
    use yttt_core::model::ids::ClientInstanceId;
    use yttt_protocol::{
        FailureCode, ProjectRelativePath,
        workspace::{WorkspaceId, WorkspaceOperationId, WorkspaceRequest, WorkspaceSnapshot},
    };

    use super::WorkspaceService;

    fn client(name: &str) -> ClientInstanceId {
        ClientInstanceId::new(name)
    }

    fn workspace() -> WorkspaceId {
        WorkspaceId::new("default").unwrap()
    }

    fn snapshot(name: &str) -> WorkspaceSnapshot {
        WorkspaceSnapshot::new(json!({ "layout": name })).unwrap()
    }

    fn acquire(service: &WorkspaceService, client: &ClientInstanceId) {
        let id = service
            .control
            .request(
                client,
                yttt_protocol::session::ProfileControlRequest::RequestControl,
            )
            .unwrap()
            .unwrap();
        service
            .control
            .finish(&id, |revisions| service.verify_revisions(revisions))
            .unwrap();
    }

    fn transfer(service: &WorkspaceService, from: &ClientInstanceId, to: &ClientInstanceId) {
        use yttt_protocol::session::ProfileControlRequest;
        service
            .control
            .request(to, ProfileControlRequest::RequestControl)
            .unwrap();
        let transfer = service.control.status(1).transfer.unwrap();
        service
            .control
            .request(
                from,
                ProfileControlRequest::Ready {
                    transfer_id: transfer.id.clone(),
                    revisions: vec![],
                },
            )
            .unwrap();
        service
            .control
            .finish(&transfer.id, |revisions| {
                service.verify_revisions(revisions)
            })
            .unwrap();
    }

    #[test]
    fn only_the_controller_can_commit_and_takeover_revokes_the_prior_controller() {
        let root = tempdir().unwrap();
        let service = WorkspaceService::new(root.path(), root.path().join("config")).unwrap();
        let first = client("first");
        let second = client("second");
        acquire(&service, &first);

        let denied = service
            .handle(
                &second,
                WorkspaceRequest::Commit {
                    drafts: Vec::new(),
                    workspace_id: workspace(),
                    expected_revision: 0,
                    operation_id: WorkspaceOperationId::new("second-commit").unwrap(),
                    snapshot: snapshot("second"),
                },
            )
            .unwrap_err();
        assert_eq!(denied.code, FailureCode::PermissionDenied);

        transfer(&service, &first, &second);
        let denied = service
            .handle(
                &first,
                WorkspaceRequest::Commit {
                    drafts: Vec::new(),
                    workspace_id: workspace(),
                    expected_revision: 0,
                    operation_id: WorkspaceOperationId::new("first-commit").unwrap(),
                    snapshot: snapshot("first"),
                },
            )
            .unwrap_err();
        assert_eq!(denied.code, FailureCode::PermissionDenied);
    }

    #[test]
    fn another_workspace_cannot_bypass_profile_control_or_takeover() {
        let root = tempdir().unwrap();
        let service = WorkspaceService::new(root.path(), root.path().join("config")).unwrap();
        let first = client("first");
        let second = client("second");
        acquire(&service, &first);
        let alternate = WorkspaceId::new("alternate").unwrap();
        let denied = service
            .handle(
                &second,
                WorkspaceRequest::Commit {
                    drafts: Vec::new(),
                    workspace_id: alternate.clone(),
                    expected_revision: 0,
                    operation_id: WorkspaceOperationId::new("alternate-commit").unwrap(),
                    snapshot: snapshot("alternate"),
                },
            )
            .unwrap_err();
        assert_eq!(denied.code, FailureCode::PermissionDenied);
        transfer(&service, &first, &second);
        assert_eq!(
            service.authorize_mutation(&first).unwrap_err().code,
            FailureCode::PermissionDenied
        );
        let id = service
            .control
            .request(
                &second,
                yttt_protocol::session::ProfileControlRequest::Release,
            )
            .unwrap()
            .unwrap();
        service.control.finish(&id, |_| true).unwrap();
        assert_eq!(
            service.authorize_mutation(&second).unwrap_err().code,
            FailureCode::PermissionDenied
        );
    }

    #[test]
    fn disconnect_releases_control_for_the_next_client_without_erasing_workspace_state() {
        let root = tempdir().unwrap();
        let service = WorkspaceService::new(root.path(), root.path().join("config")).unwrap();
        let first = client("first");
        let second = client("second");
        acquire(&service, &first);
        service
            .handle(
                &first,
                WorkspaceRequest::Commit {
                    drafts: Vec::new(),
                    workspace_id: workspace(),
                    expected_revision: 0,
                    operation_id: WorkspaceOperationId::new("before-disconnect").unwrap(),
                    snapshot: snapshot("saved"),
                },
            )
            .unwrap();
        service.control.disconnect(&first);
        acquire(&service, &second);
        let yttt_protocol::workspace::WorkspaceResponse::Opened {
            snapshot: saved, ..
        } = service
            .handle(
                &second,
                WorkspaceRequest::Open {
                    workspace_id: workspace(),
                },
            )
            .unwrap()
        else {
            panic!("opened workspace")
        };
        assert_eq!(saved, snapshot("saved"));
    }

    #[cfg(unix)]
    #[test]
    fn profile_config_rejects_symbolic_link_escape() {
        let root = tempdir().unwrap();
        let outside = root.path().join("outside-settings");
        fs::write(&outside, b"outside").unwrap();
        let service = WorkspaceService::new(root.path(), root.path().join("config")).unwrap();
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

    #[test]
    fn persisted_commit_reopens_without_a_stale_runtime_controller() {
        let root = tempdir().unwrap();
        let owner = client("owner");
        {
            let service = WorkspaceService::new(root.path(), root.path().join("config")).unwrap();
            acquire(&service, &owner);
            service
                .handle(
                    &owner,
                    WorkspaceRequest::Commit {
                        drafts: Vec::new(),
                        workspace_id: workspace(),
                        expected_revision: 0,
                        operation_id: WorkspaceOperationId::new("persist-commit").unwrap(),
                        snapshot: snapshot("persisted"),
                    },
                )
                .unwrap();
        }

        let reloaded = WorkspaceService::new(root.path(), root.path().join("config")).unwrap();
        let opened = reloaded
            .handle(
                &client("reader"),
                WorkspaceRequest::Open {
                    workspace_id: workspace(),
                },
            )
            .unwrap();
        let yttt_protocol::workspace::WorkspaceResponse::Opened {
            revision, snapshot, ..
        } = opened
        else {
            panic!("workspace open must return its snapshot");
        };
        assert_eq!(revision, 1);
        assert_eq!(snapshot.as_value(), &json!({ "layout": "persisted" }));
        assert_eq!(reloaded.control.status(1).owner, None);
    }

    #[test]
    fn failed_atomic_replace_keeps_the_old_revision_and_can_be_retried() {
        let root = tempdir().unwrap();
        let service = WorkspaceService::new(root.path(), root.path().join("config")).unwrap();
        let owner = client("owner");
        acquire(&service, &owner);
        let state_path = root.path().join("workspaces").join("default.json");
        fs::create_dir(&state_path).unwrap();

        let failed = service
            .handle(
                &owner,
                WorkspaceRequest::Commit {
                    drafts: Vec::new(),
                    workspace_id: workspace(),
                    expected_revision: 0,
                    operation_id: WorkspaceOperationId::new("retry-commit").unwrap(),
                    snapshot: snapshot("one"),
                },
            )
            .unwrap_err();
        assert_eq!(failed.code, FailureCode::Internal);
        fs::remove_dir(&state_path).unwrap();

        let committed = service
            .handle(
                &owner,
                WorkspaceRequest::Commit {
                    drafts: Vec::new(),
                    workspace_id: workspace(),
                    expected_revision: 0,
                    operation_id: WorkspaceOperationId::new("retry-commit").unwrap(),
                    snapshot: snapshot("one"),
                },
            )
            .unwrap();
        assert!(matches!(
            committed,
            yttt_protocol::workspace::WorkspaceResponse::Committed { revision: 1, .. }
        ));
    }
}

#[cfg(test)]
mod publication_tests {
    use super::*;
    use yttt_protocol::workspace::{DraftBase, DraftContentRevision, MAX_DRAFT_CONTENT_BYTES};

    fn setup(root: &Path) -> (WorkspaceService, ClientInstanceId) {
        let service = WorkspaceService::new(root, root.join("config")).unwrap();
        let client = ClientInstanceId::new("publisher");
        let transfer = service
            .control
            .request(
                &client,
                yttt_protocol::session::ProfileControlRequest::RequestControl,
            )
            .unwrap()
            .unwrap();
        service.control.finish(&transfer, |_| true).unwrap();
        (service, client)
    }
    fn register(service: &WorkspaceService, client: &ClientInstanceId, name: &str) -> WorkspaceId {
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
    fn body(content: String) -> (DraftRef, Vec<u8>) {
        let draft = DraftContentRevision {
            revision: 9,
            base: DraftBase::Json {
                metadata: serde_json::json!({ "base": "unchanged" }),
            },
            content,
        };
        (
            DraftRef::for_document(b"stable-document", &draft),
            draft.content.into_bytes(),
        )
    }
    fn publish(
        service: &WorkspaceService,
        client: &ClientInstanceId,
        id: &WorkspaceId,
        reference: &DraftRef,
    ) -> Result<WorkspaceResponse, ProtocolFailure> {
        service.handle(
            client,
            WorkspaceRequest::Commit {
                workspace_id: id.clone(),
                expected_revision: 0,
                operation_id: WorkspaceOperationId::new("publication").unwrap(),
                snapshot: WorkspaceSnapshot::new(serde_json::json!({ "layout": "split" })).unwrap(),
                drafts: vec![reference.clone()],
            },
        )
    }

    #[test]
    fn publication_requires_local_body_and_recovers_confirmed_six_mib_content() {
        let root = tempfile::tempdir().unwrap();
        let (service, client) = setup(root.path());
        let a = register(&service, &client, "a");
        let b = register(&service, &client, "b");
        let (reference, content) = body("\0".repeat(MAX_DRAFT_CONTENT_BYTES));
        assert!(publish(&service, &client, &a, &reference).is_err());
        let put = WorkspaceRequest::PutDraft {
            workspace_id: a.clone(),
            reference: reference.clone(),
            content: content.clone(),
        };
        service.handle(&client, put.clone()).unwrap();
        service.handle(&client, put).unwrap();
        assert!(publish(&service, &client, &b, &reference).is_err());
        assert!(matches!(
            publish(&service, &client, &a, &reference).unwrap(),
            WorkspaceResponse::Committed { revision: 1, .. }
        ));
        assert!(matches!(
            publish(&service, &client, &a, &reference).unwrap(),
            WorkspaceResponse::Committed { revision: 1, .. }
        ));
        drop(service);
        let (restored, client) = setup(root.path());
        let WorkspaceResponse::Draft {
            content: recovered, ..
        } = restored
            .handle(
                &client,
                WorkspaceRequest::GetDraft {
                    workspace_id: a,
                    reference,
                },
            )
            .unwrap()
        else {
            panic!("draft")
        };
        assert_eq!(recovered, content);
        assert!(matches!(
            restored
                .handle(&client, WorkspaceRequest::Open { workspace_id: b })
                .unwrap(),
            WorkspaceResponse::Opened { revision: 0, .. }
        ));
    }

    #[test]
    fn interrupted_body_and_over_quota_manifest_leave_confirmed_drafts_recoverable() {
        let root = tempfile::tempdir().unwrap();
        let (service, client) = setup(root.path());
        let id = register(&service, &client, "a");
        let (reference, content) = body("a".repeat(MAX_DRAFT_CONTENT_BYTES));
        service
            .handle(
                &client,
                WorkspaceRequest::PutDraft {
                    workspace_id: id.clone(),
                    reference: reference.clone(),
                    content,
                },
            )
            .unwrap();
        publish(&service, &client, &id, &reference).unwrap();
        let mut oversized = Vec::new();
        for index in 0..11 {
            let mut duplicate = reference.clone();
            duplicate.document_id = WorkspaceId::new(format!("document-{index}")).unwrap();
            oversized.push(duplicate);
        }
        assert!(
            service
                .handle(
                    &client,
                    WorkspaceRequest::Commit {
                        workspace_id: id.clone(),
                        expected_revision: 1,
                        operation_id: WorkspaceOperationId::new("over-quota").unwrap(),
                        snapshot: WorkspaceSnapshot::new(
                            serde_json::json!({ "layout": "rejected" })
                        )
                        .unwrap(),
                        drafts: oversized,
                    }
                )
                .is_err()
        );
        // Simulate a process dying while atomic_write is still writing a new body.
        fs::write(
            root.path().join("drafts/a/.interrupted.1.tmp"),
            b"partial body",
        )
        .unwrap();
        drop(service);
        let (restored, client) = setup(root.path());
        let WorkspaceResponse::Opened {
            revision, drafts, ..
        } = restored
            .handle(
                &client,
                WorkspaceRequest::Open {
                    workspace_id: id.clone(),
                },
            )
            .unwrap()
        else {
            panic!("workspace")
        };
        assert_eq!(revision, 1);
        assert_eq!(drafts, vec![reference.clone()]);
        assert_eq!(
            restored.drafts.read(&id, &reference).unwrap(),
            vec![b'a'; MAX_DRAFT_CONTENT_BYTES]
        );
        assert!(!root.path().join("drafts/a/.interrupted.1.tmp").exists());
    }

    #[test]
    fn crash_between_body_and_manifest_preserves_old_revision_and_collects_orphan() {
        let root = tempfile::tempdir().unwrap();
        let (service, client) = setup(root.path());
        let id = register(&service, &client, "a");
        let (reference, content) = body("unconfirmed".to_string());
        service
            .handle(
                &client,
                WorkspaceRequest::PutDraft {
                    workspace_id: id.clone(),
                    reference: reference.clone(),
                    content,
                },
            )
            .unwrap();
        drop(service);
        let (restored, client) = setup(root.path());
        assert!(restored.drafts.read(&id, &reference).is_err());
        assert!(
            matches!(restored.handle(&client, WorkspaceRequest::Open { workspace_id: id }).unwrap(), WorkspaceResponse::Opened { revision: 0, drafts, .. } if drafts.is_empty())
        );
    }

    #[test]
    fn legacy_inline_document_migrates_before_manifest_and_retains_identity() {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir(root.path().join("workspaces")).unwrap();
        let draft = DraftContentRevision {
            revision: 7,
            base: DraftBase::Json {
                metadata: serde_json::json!({ "original_base": 3 }),
            },
            content: "unsaved".to_string(),
        };
        let snapshot = serde_json::json!({ "schema_version": 1, "documents": [{ "project_id": "project", "relative_path": "file", "draft": draft }], "terminal": "original-pty" });
        let legacy =
            serde_json::json!({ "schema_version": 1, "revision": 5, "snapshot": snapshot });
        fs::write(
            root.path().join("workspaces/default.json"),
            serde_json::to_vec(&legacy).unwrap(),
        )
        .unwrap();
        let (service, client) = setup(root.path());
        let id = WorkspaceId::new("default").unwrap();
        let WorkspaceResponse::Opened {
            snapshot,
            revision,
            drafts,
        } = service
            .handle(
                &client,
                WorkspaceRequest::Open {
                    workspace_id: id.clone(),
                },
            )
            .unwrap()
        else {
            panic!("opened")
        };
        assert_eq!(revision, 5);
        assert_eq!(snapshot.as_value()["terminal"], "original-pty");
        assert!(snapshot.as_value()["documents"][0].get("draft").is_none());
        assert_eq!(service.drafts.read(&id, &drafts[0]).unwrap(), b"unsaved");
        drop(service);
        let (restored, client) = setup(root.path());
        assert!(
            matches!(restored.handle(&client, WorkspaceRequest::Open { workspace_id: id }).unwrap(), WorkspaceResponse::Opened { revision: 5, drafts, .. } if drafts.len() == 1)
        );
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
