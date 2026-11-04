use crate::config::storage::ConfigStorage;
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io,
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use yttt_client_core::{ClientCore, ClientCoreError};
use yttt_core::model::ids::ProjectId;
use yttt_protocol::{
    FailureCode, HostPath, ProjectRelativePath, Request, Response,
    project::{
        ProjectFileFingerprint, ProjectRequest, ProjectResponse, ProjectSaveMode, ProjectSaveResult,
    },
    workspace::{
        WorkspaceConfigRevision, WorkspaceProjectConfigFile, WorkspaceRequest, WorkspaceResponse,
    },
};

/// How long a cached remote configuration document is served before a read
/// also refreshes it in the background.
const CONFIG_REFRESH_AFTER: Duration = Duration::from_secs(2);

/// A remote configuration document as last read or written.
struct CachedConfig {
    /// `None` when the document does not exist.
    bytes: Option<Vec<u8>>,
    revision: Option<WorkspaceConfigRevision>,
    /// The request that reads it again.
    request: WorkspaceRequest,
    fetched_at: std::time::Instant,
    refreshing: bool,
    /// Bumped by every local write, so an older refresh cannot undo it.
    generation: u64,
}

/// Remote configuration documents by client path. A remote read waits for
/// the network only the first time; later reads return the cached document
/// and refresh it in the background, as do Host change notifications.
type ConfigCache = Arc<Mutex<HashMap<PathBuf, CachedConfig>>>;

pub struct HostStorage {
    client: Arc<ClientCore>,
    runtime: tokio::runtime::Handle,
    config_root: PathBuf,
    environment: yttt_protocol::workspace::WorkspaceEnvironment,
    remote: bool,
    state: Mutex<StorageState>,
    config_cache: ConfigCache,
}
#[derive(Default)]
struct StorageState {
    config_revisions: HashMap<PathBuf, Option<WorkspaceConfigRevision>>,
    project_config_targets: HashMap<PathBuf, (HostPath, WorkspaceProjectConfigFile)>,
    file_revisions: HashMap<PathBuf, ProjectFileFingerprint>,
    projects: Vec<(PathBuf, ProjectId, u64)>,
}

pub fn request_blocking(
    client: &ClientCore,
    runtime: &tokio::runtime::Handle,
    request: Request,
) -> Result<Response, ClientCoreError> {
    let pending = client.enqueue_request(request)?;
    let (send, recv) = flume::bounded(1);
    runtime.spawn(async move {
        let _ = send.send_async(pending.wait().await).await;
    });
    recv.recv_timeout(Duration::from_secs(125))
        .map_err(|error| match error {
            flume::RecvTimeoutError::Timeout => ClientCoreError::RequestTimeout,
            flume::RecvTimeoutError::Disconnected => ClientCoreError::SupervisorStopped,
        })?
}

impl HostStorage {
    pub fn new(
        client: Arc<ClientCore>,
        runtime: tokio::runtime::Handle,
        config_root: PathBuf,
        environment: yttt_protocol::workspace::WorkspaceEnvironment,
        remote: bool,
    ) -> Self {
        let config_cache = ConfigCache::default();
        if remote {
            follow_config_changes(&client, &runtime, &config_cache);
        }
        Self {
            client,
            runtime,
            config_root,
            environment,
            remote,
            state: Mutex::new(StorageState::default()),
            config_cache,
        }
    }

    /// The cached document at `path`, refreshing it in the background once
    /// it is older than `CONFIG_REFRESH_AFTER`. `None` when not cached.
    fn cached_config(&self, path: &Path) -> Option<Option<Vec<u8>>> {
        if !self.remote {
            return None;
        }
        let mut cache = self.config_cache.lock();
        let entry = cache.get_mut(path)?;
        if entry.fetched_at.elapsed() >= CONFIG_REFRESH_AFTER && !entry.refreshing {
            entry.refreshing = true;
            refresh_config(
                &self.client,
                &self.runtime,
                &self.config_cache,
                path.to_owned(),
                entry.request.clone(),
                entry.generation,
            );
        }
        Some(entry.bytes.clone())
    }

    fn cache_config(
        &self,
        path: &Path,
        request: WorkspaceRequest,
        bytes: Option<Vec<u8>>,
        revision: Option<WorkspaceConfigRevision>,
    ) {
        if self.remote {
            let mut cache = self.config_cache.lock();
            let generation = cache.get(path).map_or(0, |entry| entry.generation + 1);
            cache.insert(
                path.to_owned(),
                CachedConfig {
                    bytes,
                    revision,
                    request,
                    fetched_at: std::time::Instant::now(),
                    refreshing: false,
                    generation,
                },
            );
        }
    }

    /// The request that reads the configuration document at `path`.
    fn config_read_request(
        &self,
        state: &StorageState,
        path: &Path,
    ) -> io::Result<WorkspaceRequest> {
        Ok(match state.project_config_targets.get(path) {
            Some((project_root, file)) => WorkspaceRequest::ReadProjectConfig {
                project_root: project_root.clone(),
                file: *file,
            },
            None => WorkspaceRequest::ReadConfig {
                relative_path: self.config_relative(path)?,
            },
        })
    }
    fn with_state<T>(
        &self,
        operation: impl FnOnce(&mut StorageState) -> io::Result<T>,
    ) -> io::Result<T> {
        let mut state = self.state.lock();
        let result = operation(&mut state);
        let registrations = std::mem::take(&mut state.projects);
        for (root, project_id, registration_epoch) in registrations {
            if let Err(error) = self.project(ProjectRequest::Close {
                project_id: project_id.clone(),
                registration_epoch,
                view_id: "configuration".to_string(),
            }) {
                eprintln!("failed to release temporary remote configuration registration: {error}");
                state.projects.push((root, project_id, registration_epoch));
            }
        }
        result
    }
    fn request(&self, request: Request) -> io::Result<Response> {
        request_blocking(&self.client, &self.runtime, request).map_err(client_error)
    }
    fn workspace(&self, request: WorkspaceRequest) -> io::Result<WorkspaceResponse> {
        match self.request(Request::Workspace(request))? {
            Response::Workspace(response) => Ok(response),
            _ => Err(unexpected()),
        }
    }
    fn project(&self, request: ProjectRequest) -> io::Result<ProjectResponse> {
        match self.request(Request::Project(request))? {
            Response::Project(response) => Ok(response),
            _ => Err(unexpected()),
        }
    }
    fn config_relative(&self, path: &Path) -> io::Result<ProjectRelativePath> {
        ProjectRelativePath::from_path(
            path.strip_prefix(&self.config_root)
                .map_err(io::Error::other)?,
        )
        .map_err(io::Error::other)
    }
    fn browse(&self, path: &Path) -> io::Result<yttt_protocol::workspace::WorkspaceDirectory> {
        match self.workspace(WorkspaceRequest::Browse {
            path: HostPath::from_client_path(path).map_err(io::Error::other)?,
            include_hidden: true,
        })? {
            WorkspaceResponse::Directory(directory) => Ok(directory),
            _ => Err(unexpected()),
        }
    }
    fn location(
        &self,
        state: &mut StorageState,
        path: &Path,
    ) -> io::Result<(ProjectId, ProjectRelativePath)> {
        if let Some((root, id, _)) = state
            .projects
            .iter()
            .filter(|(root, _, _)| path.starts_with(root))
            .max_by_key(|(root, _, _)| root.components().count())
        {
            return Ok((
                id.clone(),
                ProjectRelativePath::from_path(path.strip_prefix(root).unwrap())
                    .map_err(io::Error::other)?,
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| io::Error::other("remote configuration path has no project root"))?;
        let directory = self.browse(parent)?;
        let root = directory.path.to_client_path().map_err(io::Error::other)?;
        self.register(state, root.clone())?;
        self.location(state, &root.join(path.file_name().ok_or_else(unexpected)?))
    }
    fn register(&self, state: &mut StorageState, root: PathBuf) -> io::Result<()> {
        if state.projects.iter().any(|(known, _, _)| known == &root) {
            return Ok(());
        }
        let id = ProjectId::new(format!(
            "config-{:x}",
            Sha256::digest(root.to_string_lossy().as_bytes())
        ));
        let ProjectResponse::Registered {
            registration_epoch, ..
        } = self.project(ProjectRequest::Register {
            project_id: id.clone(),
            root: HostPath::from_client_path(&root).map_err(io::Error::other)?,
            view_id: "configuration".to_string(),
        })?
        else {
            return Err(unexpected());
        };
        state.projects.push((root, id, registration_epoch));
        Ok(())
    }
    fn read_locked(&self, state: &mut StorageState, path: &Path) -> io::Result<Vec<u8>> {
        if path.starts_with(&self.config_root) || state.project_config_targets.contains_key(path) {
            let request = self.config_read_request(state, path)?;
            let WorkspaceResponse::Config(config) = self.workspace(request.clone())? else {
                return Err(unexpected());
            };
            let revision = config.as_ref().map(|config| config.revision.clone());
            state
                .config_revisions
                .insert(path.to_owned(), revision.clone());
            self.cache_config(
                path,
                request,
                config.as_ref().map(|config| config.bytes.clone()),
                revision,
            );
            config.map(|config| config.bytes).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    "remote configuration does not exist",
                )
            })
        } else {
            let (project_id, relative_path) = self.location(state, path)?;
            let ProjectResponse::File(file) = self.project(ProjectRequest::ReadFile {
                project_id,
                relative_path,
            })?
            else {
                return Err(unexpected());
            };
            state
                .file_revisions
                .insert(path.to_owned(), file.fingerprint);
            Ok(file.text.into_bytes())
        }
    }
}

impl ConfigStorage for HostStorage {
    fn is_remote(&self) -> bool {
        self.remote
    }
    fn environment(&self) -> &yttt_protocol::workspace::WorkspaceEnvironment {
        &self.environment
    }
    fn config_root(&self) -> &Path {
        &self.config_root
    }
    fn install_agent_hooks(&self) -> io::Result<()> {
        self.workspace(WorkspaceRequest::InstallAgentHooks)
            .map(|_| ())
    }
    fn read(&self, path: &Path) -> io::Result<Vec<u8>> {
        if let Some(bytes) = self.cached_config(path) {
            return bytes.ok_or_else(not_found);
        }
        self.with_state(|state| self.read_locked(state, path))
    }
    fn read_project_config(
        &self,
        project_root: &Path,
        file: WorkspaceProjectConfigFile,
        path: &Path,
    ) -> io::Result<Vec<u8>> {
        if let Some(bytes) = self.cached_config(path) {
            return bytes.ok_or_else(not_found);
        }
        self.with_state(|state| {
            state.project_config_targets.insert(
                path.to_path_buf(),
                (
                    HostPath::from_client_path(project_root).map_err(io::Error::other)?,
                    file,
                ),
            );
            self.read_locked(state, path)
        })
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.with_state(|state| {
            if path.starts_with(&self.config_root)
                || state.project_config_targets.contains_key(path)
            {
                if !state.config_revisions.contains_key(path) {
                    match self.read_locked(state, path) {
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                // A background refresh may have seen a newer revision.
                let expected_revision = self
                    .config_cache
                    .lock()
                    .get(path)
                    .map(|entry| entry.revision.clone())
                    .unwrap_or_else(|| state.config_revisions.get(path).cloned().flatten());
                let request = match state.project_config_targets.get(path) {
                    Some((project_root, file)) => WorkspaceRequest::WriteProjectConfig {
                        project_root: project_root.clone(),
                        file: *file,
                        expected_revision,
                        bytes: bytes.to_vec(),
                    },
                    None => WorkspaceRequest::WriteConfig {
                        relative_path: self.config_relative(path)?,
                        expected_revision,
                        bytes: bytes.to_vec(),
                    },
                };
                let WorkspaceResponse::ConfigWritten { revision, .. } = self.workspace(request)?
                else {
                    return Err(unexpected());
                };
                state
                    .config_revisions
                    .insert(path.to_owned(), Some(revision.clone()));
                let read = self.config_read_request(state, path)?;
                self.cache_config(path, read, Some(bytes.to_vec()), Some(revision));
                Ok(())
            } else {
                let (project_id, relative_path) = self.location(state, path)?;
                if !state.file_revisions.contains_key(path) {
                    match self.read_locked(state, path) {
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            let mut parent = relative_path.clone();
                            let name = parent.segments.pop().ok_or_else(unexpected)?;
                            self.project(ProjectRequest::CreateEntry {
                                project_id: project_id.clone(),
                                relative_parent: parent,
                                input: name.to_os_string().to_string_lossy().into_owned(),
                            })?;
                            self.read_locked(state, path)?;
                        }
                        Err(error) => return Err(error),
                    }
                }
                let mode = ProjectSaveMode::Check(
                    state
                        .file_revisions
                        .get(path)
                        .cloned()
                        .ok_or_else(unexpected)?,
                );
                match self.project(ProjectRequest::SaveFile {
                    project_id,
                    relative_path,
                    text: String::from_utf8(bytes.to_vec()).map_err(io::Error::other)?,
                    mode,
                })? {
                    ProjectResponse::Save(ProjectSaveResult::Saved(revision)) => {
                        state.file_revisions.insert(path.to_owned(), revision);
                        Ok(())
                    }
                    ProjectResponse::Save(ProjectSaveResult::Conflict(_)) => Err(io::Error::other(
                        "remote file changed; reload before saving",
                    )),
                    _ => Err(unexpected()),
                }
            }
        })
    }
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        if path.starts_with(&self.config_root) {
            self.workspace(WorkspaceRequest::CreateConfigDirectory {
                relative_directory: self.config_relative(path)?,
            })?;
            return Ok(());
        }
        if self.is_dir(path) {
            return Ok(());
        }
        let parent = path.parent().ok_or_else(unexpected)?;
        self.create_dir_all(parent)?;
        self.with_state(|state| {
            let (project_id, relative_parent) = self.location(state, parent)?;
            self.project(ProjectRequest::CreateEntry {
                project_id,
                relative_parent,
                input: format!(
                    "{}/",
                    path.file_name().ok_or_else(unexpected)?.to_string_lossy()
                ),
            })?;
            Ok(())
        })
    }
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        self.with_state(|state| {
            if path.starts_with(&self.config_root)
                || state.project_config_targets.contains_key(path)
            {
                if !state.config_revisions.contains_key(path) {
                    self.read_locked(state, path)?;
                }
                let revision = state
                    .config_revisions
                    .get(path)
                    .cloned()
                    .flatten()
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::NotFound, "remote config missing")
                    })?;
                let request = match state.project_config_targets.get(path) {
                    Some((project_root, file)) => WorkspaceRequest::DeleteProjectConfig {
                        project_root: project_root.clone(),
                        file: *file,
                        expected_revision: revision,
                    },
                    None => WorkspaceRequest::DeleteConfig {
                        relative_path: self.config_relative(path)?,
                        expected_revision: revision,
                    },
                };
                self.workspace(request)?;
                state.config_revisions.insert(path.to_owned(), None);
                let read = self.config_read_request(state, path)?;
                self.cache_config(path, read, None, None);
            } else {
                let (project_id, relative_path) = self.location(state, path)?;
                self.project(ProjectRequest::DeleteEntry {
                    project_id,
                    relative_path,
                })?;
                state.file_revisions.remove(path);
            }
            Ok(())
        })
    }
    fn remove_directory(&self, path: &Path) -> io::Result<()> {
        self.workspace(WorkspaceRequest::RemoveConfigDirectory {
            relative_directory: self.config_relative(path)?,
        })?;
        self.config_cache
            .lock()
            .retain(|cached, _| !cached.starts_with(path));
        Ok(())
    }
    fn rename_directory(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.with_state(|state| {
            self.workspace(WorkspaceRequest::RenameConfigDirectory {
                relative_from: self.config_relative(from)?,
                relative_to: self.config_relative(to)?,
            })?;
            state
                .config_revisions
                .retain(|path, _| !path.starts_with(from) && !path.starts_with(to));
            self.config_cache
                .lock()
                .retain(|path, _| !path.starts_with(from) && !path.starts_with(to));
            Ok(())
        })
    }
    fn canonicalize(&self, path: &Path) -> io::Result<PathBuf> {
        if path.starts_with(&self.config_root) {
            // Host config paths cannot contain symlinks. Preserve the explicit root
            // spelling so canonicalizing its ancestors cannot reroute shared IO as project IO.
            let relative_directory = self.config_relative(path)?;
            self.workspace(WorkspaceRequest::ListConfig { relative_directory })?;
            return Ok(path.to_path_buf());
        }
        let path = self
            .browse(path)?
            .path
            .to_client_path()
            .map_err(io::Error::other)?;
        Ok(path)
    }
    fn read_dir(&self, path: &Path) -> io::Result<Vec<PathBuf>> {
        if path.starts_with(&self.config_root) {
            match self.workspace(WorkspaceRequest::ListConfig {
                relative_directory: self.config_relative(path)?,
            })? {
                WorkspaceResponse::ConfigDirectory(entries) => Ok(entries
                    .into_iter()
                    .map(|entry| entry.join_under(&self.config_root))
                    .collect()),
                _ => Err(unexpected()),
            }
        } else {
            let directory = self.browse(path)?;
            Ok(directory
                .entries
                .into_iter()
                .map(|entry| path.join(entry.name.to_os_string()))
                .collect())
        }
    }
    fn exists(&self, path: &Path) -> bool {
        self.read(path).is_ok() || self.is_dir(path)
    }
    fn is_dir(&self, path: &Path) -> bool {
        self.browse(path).is_ok()
    }
}

fn not_found() -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        "remote configuration does not exist",
    )
}

/// Refreshes every cached document when the Host reports a configuration
/// change, after a reconnect (changes may have been missed), or on lag.
fn follow_config_changes(
    client: &Arc<ClientCore>,
    runtime: &tokio::runtime::Handle,
    cache: &ConfigCache,
) {
    let mut events = client.subscribe_events();
    let (client, refresh_runtime, cache) = (client.clone(), runtime.clone(), cache.clone());
    runtime.spawn(async move {
        loop {
            let stale = match events.recv().await {
                Ok(yttt_client_core::ClientEvent::Server(event)) => {
                    matches!(event.body, yttt_protocol::ServerEvent::ConfigChanged)
                }
                Ok(yttt_client_core::ClientEvent::Connection(
                    yttt_client_core::ConnectionState::Ready { .. },
                )) => true,
                Ok(_) => false,
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => true,
                Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
            };
            if !stale {
                continue;
            }
            let pending = {
                let mut cache = cache.lock();
                cache
                    .iter_mut()
                    .filter(|(_, entry)| !entry.refreshing)
                    .map(|(path, entry)| {
                        entry.refreshing = true;
                        (path.clone(), entry.request.clone(), entry.generation)
                    })
                    .collect::<Vec<_>>()
            };
            for (path, request, generation) in pending {
                refresh_config(&client, &refresh_runtime, &cache, path, request, generation);
            }
        }
    });
}

/// Reads one cached document again without blocking the caller.
fn refresh_config(
    client: &Arc<ClientCore>,
    runtime: &tokio::runtime::Handle,
    cache: &ConfigCache,
    path: PathBuf,
    request: WorkspaceRequest,
    generation: u64,
) {
    let (client, cache) = (client.clone(), cache.clone());
    runtime.spawn(async move {
        let result = client.request(Request::Workspace(request)).await;
        let mut cache = cache.lock();
        let Some(entry) = cache.get_mut(&path) else {
            return;
        };
        if entry.generation != generation {
            return;
        }
        entry.refreshing = false;
        if let Ok(Response::Workspace(WorkspaceResponse::Config(config))) = result {
            entry.revision = config.as_ref().map(|config| config.revision.clone());
            entry.bytes = config.map(|config| config.bytes);
            entry.fetched_at = std::time::Instant::now();
        }
    });
}

fn unexpected() -> io::Error {
    io::Error::other("Host returned an unexpected configuration response")
}
fn client_error(error: ClientCoreError) -> io::Error {
    let kind = match &error {
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::NotFound => {
            io::ErrorKind::NotFound
        }
        ClientCoreError::Protocol(failure) if failure.code == FailureCode::PermissionDenied => {
            io::ErrorKind::PermissionDenied
        }
        _ => io::ErrorKind::Other,
    };
    io::Error::new(kind, error)
}
