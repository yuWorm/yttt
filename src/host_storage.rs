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
    workspace::{WorkspaceConfigRevision, WorkspaceRequest, WorkspaceResponse},
};

pub struct HostStorage {
    client: Arc<ClientCore>,
    runtime: tokio::runtime::Handle,
    config_root: PathBuf,
    environment: yttt_protocol::workspace::WorkspaceEnvironment,
    remote: bool,
    state: Mutex<StorageState>,
}
#[derive(Default)]
struct StorageState {
    config_revisions: HashMap<PathBuf, Option<WorkspaceConfigRevision>>,
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
        Self {
            client,
            runtime,
            config_root,
            environment,
            remote,
            state: Mutex::new(StorageState::default()),
        }
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
            path: HostPath::from_path(path).map_err(io::Error::other)?,
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
        let root = directory.path.to_path().map_err(io::Error::other)?;
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
            root: HostPath::from_path(&root).map_err(io::Error::other)?,
            view_id: "configuration".to_string(),
        })?
        else {
            return Err(unexpected());
        };
        state.projects.push((root, id, registration_epoch));
        Ok(())
    }
    fn read_locked(&self, state: &mut StorageState, path: &Path) -> io::Result<Vec<u8>> {
        if path.starts_with(&self.config_root) {
            let WorkspaceResponse::Config(config) =
                self.workspace(WorkspaceRequest::ReadConfig {
                    relative_path: self.config_relative(path)?,
                })?
            else {
                return Err(unexpected());
            };
            state.config_revisions.insert(
                path.to_owned(),
                config.as_ref().map(|config| config.revision.clone()),
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
        self.with_state(|state| self.read_locked(state, path))
    }
    fn write(&self, path: &Path, bytes: &[u8]) -> io::Result<()> {
        self.with_state(|mut state| {
            if path.starts_with(&self.config_root) {
                if !state.config_revisions.contains_key(path) {
                    match self.read_locked(&mut state, path) {
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                        Err(error) => return Err(error),
                    }
                }
                let WorkspaceResponse::ConfigWritten { revision, .. } =
                    self.workspace(WorkspaceRequest::WriteConfig {
                        relative_path: self.config_relative(path)?,
                        expected_revision: state.config_revisions.get(path).cloned().flatten(),
                        bytes: bytes.to_vec(),
                    })?
                else {
                    return Err(unexpected());
                };
                state
                    .config_revisions
                    .insert(path.to_owned(), Some(revision));
                Ok(())
            } else {
                let (project_id, relative_path) = self.location(&mut state, path)?;
                if !state.file_revisions.contains_key(path) {
                    match self.read_locked(&mut state, path) {
                        Ok(_) => {}
                        Err(error) if error.kind() == io::ErrorKind::NotFound => {
                            let mut parent = relative_path.clone();
                            let name = parent.segments.pop().ok_or_else(unexpected)?;
                            self.project(ProjectRequest::CreateEntry {
                                project_id: project_id.clone(),
                                relative_parent: parent,
                                input: name.to_os_string().to_string_lossy().into_owned(),
                            })?;
                            self.read_locked(&mut state, path)?;
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
        self.with_state(|mut state| {
            if path.starts_with(&self.config_root) {
                if !state.config_revisions.contains_key(path) {
                    self.read_locked(&mut state, path)?;
                }
                let revision = state
                    .config_revisions
                    .get(path)
                    .cloned()
                    .flatten()
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::NotFound, "remote config missing")
                    })?;
                self.workspace(WorkspaceRequest::DeleteConfig {
                    relative_path: self.config_relative(path)?,
                    expected_revision: revision,
                })?;
                state.config_revisions.insert(path.to_owned(), None);
            } else {
                let (project_id, relative_path) = self.location(&mut state, path)?;
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
            .to_path()
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
