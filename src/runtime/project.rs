use std::fs;
use std::{
    ffi::{OsStr, OsString},
    path::{Component, Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};

use ignore::WalkBuilder;
use ignore::{
    Match,
    gitignore::{Gitignore, GitignoreBuilder},
};
use yttt_client_core::ClientCoreError;
use yttt_core::model::{
    ids::{ConnectionId, ProjectId},
    project::{RemotePathBuf, RemotePathError, RemoteRelativePathBuf},
};
use yttt_protocol::{
    FailureCode, Request, Response,
    project::{
        PlatformArgument, PlatformPath, ProjectDirectory, ProjectEntryKind as HostEntryKind,
        ProjectEntryMutation as HostEntryMutation, ProjectFileContent, ProjectFileFingerprint,
        ProjectFileState, ProjectPasteMode as HostPasteMode, ProjectRequest, ProjectResponse,
        ProjectSaveMode, ProjectSaveResult,
    },
    ssh::{
        RemoteCommandRequest, RemoteCommandResponse, RemoteDirectory, RemoteEntryMutation,
        RemoteFileContent, RemoteFileFingerprint, RemoteFileKind, RemoteFileRequest,
        RemoteFileResponse, RemoteFileState, RemoteSaveResult,
    },
};

use super::git_status::execute_local_git;
use super::git_status::{GitCommandOutput, ProjectGitExecutor};
use crate::host_runtime::DesktopHostRuntime;

use crate::ui::{
    editor::{
        CurrentDiskState, DiskFingerprint, LoadedProjectFile, MAX_PROJECT_FILE_BYTES,
        ProjectFileIoError, SaveMode, SaveProjectFileOutcome,
    },
    project_tree::{
        DirectorySnapshot, ProjectEntryFsError, ProjectEntryMutation, ProjectEntryPasteMode,
        ProjectTreeEntry, ProjectTreeEntryKind, ProjectTreeFsError,
    },
};
use crate::ui::{
    editor::{project_relative_path, read_project_file, save_project_file},
    project_tree::{
        create_project_entry, delete_project_entry, paste_project_entry, rename_project_entry,
        scan_project_directory,
    },
};

trait ProjectHostTransport: Send + Sync {
    fn request(&self, request: Request) -> Result<Response, ClientCoreError>;
}

impl ProjectHostTransport for DesktopHostRuntime {
    fn request(&self, request: Request) -> Result<Response, ClientCoreError> {
        self.request_blocking_typed(request)
    }
}

#[derive(Clone)]
pub struct ProjectServices {
    backend: Arc<ProjectBackend>,
}

enum ProjectBackend {
    Host(HostProjectServices),
    Local(LocalProjectServices),
    Ssh(HostSshProject),
}

struct LocalProjectServices {
    root: PathBuf,
}

struct HostProjectServices {
    runtime: Arc<dyn ProjectHostTransport>,
    project_id: ProjectId,
    registration_epoch: AtomicU64,
    watch_error: Option<String>,
    root: PathBuf,
    registration_lock: Mutex<()>,
}

struct HostSshProject {
    runtime: Arc<dyn ProjectHostTransport>,
    project_id: ProjectId,
    registration_epoch: AtomicU64,
    connection_id: ConnectionId,
    root: RemotePathBuf,
    registration_lock: Mutex<()>,
}

impl HostSshProject {
    fn send(&self, request: Request) -> Result<Response, ClientCoreError> {
        self.runtime.request(request)
    }

    fn register(&self) -> Result<(), String> {
        let response = self
            .send(Request::Project(ProjectRequest::RegisterSsh {
                project_id: self.project_id.clone(),
                connection_id: self.connection_id.as_str().to_string(),
                root: self.root.as_str().to_string(),
            }))
            .map_err(|error| error.to_string())?;
        let Response::Project(ProjectResponse::Registered {
            registration_epoch, ..
        }) = response
        else {
            return Err(
                "Host returned an unexpected SSH project registration response".to_string(),
            );
        };
        self.registration_epoch
            .store(registration_epoch, Ordering::Release);
        Ok(())
    }

    fn request(&self, request: Request) -> Result<Response, String> {
        match self.send(request.clone()) {
            Ok(response) => Ok(response),
            Err(error) if HostProjectServices::registration_missing(&error) => {
                let _registration = self
                    .registration_lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match self.send(request.clone()) {
                    Ok(response) => Ok(response),
                    Err(error) if HostProjectServices::registration_missing(&error) => {
                        self.register()?;
                        self.send(request).map_err(|error| error.to_string())
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }

    fn remote_file(&self, request: RemoteFileRequest) -> Result<RemoteFileResponse, String> {
        match self.request(Request::RemoteFile(request))? {
            Response::RemoteFile(response) => Ok(response),
            _ => Err("Host returned an unexpected remote-file response".to_string()),
        }
    }

    fn remote_command(
        &self,
        program: impl Into<String>,
        args: Vec<String>,
    ) -> Result<RemoteCommandResponse, String> {
        match self.request(Request::RemoteCommand(RemoteCommandRequest {
            project_id: self.project_id.clone(),
            program: program.into(),
            args,
        }))? {
            Response::RemoteCommand(response) => Ok(response),
            _ => Err("Host returned an unexpected remote-command response".to_string()),
        }
    }
}

pub fn resolve_ssh_home(
    runtime: Arc<DesktopHostRuntime>,
    connection_id: ConnectionId,
) -> Result<RemotePathBuf, String> {
    let response = runtime
        .request_blocking_typed(Request::RemoteFile(RemoteFileRequest::ResolveHome {
            connection_id: connection_id.as_str().to_string(),
        }))
        .map_err(|error| error.to_string())?;
    let Response::RemoteFile(RemoteFileResponse::Home(home)) = response else {
        return Err("Host returned an unexpected remote-home response".to_string());
    };
    RemotePathBuf::new(home).map_err(|error| error.to_string())
}

impl HostProjectServices {
    fn send(&self, request: ProjectRequest) -> Result<Response, ClientCoreError> {
        self.runtime.request(Request::Project(request))
    }

    fn decode(response: Response) -> Result<ProjectResponse, String> {
        match response {
            Response::Project(response) => Ok(response),
            _ => Err("Host returned an unexpected project response".to_string()),
        }
    }

    fn registration_missing(error: &ClientCoreError) -> bool {
        matches!(
            error,
            ClientCoreError::Protocol(failure)
                if failure.code == FailureCode::NotFound
                    && failure.message.starts_with("project is not registered with Host:")
        )
    }

    fn register(&self) -> Result<(), String> {
        let response = Self::decode(
            self.send(ProjectRequest::Register {
                project_id: self.project_id.clone(),
                root: path_to_platform(&self.root),
            })
            .map_err(|error| error.to_string())?,
        )?;
        let ProjectResponse::Registered {
            registration_epoch, ..
        } = response
        else {
            return Err("Host returned an unexpected project registration response".to_string());
        };
        self.registration_epoch
            .store(registration_epoch, Ordering::Release);
        Ok(())
    }

    fn request(&self, request: ProjectRequest) -> Result<ProjectResponse, String> {
        match self.send(request.clone()) {
            Ok(response) => Self::decode(response),
            Err(error) if Self::registration_missing(&error) => {
                let _registration = self
                    .registration_lock
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                match self.send(request.clone()) {
                    Ok(response) => Self::decode(response),
                    Err(error) if Self::registration_missing(&error) => {
                        self.register()?;
                        Self::decode(self.send(request).map_err(|error| error.to_string())?)
                    }
                    Err(error) => Err(error.to_string()),
                }
            }
            Err(error) => Err(error.to_string()),
        }
    }
}

impl ProjectServices {
    pub(crate) fn local_for_test(root: impl Into<PathBuf>) -> Self {
        Self {
            backend: Arc::new(ProjectBackend::Local(LocalProjectServices {
                root: root.into(),
            })),
        }
    }

    pub fn ssh(
        runtime: Arc<DesktopHostRuntime>,
        project_id: ProjectId,
        connection_id: ConnectionId,
        root: RemotePathBuf,
    ) -> Result<Self, String> {
        Self::ssh_with_transport(runtime, project_id, connection_id, root)
    }

    fn ssh_with_transport(
        runtime: Arc<dyn ProjectHostTransport>,
        project_id: ProjectId,
        connection_id: ConnectionId,
        root: RemotePathBuf,
    ) -> Result<Self, String> {
        let project = HostSshProject {
            runtime,
            project_id,
            registration_epoch: AtomicU64::new(0),
            connection_id,
            root,
            registration_lock: Mutex::new(()),
        };
        project.register()?;
        Ok(Self {
            backend: Arc::new(ProjectBackend::Ssh(project)),
        })
    }

    pub fn host(
        runtime: Arc<DesktopHostRuntime>,
        project_id: ProjectId,
        root: impl Into<PathBuf>,
    ) -> Result<Self, String> {
        Self::host_with_transport(runtime, project_id, root.into())
    }

    fn host_with_transport(
        runtime: Arc<dyn ProjectHostTransport>,
        project_id: ProjectId,
        root: PathBuf,
    ) -> Result<Self, String> {
        let response = runtime
            .request(Request::Project(ProjectRequest::Register {
                project_id: project_id.clone(),
                root: path_to_platform(&root),
            }))
            .map_err(|error| error.to_string())?;
        let Response::Project(ProjectResponse::Registered {
            canonical_root,
            registration_epoch,
            watch_error,
            ..
        }) = response
        else {
            return Err("Host returned an unexpected project registration response".to_string());
        };
        Ok(Self {
            backend: Arc::new(ProjectBackend::Host(HostProjectServices {
                runtime,
                project_id,
                registration_epoch: AtomicU64::new(registration_epoch),
                watch_error,
                root: platform_path(canonical_root)?,
                registration_lock: Mutex::new(()),
            })),
        })
    }

    pub fn host_registration_epoch(&self) -> Option<u64> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => Some(host.registration_epoch.load(Ordering::Acquire)),
            ProjectBackend::Local(_) => None,
            ProjectBackend::Ssh(project) => {
                Some(project.registration_epoch.load(Ordering::Acquire))
            }
        }
    }

    pub fn close_host_registration(&self) -> Result<(), String> {
        let (response, expected) = match self.backend.as_ref() {
            ProjectBackend::Host(host) => (
                host.send(ProjectRequest::Close {
                    project_id: host.project_id.clone(),
                    registration_epoch: host.registration_epoch.load(Ordering::Acquire),
                })
                .map_err(|error| error.to_string())?,
                "project",
            ),
            ProjectBackend::Ssh(project) => (
                project.request(Request::Project(ProjectRequest::Close {
                    project_id: project.project_id.clone(),
                    registration_epoch: project.registration_epoch.load(Ordering::Acquire),
                }))?,
                "SSH project",
            ),
            ProjectBackend::Local(_) => return Ok(()),
        };
        match response {
            Response::Project(ProjectResponse::Closed) => Ok(()),
            _ => Err(format!(
                "Host returned an unexpected {expected} close response"
            )),
        }
    }

    pub fn watch_error(&self) -> Option<&str> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => host.watch_error.as_deref(),
            ProjectBackend::Local(_) => None,
            ProjectBackend::Ssh(_) => None,
        }
    }

    pub fn document_path(&self, relative_path: &Path) -> Option<PathBuf> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => Some(host.root.join(relative_path)),
            ProjectBackend::Local(local) => Some(local.root.join(relative_path)),
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_path).ok()?;
                Some(remote_document_path(project, &relative))
            }
        }
    }

    pub fn relative_path_for_document(
        &self,
        document_path: &Path,
    ) -> Result<PathBuf, ProjectFileIoError> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => host_relative_path(&host.root, document_path),
            ProjectBackend::Local(local) => project_relative_path(&local.root, document_path),
            ProjectBackend::Ssh(project) => {
                let root = PathBuf::from(project.root.as_str());
                let relative = document_path.strip_prefix(&root).map_err(|_| {
                    ProjectFileIoError::PathOutsideProject {
                        path: document_path.to_path_buf(),
                    }
                })?;
                let relative = remote_relative(relative).map_err(|_| {
                    ProjectFileIoError::PathOutsideProject {
                        path: document_path.to_path_buf(),
                    }
                })?;
                if relative.as_str().is_empty() {
                    return Err(ProjectFileIoError::PathOutsideProject {
                        path: document_path.to_path_buf(),
                    });
                }
                Ok(pathbuf_from_remote(&relative))
            }
        }
    }

    pub fn scan_directory(
        &self,
        relative_directory: &Path,
        show_hidden: bool,
    ) -> Result<DirectorySnapshot, ProjectTreeFsError> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => host
                .request(ProjectRequest::ScanDirectory {
                    project_id: host.project_id.clone(),
                    relative_directory: path_to_platform(relative_directory),
                    show_hidden,
                })
                .and_then(|response| match response {
                    ProjectResponse::Directory(snapshot) => Ok(snapshot),
                    _ => Err("Host returned an unexpected directory response".to_string()),
                })
                .and_then(host_directory_snapshot)
                .map_err(|message| tree_remote_error(relative_directory, message)),
            ProjectBackend::Local(local) => {
                scan_project_directory(&local.root, relative_directory, show_hidden)
            }
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_directory)
                    .map_err(|message| tree_remote_error(relative_directory, message))?;
                let response = project
                    .remote_file(RemoteFileRequest::ScanDirectory {
                        project_id: project.project_id.clone(),
                        relative_directory: relative.as_str().to_string(),
                        show_hidden,
                    })
                    .map_err(|message| tree_remote_error(relative_directory, message))?;
                let RemoteFileResponse::Directory(snapshot) = response else {
                    return Err(tree_remote_error(
                        relative_directory,
                        "Host returned an unexpected remote directory response".to_string(),
                    ));
                };
                remote_directory_snapshot(snapshot)
                    .map_err(|message| tree_remote_error(relative_directory, message))
            }
        }
    }

    pub fn searchable_files(&self, show_hidden: bool) -> Result<Vec<PathBuf>, String> {
        if let Some(paths) = searchable_git_files(self, show_hidden) {
            return Ok(paths);
        }

        match self.backend.as_ref() {
            ProjectBackend::Local(local) => searchable_local_files(&local.root, show_hidden),
            ProjectBackend::Host(_) => searchable_remote_files(self, show_hidden),
            ProjectBackend::Ssh(_) => searchable_remote_files(self, show_hidden),
        }
    }

    pub fn read_file(&self, relative_path: &Path) -> Result<LoadedProjectFile, ProjectFileIoError> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => host
                .request(ProjectRequest::ReadFile {
                    project_id: host.project_id.clone(),
                    relative_path: path_to_platform(relative_path),
                })
                .and_then(|response| match response {
                    ProjectResponse::File(file) => host_loaded_file(file),
                    _ => Err("Host returned an unexpected file response".to_string()),
                })
                .map_err(|message| file_remote_error(relative_path, message)),
            ProjectBackend::Local(local) => read_project_file(&local.root, relative_path),
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_path)
                    .map_err(|message| file_remote_error(relative_path, message))?;
                let response = project
                    .remote_file(RemoteFileRequest::Read {
                        project_id: project.project_id.clone(),
                        relative_path: relative.as_str().to_string(),
                        maximum_bytes: MAX_PROJECT_FILE_BYTES,
                    })
                    .map_err(|message| file_remote_error(relative_path, message))?;
                let RemoteFileResponse::File(file) = response else {
                    return Err(file_remote_error(
                        relative_path,
                        "Host returned an unexpected remote file response".to_string(),
                    ));
                };
                remote_loaded_file(project, file)
            }
        }
    }

    pub fn save_file(
        &self,
        relative_path: &Path,
        text: &str,
        expected: Option<&DiskFingerprint>,
        force: bool,
    ) -> Result<SaveProjectFileOutcome, ProjectFileIoError> {
        let mode = if force {
            SaveMode::Force
        } else {
            SaveMode::Check(expected.expect("checked save requires an expected file version"))
        };
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => {
                let mode = match mode {
                    SaveMode::Check(fingerprint) => {
                        ProjectSaveMode::Check(fingerprint_to_host(fingerprint))
                    }
                    SaveMode::Force => ProjectSaveMode::Force,
                };
                host.request(ProjectRequest::SaveFile {
                    project_id: host.project_id.clone(),
                    relative_path: path_to_platform(relative_path),
                    text: text.to_string(),
                    mode,
                })
                .and_then(|response| match response {
                    ProjectResponse::Save(outcome) => host_save_outcome(outcome),
                    _ => Err("Host returned an unexpected save response".to_string()),
                })
                .map_err(|message| file_remote_error(relative_path, message))
            }
            ProjectBackend::Local(local) => {
                save_project_file(&local.root, relative_path, text, mode)
            }
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_path)
                    .map_err(|message| file_remote_error(relative_path, message))?;
                let expected = match mode {
                    SaveMode::Check(fingerprint) if fingerprint.exists => {
                        Some(remote_fingerprint(fingerprint))
                    }
                    SaveMode::Check(_) | SaveMode::Force => None,
                };
                let response = project
                    .remote_file(RemoteFileRequest::Save {
                        project_id: project.project_id.clone(),
                        relative_path: relative.as_str().to_string(),
                        expected,
                        force,
                        maximum_bytes: MAX_PROJECT_FILE_BYTES,
                        bytes: text.as_bytes().to_vec(),
                    })
                    .map_err(|message| file_remote_error(relative_path, message))?;
                let RemoteFileResponse::Save(outcome) = response else {
                    return Err(file_remote_error(
                        relative_path,
                        "Host returned an unexpected remote save response".to_string(),
                    ));
                };
                Ok(remote_save_outcome(outcome))
            }
        }
    }

    pub fn create_entry(
        &self,
        relative_directory: &Path,
        input: &str,
    ) -> Result<ProjectEntryMutation, ProjectEntryFsError> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => host
                .request(ProjectRequest::CreateEntry {
                    project_id: host.project_id.clone(),
                    relative_parent: path_to_platform(relative_directory),
                    input: input.to_string(),
                })
                .and_then(|response| match response {
                    ProjectResponse::Mutation(mutation) => host_entry_mutation(mutation),
                    _ => Err("Host returned an unexpected create response".to_string()),
                })
                .map_err(|message| entry_remote_error(relative_directory, message)),
            ProjectBackend::Local(local) => {
                create_project_entry(&local.root, relative_directory, input)
            }
            ProjectBackend::Ssh(project) => {
                let directory = input.ends_with('/');
                let entry_input = input.trim_end_matches('/');
                if entry_input.is_empty() {
                    return Err(ProjectEntryFsError::InvalidEntryName {
                        input: input.to_string(),
                    });
                }
                let parent = remote_relative(relative_directory).map_err(|_| {
                    ProjectEntryFsError::InvalidEntryName {
                        input: input.to_string(),
                    }
                })?;
                let input_path = RemoteRelativePathBuf::new(entry_input).map_err(|_| {
                    ProjectEntryFsError::InvalidEntryName {
                        input: input.to_string(),
                    }
                })?;
                if input_path.as_str().is_empty() {
                    return Err(ProjectEntryFsError::InvalidEntryName {
                        input: input.to_string(),
                    });
                }
                let relative_path = combine_remote(&parent, &input_path).map_err(|_| {
                    ProjectEntryFsError::InvalidEntryName {
                        input: input.to_string(),
                    }
                })?;
                let response = project
                    .remote_file(RemoteFileRequest::Create {
                        project_id: project.project_id.clone(),
                        relative_path: relative_path.as_str().to_string(),
                        directory,
                    })
                    .map_err(|message| entry_remote_error(relative_directory, message))?;
                let RemoteFileResponse::Mutation(mutation) = response else {
                    return Err(entry_remote_error(
                        relative_directory,
                        "Host returned an unexpected remote create response".to_string(),
                    ));
                };
                Ok(remote_entry_mutation(mutation))
            }
        }
    }

    pub fn rename_entry(
        &self,
        relative_path: &Path,
        new_name: &str,
    ) -> Result<ProjectEntryMutation, ProjectEntryFsError> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => host
                .request(ProjectRequest::RenameEntry {
                    project_id: host.project_id.clone(),
                    relative_path: path_to_platform(relative_path),
                    new_name: new_name.to_string(),
                })
                .and_then(|response| match response {
                    ProjectResponse::Mutation(mutation) => host_entry_mutation(mutation),
                    _ => Err("Host returned an unexpected rename response".to_string()),
                })
                .map_err(|message| entry_remote_error(relative_path, message)),
            ProjectBackend::Local(local) => {
                rename_project_entry(&local.root, relative_path, new_name)
            }
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_path).map_err(|_| {
                    ProjectEntryFsError::InvalidEntryName {
                        input: new_name.to_string(),
                    }
                })?;
                let response = project
                    .remote_file(RemoteFileRequest::Rename {
                        project_id: project.project_id.clone(),
                        relative_path: relative.as_str().to_string(),
                        new_name: new_name.to_string(),
                    })
                    .map_err(|message| entry_remote_error(relative_path, message))?;
                let RemoteFileResponse::Mutation(mutation) = response else {
                    return Err(entry_remote_error(
                        relative_path,
                        "Host returned an unexpected remote rename response".to_string(),
                    ));
                };
                Ok(remote_entry_mutation(mutation))
            }
        }
    }

    pub fn delete_entry(&self, relative_path: &Path) -> Result<(), ProjectEntryFsError> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => host
                .request(ProjectRequest::DeleteEntry {
                    project_id: host.project_id.clone(),
                    relative_path: path_to_platform(relative_path),
                })
                .and_then(|response| match response {
                    ProjectResponse::Deleted => Ok(()),
                    _ => Err("Host returned an unexpected delete response".to_string()),
                })
                .map_err(|message| entry_remote_error(relative_path, message)),
            ProjectBackend::Local(local) => delete_project_entry(&local.root, relative_path),
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_path)
                    .map_err(|message| entry_remote_error(relative_path, message))?;
                let response = project
                    .remote_file(RemoteFileRequest::Delete {
                        project_id: project.project_id.clone(),
                        relative_path: relative.as_str().to_string(),
                    })
                    .map_err(|message| entry_remote_error(relative_path, message))?;
                if matches!(response, RemoteFileResponse::Deleted) {
                    Ok(())
                } else {
                    Err(entry_remote_error(
                        relative_path,
                        "Host returned an unexpected remote delete response".to_string(),
                    ))
                }
            }
        }
    }

    pub fn paste_entry(
        &self,
        source_relative_path: &Path,
        destination: &ProjectServices,
        destination_relative_directory: &Path,
        mode: ProjectEntryPasteMode,
    ) -> Result<ProjectEntryMutation, ProjectEntryFsError> {
        match (self.backend.as_ref(), destination.backend.as_ref()) {
            (ProjectBackend::Host(source), ProjectBackend::Host(destination))
                if Arc::ptr_eq(&source.runtime, &destination.runtime) =>
            {
                let mode = match mode {
                    ProjectEntryPasteMode::Copy => HostPasteMode::Copy,
                    ProjectEntryPasteMode::Cut => HostPasteMode::Cut,
                };
                source
                    .request(ProjectRequest::PasteEntry {
                        source_project_id: source.project_id.clone(),
                        source_relative_path: path_to_platform(source_relative_path),
                        destination_project_id: destination.project_id.clone(),
                        destination_relative_directory: path_to_platform(
                            destination_relative_directory,
                        ),
                        mode,
                    })
                    .and_then(|response| match response {
                        ProjectResponse::Mutation(mutation) => host_entry_mutation(mutation),
                        _ => Err("Host returned an unexpected paste response".to_string()),
                    })
                    .map_err(|message| entry_remote_error(source_relative_path, message))
            }
            (ProjectBackend::Local(source), ProjectBackend::Local(destination)) => {
                paste_project_entry(
                    &source.root,
                    source_relative_path,
                    &destination.root,
                    destination_relative_directory,
                    mode,
                )
            }
            _ => Err(ProjectEntryFsError::UnsupportedOperation {
                operation: "paste entries across different project backends",
            }),
        }
    }
}
impl ProjectGitExecutor for ProjectServices {
    fn execute_git(
        &self,
        args: &[OsString],
        optional_locks: bool,
    ) -> Result<GitCommandOutput, String> {
        match self.backend.as_ref() {
            ProjectBackend::Host(host) => {
                let response = host.request(ProjectRequest::Git {
                    project_id: host.project_id.clone(),
                    args: args.iter().map(os_string_to_platform).collect(),
                    optional_locks,
                })?;
                let ProjectResponse::Git(output) = response else {
                    return Err("Host returned an unexpected Git response".to_string());
                };
                Ok(GitCommandOutput {
                    success: output.success,
                    exit_code: output.exit_code,
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            }
            ProjectBackend::Local(local) => execute_local_git(&local.root, args, optional_locks),
            ProjectBackend::Ssh(project) => {
                let args = args
                    .iter()
                    .map(|arg| {
                        arg.to_str()
                            .map(str::to_string)
                            .ok_or_else(|| "remote Git arguments must be valid UTF-8".to_string())
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                let output = project.remote_command("git", args)?;
                Ok(GitCommandOutput {
                    success: output.exit_status == 0,
                    exit_code: i32::try_from(output.exit_status).ok(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            }
        }
    }

    fn null_device_path(&self) -> &'static str {
        match self.backend.as_ref() {
            ProjectBackend::Host(_) if cfg!(windows) => "NUL",
            ProjectBackend::Host(_) => "/dev/null",
            ProjectBackend::Local(_) if cfg!(windows) => "NUL",
            ProjectBackend::Local(_) => "/dev/null",
            ProjectBackend::Ssh(_) => "/dev/null",
        }
    }
}

fn host_relative_path(root: &Path, document_path: &Path) -> Result<PathBuf, ProjectFileIoError> {
    let relative =
        document_path
            .strip_prefix(root)
            .map_err(|_| ProjectFileIoError::PathOutsideProject {
                path: document_path.to_path_buf(),
            })?;
    if relative.as_os_str().is_empty()
        || relative.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err(ProjectFileIoError::PathOutsideProject {
            path: document_path.to_path_buf(),
        });
    }
    Ok(relative.to_path_buf())
}

fn host_directory_snapshot(snapshot: ProjectDirectory) -> Result<DirectorySnapshot, String> {
    Ok(DirectorySnapshot {
        relative_directory: platform_path(snapshot.relative_directory)?,
        entries: snapshot
            .entries
            .into_iter()
            .map(|entry| {
                Ok(ProjectTreeEntry {
                    name: platform_argument(entry.name)?,
                    relative_path: platform_path(entry.relative_path)?,
                    kind: host_entry_kind(entry.kind),
                })
            })
            .collect::<Result<Vec<_>, String>>()?,
    })
}

fn host_loaded_file(file: ProjectFileContent) -> Result<LoadedProjectFile, String> {
    Ok(LoadedProjectFile {
        canonical_path: platform_path(file.canonical_path)?,
        relative_path: platform_path(file.relative_path)?,
        text: file.text,
        fingerprint: fingerprint_from_host(file.fingerprint)?,
    })
}

fn fingerprint_to_host(fingerprint: &DiskFingerprint) -> ProjectFileFingerprint {
    ProjectFileFingerprint {
        exists: fingerprint.exists,
        byte_len: fingerprint.byte_len,
        modified_nanos: fingerprint.modified.and_then(|modified| {
            modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .map(|duration| duration.as_nanos())
        }),
        content_hash: fingerprint.content_hash,
    }
}

fn fingerprint_from_host(fingerprint: ProjectFileFingerprint) -> Result<DiskFingerprint, String> {
    let modified = fingerprint
        .modified_nanos
        .map(|nanos| {
            let nanos = u64::try_from(nanos)
                .map_err(|_| "project file timestamp exceeds platform range".to_string())?;
            Ok::<_, String>(UNIX_EPOCH + Duration::from_nanos(nanos))
        })
        .transpose()?;
    Ok(DiskFingerprint {
        exists: fingerprint.exists,
        byte_len: fingerprint.byte_len,
        modified,
        content_hash: fingerprint.content_hash,
    })
}

fn host_save_outcome(outcome: ProjectSaveResult) -> Result<SaveProjectFileOutcome, String> {
    match outcome {
        ProjectSaveResult::Saved(fingerprint) => Ok(SaveProjectFileOutcome::Saved(
            fingerprint_from_host(fingerprint)?,
        )),
        ProjectSaveResult::Conflict(ProjectFileState::Missing) => {
            Ok(SaveProjectFileOutcome::Conflict(CurrentDiskState::Missing))
        }
        ProjectSaveResult::Conflict(ProjectFileState::Present(fingerprint)) => {
            Ok(SaveProjectFileOutcome::Conflict(CurrentDiskState::Present(
                fingerprint_from_host(fingerprint)?,
            )))
        }
    }
}

fn host_entry_mutation(mutation: HostEntryMutation) -> Result<ProjectEntryMutation, String> {
    Ok(ProjectEntryMutation {
        relative_path: platform_path(mutation.relative_path)?,
        kind: host_entry_kind(mutation.kind),
    })
}

fn host_entry_kind(kind: HostEntryKind) -> ProjectTreeEntryKind {
    match kind {
        HostEntryKind::Directory => ProjectTreeEntryKind::Directory,
        HostEntryKind::File => ProjectTreeEntryKind::File,
        HostEntryKind::SymlinkFile => ProjectTreeEntryKind::SymlinkFile,
        HostEntryKind::SymlinkDirectory => ProjectTreeEntryKind::SymlinkDirectory,
    }
}

#[cfg(unix)]
pub(crate) fn path_to_platform(path: &Path) -> PlatformPath {
    use std::os::unix::ffi::OsStrExt as _;
    PlatformPath::Unix(path.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
pub(crate) fn path_to_platform(path: &Path) -> PlatformPath {
    use std::os::windows::ffi::OsStrExt as _;
    PlatformPath::Windows(path.as_os_str().encode_wide().collect())
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn path_to_platform(path: &Path) -> PlatformPath {
    PlatformPath::Unix(path.to_string_lossy().as_bytes().to_vec())
}

#[cfg(unix)]
pub(crate) fn platform_path(path: PlatformPath) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt as _;
    match path {
        PlatformPath::Unix(bytes) => Ok(PathBuf::from(OsString::from_vec(bytes))),
        PlatformPath::Windows(_) => Err("received a Windows path on a Unix client".to_string()),
    }
}

#[cfg(windows)]
pub(crate) fn platform_path(path: PlatformPath) -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt as _;
    match path {
        PlatformPath::Windows(wide) => Ok(PathBuf::from(OsString::from_wide(&wide))),
        PlatformPath::Unix(_) => Err("received a Unix path on a Windows client".to_string()),
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn platform_path(_path: PlatformPath) -> Result<PathBuf, String> {
    Err("project paths are unsupported on this platform".to_string())
}

#[cfg(unix)]
fn os_string_to_platform(value: &OsString) -> PlatformArgument {
    use std::os::unix::ffi::OsStrExt as _;
    PlatformArgument::Unix(value.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
fn os_string_to_platform(value: &OsString) -> PlatformArgument {
    use std::os::windows::ffi::OsStrExt as _;
    PlatformArgument::Windows(value.encode_wide().collect())
}

#[cfg(not(any(unix, windows)))]
fn os_string_to_platform(value: &OsString) -> PlatformArgument {
    PlatformArgument::Unix(value.to_string_lossy().as_bytes().to_vec())
}

#[cfg(unix)]
fn platform_argument(value: PlatformArgument) -> Result<OsString, String> {
    use std::os::unix::ffi::OsStringExt as _;
    match value {
        PlatformArgument::Unix(bytes) => Ok(OsString::from_vec(bytes)),
        PlatformArgument::Windows(_) => {
            Err("received a Windows argument on a Unix client".to_string())
        }
    }
}

#[cfg(windows)]
fn platform_argument(value: PlatformArgument) -> Result<OsString, String> {
    use std::os::windows::ffi::OsStringExt as _;
    match value {
        PlatformArgument::Windows(wide) => Ok(OsString::from_wide(&wide)),
        PlatformArgument::Unix(_) => {
            Err("received a Unix argument on a Windows client".to_string())
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_argument(_value: PlatformArgument) -> Result<OsString, String> {
    Err("project arguments are unsupported on this platform".to_string())
}

fn searchable_git_files(services: &ProjectServices, show_hidden: bool) -> Option<Vec<PathBuf>> {
    let args = [
        OsString::from("ls-files"),
        OsString::from("--cached"),
        OsString::from("--others"),
        OsString::from("--exclude-standard"),
        OsString::from("-z"),
        OsString::from("--"),
    ];
    let output = services.execute_git(&args, false).ok()?;
    if !output.success {
        return None;
    }

    let mut paths = output
        .stdout
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .map(|path| String::from_utf8(path.to_vec()).map(PathBuf::from))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    paths.retain(|path| show_hidden || !path_has_hidden_component(path));
    sort_searchable_paths(&mut paths);
    Some(paths)
}

fn searchable_local_files(root: &Path, show_hidden: bool) -> Result<Vec<PathBuf>, String> {
    let mut builder = WalkBuilder::new(root);
    builder
        .follow_links(false)
        .hidden(!show_hidden)
        .require_git(false)
        .filter_entry(|entry| entry.file_name() != OsStr::new(".git"));

    let mut paths = Vec::new();
    for entry in builder.build() {
        let entry = entry.map_err(|error| error.to_string())?;
        let Some(file_type) = entry.file_type() else {
            continue;
        };
        let is_file = file_type.is_file()
            || (file_type.is_symlink()
                && fs::metadata(entry.path())
                    .map(|metadata| metadata.is_file())
                    .unwrap_or(false));
        if !is_file {
            continue;
        }
        let relative = entry
            .path()
            .strip_prefix(root)
            .map_err(|_| {
                format!(
                    "file search path escaped project root: {}",
                    entry.path().display()
                )
            })?
            .to_path_buf();
        paths.push(relative);
    }
    sort_searchable_paths(&mut paths);
    Ok(paths)
}

fn searchable_remote_files(
    services: &ProjectServices,
    show_hidden: bool,
) -> Result<Vec<PathBuf>, String> {
    let mut paths = Vec::new();
    let mut pending = vec![(PathBuf::new(), Vec::<Gitignore>::new())];

    while let Some((directory, mut matchers)) = pending.pop() {
        let snapshot = services
            .scan_directory(&directory, true)
            .map_err(|error| error.to_string())?;
        let ignore_path = directory.join(".gitignore");
        if snapshot.entries.iter().any(|entry| {
            entry.name == OsStr::new(".gitignore")
                && matches!(
                    entry.kind,
                    ProjectTreeEntryKind::File | ProjectTreeEntryKind::SymlinkFile
                )
        }) {
            let loaded = services
                .read_file(&ignore_path)
                .map_err(|error| error.to_string())?;
            let mut builder = GitignoreBuilder::new(&directory);
            for line in loaded.text.lines() {
                builder
                    .add_line(Some(ignore_path.clone()), line)
                    .map_err(|error| error.to_string())?;
            }
            matchers.push(builder.build().map_err(|error| error.to_string())?);
        }

        for entry in snapshot.entries {
            if entry.name == OsStr::new(".git") {
                continue;
            }
            let is_directory = entry.kind.is_directory();
            if (!show_hidden && path_has_hidden_component(&entry.relative_path))
                || path_is_ignored(&matchers, &entry.relative_path, is_directory)
            {
                continue;
            }
            match entry.kind {
                ProjectTreeEntryKind::Directory => {
                    pending.push((entry.relative_path, matchers.clone()));
                }
                ProjectTreeEntryKind::File | ProjectTreeEntryKind::SymlinkFile => {
                    paths.push(entry.relative_path);
                }
                ProjectTreeEntryKind::SymlinkDirectory => {}
            }
        }
    }

    sort_searchable_paths(&mut paths);
    Ok(paths)
}

fn path_is_ignored(matchers: &[Gitignore], path: &Path, is_directory: bool) -> bool {
    for matcher in matchers.iter().rev() {
        match matcher.matched(path, is_directory) {
            Match::Ignore(_) => return true,
            Match::Whitelist(_) => return false,
            Match::None => {}
        }
    }
    false
}

fn path_has_hidden_component(path: &Path) -> bool {
    path.components().any(|component| {
        matches!(component, Component::Normal(name) if name.to_string_lossy().starts_with('.'))
    })
}

fn sort_searchable_paths(paths: &mut Vec<PathBuf>) {
    paths.sort_by(|left, right| {
        left.to_string_lossy()
            .to_lowercase()
            .cmp(&right.to_string_lossy().to_lowercase())
            .then_with(|| left.cmp(right))
    });
    paths.dedup();
}

fn remote_relative(path: &Path) -> Result<RemoteRelativePathBuf, String> {
    let mut components = Vec::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => components.push(
                component
                    .to_str()
                    .ok_or_else(|| "remote project paths must be valid UTF-8".to_string())?,
            ),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err("remote project path escapes its root".to_string());
            }
        }
    }
    RemoteRelativePathBuf::new(components.join("/")).map_err(|error| error.to_string())
}

fn combine_remote(
    parent: &RemoteRelativePathBuf,
    child: &RemoteRelativePathBuf,
) -> Result<RemoteRelativePathBuf, RemotePathError> {
    if parent.as_str().is_empty() {
        return Ok(child.clone());
    }
    if child.as_str().is_empty() {
        return Ok(parent.clone());
    }
    RemoteRelativePathBuf::new(format!("{}/{}", parent.as_str(), child.as_str()))
}

fn pathbuf_from_remote(path: &RemoteRelativePathBuf) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path
        .as_str()
        .split('/')
        .filter(|component| !component.is_empty())
    {
        result.push(component);
    }
    result
}

fn remote_document_path(project: &HostSshProject, relative: &RemoteRelativePathBuf) -> PathBuf {
    let mut path = PathBuf::from(project.root.as_str());
    for component in relative
        .as_str()
        .split('/')
        .filter(|component| !component.is_empty())
    {
        path.push(component);
    }
    path
}

fn pathbuf_from_wire(path: &str) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.split('/').filter(|component| !component.is_empty()) {
        result.push(component);
    }
    result
}

fn remote_directory_snapshot(snapshot: RemoteDirectory) -> Result<DirectorySnapshot, String> {
    Ok(DirectorySnapshot {
        relative_directory: pathbuf_from_wire(&snapshot.relative_directory),
        entries: snapshot
            .entries
            .into_iter()
            .map(|entry| ProjectTreeEntry {
                name: OsString::from(entry.name),
                relative_path: pathbuf_from_wire(&entry.relative_path),
                kind: project_entry_kind(entry.kind),
            })
            .collect(),
    })
}

fn remote_loaded_file(
    project: &HostSshProject,
    file: RemoteFileContent,
) -> Result<LoadedProjectFile, ProjectFileIoError> {
    let relative = RemoteRelativePathBuf::new(file.relative_path.clone())
        .map_err(|error| file_remote_error(Path::new(&file.relative_path), error.to_string()))?;
    let relative_path = pathbuf_from_remote(&relative);
    let canonical_path = remote_document_path(project, &relative);
    let fingerprint = disk_fingerprint(file.fingerprint);
    if file.bytes.contains(&0) {
        return Err(ProjectFileIoError::BinaryContent {
            path: relative_path,
        });
    }
    let text = String::from_utf8(file.bytes).map_err(|_| ProjectFileIoError::InvalidUtf8 {
        path: relative_path.clone(),
    })?;
    Ok(LoadedProjectFile {
        canonical_path,
        relative_path,
        text,
        fingerprint,
    })
}

fn remote_entry_mutation(mutation: RemoteEntryMutation) -> ProjectEntryMutation {
    ProjectEntryMutation {
        relative_path: pathbuf_from_wire(&mutation.relative_path),
        kind: project_entry_kind(mutation.kind),
    }
}

fn project_entry_kind(kind: RemoteFileKind) -> ProjectTreeEntryKind {
    match kind {
        RemoteFileKind::Directory => ProjectTreeEntryKind::Directory,
        RemoteFileKind::File => ProjectTreeEntryKind::File,
        RemoteFileKind::SymlinkFile => ProjectTreeEntryKind::SymlinkFile,
        RemoteFileKind::SymlinkDirectory => ProjectTreeEntryKind::SymlinkDirectory,
    }
}

fn remote_save_outcome(outcome: RemoteSaveResult) -> SaveProjectFileOutcome {
    match outcome {
        RemoteSaveResult::Saved(fingerprint) => {
            SaveProjectFileOutcome::Saved(disk_fingerprint(fingerprint))
        }
        RemoteSaveResult::Conflict(RemoteFileState::Missing) => {
            SaveProjectFileOutcome::Conflict(CurrentDiskState::Missing)
        }
        RemoteSaveResult::Conflict(RemoteFileState::Present(fingerprint)) => {
            SaveProjectFileOutcome::Conflict(CurrentDiskState::Present(disk_fingerprint(
                fingerprint,
            )))
        }
    }
}

fn remote_fingerprint(fingerprint: &DiskFingerprint) -> RemoteFileFingerprint {
    let modified_seconds = fingerprint.modified.and_then(|modified| {
        modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u32::try_from(duration.as_secs()).ok())
    });
    RemoteFileFingerprint {
        byte_len: fingerprint.byte_len,
        modified_seconds,
        content_hash: fingerprint.content_hash,
    }
}

fn disk_fingerprint(fingerprint: RemoteFileFingerprint) -> DiskFingerprint {
    DiskFingerprint {
        exists: true,
        byte_len: fingerprint.byte_len,
        modified: fingerprint
            .modified_seconds
            .map(|seconds| UNIX_EPOCH + Duration::from_secs(u64::from(seconds))),
        content_hash: fingerprint.content_hash,
    }
}

fn file_remote_error(path: &Path, message: String) -> ProjectFileIoError {
    ProjectFileIoError::Remote {
        path: path.to_path_buf(),
        message,
    }
}

fn tree_remote_error(path: &Path, message: String) -> ProjectTreeFsError {
    ProjectTreeFsError::Remote {
        path: path.to_path_buf(),
        message,
    }
}

fn entry_remote_error(path: &Path, message: String) -> ProjectEntryFsError {
    ProjectEntryFsError::Remote {
        path: path.to_path_buf(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remote_relative_paths_use_posix_components() {
        assert_eq!(
            remote_relative(Path::new("src/main.rs")).unwrap().as_str(),
            "src/main.rs"
        );
        assert!(remote_relative(Path::new("../secret")).is_err());
    }

    #[test]
    fn searchable_files_respect_gitignore_and_hidden_setting() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::create_dir_all(temp.path().join("target")).unwrap();
        fs::create_dir_all(temp.path().join(".config")).unwrap();
        fs::write(
            temp.path().join(".gitignore"),
            "target/\n*.log\n!keep.log\n",
        )
        .unwrap();
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(temp.path().join("target/output.bin"), "ignored").unwrap();
        fs::write(temp.path().join("debug.log"), "ignored").unwrap();
        fs::write(temp.path().join("keep.log"), "included").unwrap();
        fs::write(
            temp.path().join(".config/settings.toml"),
            "theme = 'dark'\n",
        )
        .unwrap();
        let services = ProjectServices::local_for_test(temp.path());

        let visible = services.searchable_files(false).unwrap();
        assert_eq!(
            visible,
            vec![PathBuf::from("keep.log"), PathBuf::from("src/main.rs")]
        );

        let with_hidden = services.searchable_files(true).unwrap();
        assert!(with_hidden.contains(&PathBuf::from(".config/settings.toml")));
        assert!(with_hidden.contains(&PathBuf::from(".gitignore")));
        assert!(!with_hidden.contains(&PathBuf::from("target/output.bin")));
        assert!(!with_hidden.contains(&PathBuf::from("debug.log")));
    }

    #[test]
    fn remote_fingerprint_round_trips() {
        let remote = RemoteFileFingerprint {
            byte_len: 42,
            modified_seconds: Some(123),
            content_hash: 99,
        };
        assert_eq!(
            remote_fingerprint(&disk_fingerprint(remote.clone())),
            remote
        );
    }

    struct RecoveringProjectHost {
        registrations: AtomicU64,
        reads: AtomicU64,
        closes: AtomicU64,
    }

    impl ProjectHostTransport for RecoveringProjectHost {
        fn request(&self, request: Request) -> Result<Response, ClientCoreError> {
            let Request::Project(request) = request else {
                panic!("unexpected non-project request");
            };
            match request {
                ProjectRequest::Register { root, .. } => {
                    let registration_epoch = self.registrations.fetch_add(1, Ordering::Relaxed) + 1;
                    Ok(Response::Project(ProjectResponse::Registered {
                        canonical_root: root,
                        registration_epoch,
                        watch_error: None,
                        null_device: "/dev/null".to_string(),
                    }))
                }
                ProjectRequest::ReadFile { relative_path, .. } => {
                    let attempt = self.reads.fetch_add(1, Ordering::Relaxed) + 1;
                    if attempt <= 2 {
                        return Err(ClientCoreError::Protocol(
                            yttt_protocol::ProtocolFailure::new(
                                FailureCode::NotFound,
                                "project is not registered with Host: project",
                                false,
                            ),
                        ));
                    }
                    Ok(Response::Project(ProjectResponse::File(
                        ProjectFileContent {
                            canonical_path: relative_path.clone(),
                            relative_path,
                            text: "recovered".to_string(),
                            fingerprint: ProjectFileFingerprint {
                                exists: true,
                                byte_len: 9,
                                modified_nanos: Some(1_000_000_000),
                                content_hash: 1,
                            },
                        },
                    )))
                }
                ProjectRequest::Close { .. } => {
                    self.closes.fetch_add(1, Ordering::Relaxed);
                    Ok(Response::Project(ProjectResponse::Closed))
                }
                request => panic!("unexpected project request: {request:?}"),
            }
        }
    }

    #[test]
    fn host_project_service_reregisters_after_host_state_loss() {
        let transport = Arc::new(RecoveringProjectHost {
            registrations: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            closes: AtomicU64::new(0),
        });
        let services = ProjectServices::host_with_transport(
            transport.clone(),
            ProjectId::new("project"),
            PathBuf::from("/project"),
        )
        .unwrap();

        let loaded = services.read_file(Path::new("notes.txt")).unwrap();

        assert_eq!(loaded.text, "recovered");
        assert_eq!(services.host_registration_epoch(), Some(2));
        assert_eq!(transport.registrations.load(Ordering::Relaxed), 2);
        assert_eq!(transport.reads.load(Ordering::Relaxed), 3);
    }

    #[test]
    fn host_project_registration_closes_only_on_explicit_request() {
        let transport = Arc::new(RecoveringProjectHost {
            registrations: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            closes: AtomicU64::new(0),
        });
        let services = ProjectServices::host_with_transport(
            transport.clone(),
            ProjectId::new("project"),
            PathBuf::from("/project"),
        )
        .unwrap();

        drop(services);
        assert_eq!(transport.closes.load(Ordering::Relaxed), 0);

        let services = ProjectServices::host_with_transport(
            transport.clone(),
            ProjectId::new("project"),
            PathBuf::from("/project"),
        )
        .unwrap();
        services.close_host_registration().unwrap();
        assert_eq!(transport.closes.load(Ordering::Relaxed), 1);
    }

    struct RecoveringSshProjectHost {
        registrations: AtomicU64,
        reads: AtomicU64,
        closes: AtomicU64,
    }

    impl ProjectHostTransport for RecoveringSshProjectHost {
        fn request(&self, request: Request) -> Result<Response, ClientCoreError> {
            match request {
                Request::Project(ProjectRequest::RegisterSsh {
                    project_id,
                    connection_id,
                    root,
                }) => {
                    assert_eq!(project_id, ProjectId::new("project"));
                    assert_eq!(connection_id, "connection");
                    assert_eq!(root, "/remote");
                    let registration_epoch = self.registrations.fetch_add(1, Ordering::Relaxed) + 1;
                    Ok(Response::Project(ProjectResponse::Registered {
                        canonical_root: PlatformPath::Unix(root.into_bytes()),
                        registration_epoch,
                        watch_error: None,
                        null_device: "/dev/null".to_string(),
                    }))
                }
                Request::RemoteFile(RemoteFileRequest::Read {
                    project_id,
                    relative_path,
                    maximum_bytes,
                }) => {
                    assert_eq!(project_id, ProjectId::new("project"));
                    assert_eq!(relative_path, "notes.txt");
                    assert_eq!(maximum_bytes, MAX_PROJECT_FILE_BYTES);
                    let attempt = self.reads.fetch_add(1, Ordering::Relaxed) + 1;
                    if attempt <= 2 {
                        return Err(ClientCoreError::Protocol(
                            yttt_protocol::ProtocolFailure::new(
                                FailureCode::NotFound,
                                "project is not registered with Host: project",
                                false,
                            ),
                        ));
                    }
                    Ok(Response::RemoteFile(RemoteFileResponse::File(
                        RemoteFileContent {
                            relative_path,
                            bytes: b"recovered".to_vec(),
                            fingerprint: RemoteFileFingerprint {
                                byte_len: 9,
                                modified_seconds: Some(1),
                                content_hash: 1,
                            },
                        },
                    )))
                }
                Request::Project(ProjectRequest::Close {
                    project_id,
                    registration_epoch,
                }) => {
                    assert_eq!(project_id, ProjectId::new("project"));
                    assert_eq!(
                        registration_epoch,
                        self.registrations.load(Ordering::Relaxed)
                    );
                    self.closes.fetch_add(1, Ordering::Relaxed);
                    Ok(Response::Project(ProjectResponse::Closed))
                }
                request => panic!("unexpected request: {request:?}"),
            }
        }
    }

    #[test]
    fn ssh_project_service_reregisters_and_closes_through_host() {
        let transport = Arc::new(RecoveringSshProjectHost {
            registrations: AtomicU64::new(0),
            reads: AtomicU64::new(0),
            closes: AtomicU64::new(0),
        });
        let services = ProjectServices::ssh_with_transport(
            transport.clone(),
            ProjectId::new("project"),
            ConnectionId::new("connection"),
            RemotePathBuf::new("/remote").unwrap(),
        )
        .unwrap();

        let loaded = services.read_file(Path::new("notes.txt")).unwrap();

        assert_eq!(loaded.text, "recovered");
        assert_eq!(services.host_registration_epoch(), Some(2));
        assert_eq!(transport.registrations.load(Ordering::Relaxed), 2);
        assert_eq!(transport.reads.load(Ordering::Relaxed), 3);
        services.close_host_registration().unwrap();
        assert_eq!(transport.closes.load(Ordering::Relaxed), 1);
    }
}
