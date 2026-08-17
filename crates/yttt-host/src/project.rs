use notify::Watcher as _;
use std::{
    collections::HashMap,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, UNIX_EPOCH},
};
use tokio::sync::broadcast;

use yttt_core::model::{ids::ProjectId, project::RemotePathBuf};
use yttt_project_core::{
    file::{
        CurrentDiskState, DiskFingerprint, SaveMode, SaveProjectFileOutcome, read_project_file,
        save_project_file,
    },
    tree::{
        ProjectEntryMutation as CoreEntryMutation, ProjectEntryPasteMode as CorePasteMode,
        ProjectTreeEntryKind as CoreEntryKind, create_project_entry, delete_project_entry,
        paste_project_entry, rename_project_entry, scan_project_directory,
    },
};
use yttt_protocol::{
    HostPath, PathSegment, ProjectRelativePath,
    project::{
        ContentRevision, ProjectChange, ProjectDirectory, ProjectEntry, ProjectEntryKind,
        ProjectEntryMutation, ProjectFileContent, ProjectFileFingerprint, ProjectFileState,
        ProjectGitOutput, ProjectPasteMode, ProjectRequest, ProjectResponse, ProjectSaveMode,
        ProjectSaveResult,
    },
};

const PROJECT_EVENT_CAPACITY: usize = 256;

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct HostProjectError {
    pub code: yttt_protocol::FailureCode,
    message: String,
}

impl HostProjectError {
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            code: yttt_protocol::FailureCode::NotFound,
            message: message.into(),
        }
    }

    pub fn with_code(code: yttt_protocol::FailureCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }
}

impl From<yttt_project_core::file::ProjectFileIoError> for HostProjectError {
    fn from(error: yttt_project_core::file::ProjectFileIoError) -> Self {
        let code = match error {
            yttt_project_core::file::ProjectFileIoError::FileTooLarge { .. } => {
                yttt_protocol::FailureCode::ResourceLimit
            }
            yttt_project_core::file::ProjectFileIoError::PathOutsideProject { .. } => {
                yttt_protocol::FailureCode::PermissionDenied
            }
            yttt_project_core::file::ProjectFileIoError::NotAFile { .. }
            | yttt_project_core::file::ProjectFileIoError::BinaryContent { .. }
            | yttt_project_core::file::ProjectFileIoError::InvalidUtf8 { .. } => {
                yttt_protocol::FailureCode::InvalidRequest
            }
            yttt_project_core::file::ProjectFileIoError::Io { .. }
            | yttt_project_core::file::ProjectFileIoError::Remote { .. } => {
                yttt_protocol::FailureCode::Internal
            }
        };
        Self {
            code,
            message: error.to_string(),
        }
    }
}

impl From<String> for HostProjectError {
    fn from(message: String) -> Self {
        Self {
            code: yttt_protocol::FailureCode::Internal,
            message,
        }
    }
}

enum RegisteredProjectRoot {
    Local(PathBuf),
    Ssh(RegisteredSshProject),
}

#[derive(Clone)]
pub struct RegisteredSshProject {
    pub connection_id: String,
    pub root: RemotePathBuf,
}

struct RegisteredProject {
    root: RegisteredProjectRoot,
    registration_epoch: u64,
    _watcher: Option<notify::RecommendedWatcher>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AssignedRevision {
    revision_number: u64,
    content_sha256: [u8; 32],
}

pub struct HostProjectRuntime {
    roots: RwLock<HashMap<ProjectId, RegisteredProject>>,
    next_registration_epoch: AtomicU64,
    events: broadcast::Sender<ProjectChange>,
    workspace_epoch: u64,
    next_revision: AtomicU64,
    revisions: RwLock<HashMap<(ProjectId, String), AssignedRevision>>,
}

impl HostProjectRuntime {
    #[cfg(test)]
    pub fn new() -> Self {
        Self::new_with_epoch(1)
    }

    pub fn new_with_epoch(workspace_epoch: u64) -> Self {
        let (events, _) = broadcast::channel(PROJECT_EVENT_CAPACITY);
        Self {
            roots: RwLock::new(HashMap::new()),
            next_registration_epoch: AtomicU64::new(0),
            events,
            workspace_epoch,
            next_revision: AtomicU64::new(1),
            revisions: RwLock::new(HashMap::new()),
        }
    }

    #[cfg(test)]
    pub fn workspace_epoch(&self) -> u64 {
        self.workspace_epoch
    }

    pub fn bind_revision(
        &self,
        project_id: &ProjectId,
        relative_path: &str,
        content_sha256: [u8; 32],
    ) -> ContentRevision {
        let key = (project_id.clone(), relative_path.to_string());
        let mut revisions = self
            .revisions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(current) = revisions.get(&key)
            && current.content_sha256 == content_sha256
        {
            return ContentRevision {
                workspace_epoch: self.workspace_epoch,
                revision_number: current.revision_number,
                content_sha256,
            };
        }
        let revision_number = self.next_revision.fetch_add(1, Ordering::Relaxed);
        revisions.insert(
            key,
            AssignedRevision {
                revision_number,
                content_sha256,
            },
        );
        ContentRevision {
            workspace_epoch: self.workspace_epoch,
            revision_number,
            content_sha256,
        }
    }

    pub fn bump_revision(
        &self,
        project_id: &ProjectId,
        relative_path: &str,
        content_sha256: [u8; 32],
    ) -> ContentRevision {
        let revision_number = self.next_revision.fetch_add(1, Ordering::Relaxed);
        self.revisions
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .insert(
                (project_id.clone(), relative_path.to_string()),
                AssignedRevision {
                    revision_number,
                    content_sha256,
                },
            );
        ContentRevision {
            workspace_epoch: self.workspace_epoch,
            revision_number,
            content_sha256,
        }
    }

    pub fn revision_matches(
        &self,
        project_id: &ProjectId,
        relative_path: &str,
        expected: &ContentRevision,
    ) -> bool {
        if expected.workspace_epoch != self.workspace_epoch {
            return false;
        }
        self.revisions
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(&(project_id.clone(), relative_path.to_string()))
            .is_some_and(|current| {
                current.revision_number == expected.revision_number
                    && current.content_sha256 == expected.content_sha256
            })
    }

    pub fn subscribe(&self) -> broadcast::Receiver<ProjectChange> {
        self.events.subscribe()
    }

    pub fn projects(&self) -> Vec<ProjectId> {
        let mut projects = self
            .roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        projects.sort_by(|left, right| left.as_str().cmp(right.as_str()));
        projects
    }

    pub fn resync_changes(&self) -> Vec<ProjectChange> {
        self.roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .map(|(project_id, project)| ProjectChange {
                project_id: project_id.clone(),
                registration_epoch: project.registration_epoch,
                relative_paths: Vec::new(),
                refresh_status: true,
                refresh_tree: true,
                refresh_all: true,
            })
            .collect()
    }

    pub fn handle(&self, request: ProjectRequest) -> Result<ProjectResponse, HostProjectError> {
        match request {
            ProjectRequest::Register { project_id, root } => {
                let root = host_os_path(root)?;
                let root = fs::canonicalize(&root).map_err(|error| {
                    format!("failed to open project {}: {error}", root.display())
                })?;
                if !root.is_dir() {
                    return Err(
                        format!("project root is not a directory: {}", root.display()).into(),
                    );
                }
                let registration_epoch = self
                    .next_registration_epoch
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                let (watcher, watch_error) = match watch_project(
                    self.events.clone(),
                    project_id.clone(),
                    registration_epoch,
                    root.clone(),
                ) {
                    Ok(watcher) => (Some(watcher), None),
                    Err(error) => (None, Some(error)),
                };
                self.roots
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(
                        project_id,
                        RegisteredProject {
                            root: RegisteredProjectRoot::Local(root.clone()),
                            registration_epoch,
                            _watcher: watcher,
                        },
                    );
                Ok(ProjectResponse::Registered {
                    registration_epoch,
                    watch_error,
                    null_device: null_device_path().to_string(),
                })
            }
            ProjectRequest::RegisterSsh {
                project_id,
                connection_id,
                root,
            } => {
                let root = remote_root(root)?;
                let registration_epoch = self
                    .next_registration_epoch
                    .fetch_add(1, Ordering::Relaxed)
                    .saturating_add(1);
                self.roots
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(
                        project_id,
                        RegisteredProject {
                            root: RegisteredProjectRoot::Ssh(RegisteredSshProject {
                                connection_id,
                                root: root.clone(),
                            }),
                            registration_epoch,
                            _watcher: None,
                        },
                    );
                Ok(ProjectResponse::Registered {
                    registration_epoch,
                    watch_error: None,
                    null_device: "/dev/null".to_string(),
                })
            }
            ProjectRequest::Close {
                project_id,
                registration_epoch,
            } => {
                let mut roots = self
                    .roots
                    .write()
                    .unwrap_or_else(|poisoned| poisoned.into_inner());
                if roots
                    .get(&project_id)
                    .is_some_and(|project| project.registration_epoch == registration_epoch)
                {
                    roots.remove(&project_id);
                }
                Ok(ProjectResponse::Closed)
            }
            ProjectRequest::ScanDirectory {
                project_id,
                relative_directory,
                show_hidden,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_directory = relative_os_path(relative_directory);
                let snapshot = scan_project_directory(&root, &relative_directory, show_hidden)
                    .map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Directory(ProjectDirectory {
                    relative_directory: path_to_relative(&snapshot.relative_directory)?,
                    entries: snapshot
                        .entries
                        .into_iter()
                        .map(|entry| {
                            Ok(ProjectEntry {
                                name: os_string_to_segment(entry.name)?,
                                relative_path: path_to_relative(&entry.relative_path)?,
                                kind: entry_kind(entry.kind),
                            })
                        })
                        .collect::<Result<Vec<_>, String>>()?,
                }))
            }
            ProjectRequest::ReadFile {
                project_id,
                relative_path,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_path = relative_os_path(relative_path);
                let path_key = relative_path.to_string_lossy().into_owned();
                let loaded = read_project_file(&root, &relative_path)?;
                Ok(ProjectResponse::File(ProjectFileContent {
                    relative_path: path_to_relative(&loaded.relative_path)?,
                    text: loaded.text,
                    fingerprint: self.fingerprint_to_wire(
                        &project_id,
                        &path_key,
                        &loaded.fingerprint,
                    ),
                }))
            }
            ProjectRequest::SaveFile {
                project_id,
                relative_path,
                text,
                mode,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_path = relative_os_path(relative_path);
                let path_key = relative_path.to_string_lossy().into_owned();
                if let ProjectSaveMode::Check(base) = &mode
                    && !self.revision_matches(&project_id, &path_key, &base.revision)
                {
                    let current = match read_project_file(&root, &relative_path) {
                        Ok(loaded) => ProjectFileState::Present(self.fingerprint_to_wire(
                            &project_id,
                            &loaded.relative_path.to_string_lossy(),
                            &loaded.fingerprint,
                        )),
                        Err(yttt_project_core::file::ProjectFileIoError::Io { source, .. })
                            if source.kind() == std::io::ErrorKind::NotFound =>
                        {
                            ProjectFileState::Missing
                        }
                        Err(error) => return Err(error.into()),
                    };
                    return Ok(ProjectResponse::Save(ProjectSaveResult::Conflict(current)));
                }
                let expected;
                let mode = match mode {
                    ProjectSaveMode::Check(fingerprint) => {
                        expected = fingerprint_from_wire(fingerprint)?;
                        SaveMode::Check(&expected)
                    }
                    ProjectSaveMode::Force => SaveMode::Force,
                };
                let result = save_project_file(&root, &relative_path, &text, mode)?;
                Ok(ProjectResponse::Save(match result {
                    SaveProjectFileOutcome::Saved(fingerprint) => {
                        let revision =
                            self.bump_revision(&project_id, &path_key, fingerprint.content_sha256);
                        ProjectSaveResult::Saved(fingerprint_to_wire_with_revision(
                            &fingerprint,
                            revision,
                        ))
                    }
                    SaveProjectFileOutcome::Conflict(state) => ProjectSaveResult::Conflict(
                        self.file_state_to_wire(&project_id, &path_key, state),
                    ),
                }))
            }
            ProjectRequest::CreateEntry {
                project_id,
                relative_parent,
                input,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_parent = relative_os_path(relative_parent);
                let mutation = create_project_entry(&root, &relative_parent, &input)
                    .map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Mutation(mutation_to_wire(mutation)))
            }
            ProjectRequest::RenameEntry {
                project_id,
                relative_path,
                new_name,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_path = relative_os_path(relative_path);
                let mutation = rename_project_entry(&root, &relative_path, &new_name)
                    .map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Mutation(mutation_to_wire(mutation)))
            }
            ProjectRequest::DeleteEntry {
                project_id,
                relative_path,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_path = relative_os_path(relative_path);
                delete_project_entry(&root, &relative_path).map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Deleted)
            }
            ProjectRequest::PasteEntry {
                source_project_id,
                source_relative_path,
                destination_project_id,
                destination_relative_directory,
                mode,
            } => {
                let source_root = self.local_root(&source_project_id)?;
                let destination_root = self.local_root(&destination_project_id)?;
                let source_relative_path = relative_os_path(source_relative_path);
                let destination_relative_directory =
                    relative_os_path(destination_relative_directory);
                let mode = match mode {
                    ProjectPasteMode::Copy => CorePasteMode::Copy,
                    ProjectPasteMode::Cut => CorePasteMode::Cut,
                };
                let mutation = paste_project_entry(
                    &source_root,
                    &source_relative_path,
                    &destination_root,
                    &destination_relative_directory,
                    mode,
                )
                .map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Mutation(mutation_to_wire(mutation)))
            }
            ProjectRequest::Git {
                project_id,
                operation,
            } => {
                let root = self.local_root(&project_id)?;
                let cwd = match operation.work_tree() {
                    Some(work_tree) => work_tree.join_under(&root),
                    None => root,
                };
                let args = operation.argv(null_device_path()).map_err(|error| {
                    HostProjectError::with_code(
                        yttt_protocol::FailureCode::InvalidRequest,
                        error.to_string(),
                    )
                })?;
                if !yttt_protocol::git_argv_is_safe(&args) {
                    return Err(HostProjectError::with_code(
                        yttt_protocol::FailureCode::PermissionDenied,
                        "git arguments are not allowed",
                    ));
                }
                let mut command = git_command();
                command
                    .args(args)
                    .current_dir(cwd)
                    .stdin(Stdio::null())
                    .stderr(Stdio::piped());
                if operation.optional_locks() {
                    command.env("GIT_OPTIONAL_LOCKS", "0");
                }
                let output = command.output().map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Git(ProjectGitOutput {
                    success: output.status.success(),
                    exit_code: output.status.code(),
                    stdout: output.stdout,
                    stderr: output.stderr,
                }))
            }
        }
    }

    pub(crate) fn local_root(&self, project_id: &ProjectId) -> Result<PathBuf, HostProjectError> {
        self.roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(project_id)
            .and_then(|project| match &project.root {
                RegisteredProjectRoot::Local(root) => Some(root.clone()),
                RegisteredProjectRoot::Ssh(_) => None,
            })
            .ok_or_else(|| {
                HostProjectError::not_found(format!(
                    "project is not registered with Host: {project_id}"
                ))
            })
    }

    fn fingerprint_to_wire(
        &self,
        project_id: &ProjectId,
        relative_path: &str,
        fingerprint: &DiskFingerprint,
    ) -> ProjectFileFingerprint {
        let revision = self.bind_revision(project_id, relative_path, fingerprint.content_sha256);
        fingerprint_to_wire_with_revision(fingerprint, revision)
    }

    fn file_state_to_wire(
        &self,
        project_id: &ProjectId,
        relative_path: &str,
        state: CurrentDiskState,
    ) -> ProjectFileState {
        match state {
            CurrentDiskState::Missing => ProjectFileState::Missing,
            CurrentDiskState::Present(fingerprint) => ProjectFileState::Present(
                self.fingerprint_to_wire(project_id, relative_path, &fingerprint),
            ),
        }
    }

    pub fn ssh_project(
        &self,
        project_id: &ProjectId,
    ) -> Result<RegisteredSshProject, HostProjectError> {
        self.roots
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(project_id)
            .and_then(|project| match &project.root {
                RegisteredProjectRoot::Local(_) => None,
                RegisteredProjectRoot::Ssh(project) => Some(project.clone()),
            })
            .ok_or_else(|| {
                HostProjectError::not_found(format!(
                    "project is not registered with Host: {project_id}"
                ))
            })
    }
}

fn watch_project(
    events: broadcast::Sender<ProjectChange>,
    project_id: ProjectId,
    registration_epoch: u64,
    root: PathBuf,
) -> Result<notify::RecommendedWatcher, String> {
    let callback_root = root.clone();
    let callback_project_id = project_id.clone();
    let callback_events = events.clone();
    let mut watcher =
        notify::recommended_watcher(move |result: notify::Result<notify::Event>| match result {
            Ok(event) => {
                let rescan = event.need_rescan();
                let refresh_status = rescan
                    || matches!(
                        event.kind,
                        notify::EventKind::Any
                            | notify::EventKind::Create(_)
                            | notify::EventKind::Modify(_)
                            | notify::EventKind::Remove(_)
                    );
                if !refresh_status {
                    return;
                }
                let refresh_tree = rescan
                    || matches!(
                        event.kind,
                        notify::EventKind::Any
                            | notify::EventKind::Create(_)
                            | notify::EventKind::Modify(
                                notify::event::ModifyKind::Any
                                    | notify::event::ModifyKind::Name(_)
                                    | notify::event::ModifyKind::Other
                            )
                            | notify::EventKind::Remove(_)
                    );
                let mut refresh_all = rescan
                    || matches!(
                        event.kind,
                        notify::EventKind::Any
                            | notify::EventKind::Modify(
                                notify::event::ModifyKind::Any | notify::event::ModifyKind::Other
                            )
                    );
                let relative_paths = event
                    .paths
                    .into_iter()
                    .filter_map(|path| {
                        path.strip_prefix(&callback_root)
                            .ok()
                            .and_then(|relative| path_to_relative(relative).ok())
                    })
                    .collect::<Vec<_>>();
                if refresh_tree && relative_paths.is_empty() {
                    refresh_all = true;
                }
                let _ = callback_events.send(ProjectChange {
                    project_id: callback_project_id.clone(),
                    registration_epoch,
                    relative_paths,
                    refresh_status,
                    refresh_tree,
                    refresh_all,
                });
            }
            Err(_) => {
                let _ = callback_events.send(ProjectChange {
                    project_id: callback_project_id.clone(),
                    registration_epoch,
                    relative_paths: Vec::new(),
                    refresh_status: true,
                    refresh_tree: true,
                    refresh_all: true,
                });
            }
        })
        .map_err(|error| format!("failed to create project file watcher: {error}"))?;
    watcher
        .watch(&root, notify::RecursiveMode::Recursive)
        .map_err(|error| {
            format!(
                "failed to watch project files at {}: {error}",
                root.display()
            )
        })?;
    Ok(watcher)
}

fn fingerprint_to_wire_with_revision(
    fingerprint: &DiskFingerprint,
    revision: ContentRevision,
) -> ProjectFileFingerprint {
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
        revision,
    }
}

fn fingerprint_from_wire(fingerprint: ProjectFileFingerprint) -> Result<DiskFingerprint, String> {
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
        workspace_epoch: fingerprint.revision.workspace_epoch,
        revision_number: fingerprint.revision.revision_number,
        content_sha256: fingerprint.revision.content_sha256,
    })
}

fn mutation_to_wire(mutation: CoreEntryMutation) -> ProjectEntryMutation {
    ProjectEntryMutation {
        relative_path: path_to_relative(&mutation.relative_path)
            .expect("project mutations stay inside the project root"),
        kind: entry_kind(mutation.kind),
    }
}

fn entry_kind(kind: CoreEntryKind) -> ProjectEntryKind {
    match kind {
        CoreEntryKind::Directory => ProjectEntryKind::Directory,
        CoreEntryKind::File => ProjectEntryKind::File,
        CoreEntryKind::SymlinkFile => ProjectEntryKind::SymlinkFile,
        CoreEntryKind::SymlinkDirectory => ProjectEntryKind::SymlinkDirectory,
    }
}

fn host_os_path(path: HostPath) -> Result<PathBuf, String> {
    path.to_path().map_err(|error| error.to_string())
}

fn relative_os_path(path: ProjectRelativePath) -> PathBuf {
    path.join_under(Path::new(""))
}

fn path_to_relative(path: &Path) -> Result<ProjectRelativePath, String> {
    ProjectRelativePath::from_path(path).map_err(|error| error.to_string())
}

fn remote_root(path: ProjectRelativePath) -> Result<RemotePathBuf, String> {
    let relative = path.to_utf8().map_err(|error| error.to_string())?;
    let absolute = if relative.is_empty() {
        "/".to_string()
    } else {
        format!("/{relative}")
    };
    RemotePathBuf::new(absolute).map_err(|error| error.to_string())
}

fn os_string_to_segment(value: OsString) -> Result<PathSegment, String> {
    PathSegment::from_os_str(&value).map_err(|error| error.to_string())
}

#[cfg(windows)]
fn git_command() -> Command {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let mut command = Command::new("git");
    command.creation_flags(CREATE_NO_WINDOW);
    command
}

#[cfg(not(windows))]
fn git_command() -> Command {
    Command::new("git")
}

#[cfg(windows)]
fn null_device_path() -> &'static str {
    "NUL"
}

#[cfg(not(windows))]
fn null_device_path() -> &'static str {
    "/dev/null"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ssh_registration_is_epoch_guarded_and_host_owned() {
        let runtime = HostProjectRuntime::new();
        let project_id = ProjectId::new("remote");
        let ProjectResponse::Registered {
            registration_epoch: first_epoch,
            ..
        } = runtime
            .handle(ProjectRequest::RegisterSsh {
                project_id: project_id.clone(),
                connection_id: "first".to_string(),
                root: ProjectRelativePath::from_utf8("first").unwrap(),
            })
            .unwrap()
        else {
            panic!("unexpected first SSH registration response");
        };
        let ProjectResponse::Registered {
            registration_epoch: second_epoch,
            ..
        } = runtime
            .handle(ProjectRequest::RegisterSsh {
                project_id: project_id.clone(),
                connection_id: "second".to_string(),
                root: ProjectRelativePath::from_utf8("second").unwrap(),
            })
            .unwrap()
        else {
            panic!("unexpected second SSH registration response");
        };
        assert!(second_epoch > first_epoch);

        runtime
            .handle(ProjectRequest::Close {
                project_id: project_id.clone(),
                registration_epoch: first_epoch,
            })
            .unwrap();
        let registered = runtime.ssh_project(&project_id).unwrap();
        assert_eq!(registered.connection_id, "second");
        assert_eq!(registered.root.as_str(), "/second");

        runtime
            .handle(ProjectRequest::Close {
                project_id: project_id.clone(),
                registration_epoch: second_epoch,
            })
            .unwrap();
        let error = runtime.ssh_project(&project_id).err().unwrap();
        assert_eq!(error.code, yttt_protocol::FailureCode::NotFound);
        assert_eq!(
            error.to_string(),
            "project is not registered with Host: remote"
        );
    }

    #[test]
    fn stale_workspace_epoch_does_not_overwrite_and_returns_conflict() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("notes.txt"), "v1").unwrap();
        let first = HostProjectRuntime::new_with_epoch(1);
        let project_id = ProjectId::new("notes");
        first
            .handle(ProjectRequest::Register {
                project_id: project_id.clone(),
                root: HostPath::from_path(root.path()).unwrap(),
            })
            .unwrap();
        let ProjectResponse::File(file) = first
            .handle(ProjectRequest::ReadFile {
                project_id: project_id.clone(),
                relative_path: ProjectRelativePath::from_utf8("notes.txt").unwrap(),
            })
            .unwrap()
        else {
            panic!("expected file response");
        };
        assert_eq!(
            file.fingerprint.revision.workspace_epoch,
            first.workspace_epoch()
        );
        assert_eq!(file.fingerprint.revision.revision_number, 1);

        let second = HostProjectRuntime::new_with_epoch(2);
        second
            .handle(ProjectRequest::Register {
                project_id: project_id.clone(),
                root: HostPath::from_path(root.path()).unwrap(),
            })
            .unwrap();
        let ProjectResponse::Save(ProjectSaveResult::Conflict(ProjectFileState::Present(current))) =
            second
                .handle(ProjectRequest::SaveFile {
                    project_id: project_id.clone(),
                    relative_path: ProjectRelativePath::from_utf8("notes.txt").unwrap(),
                    text: "stale".to_string(),
                    mode: ProjectSaveMode::Check(file.fingerprint),
                })
                .unwrap()
        else {
            panic!("expected stale epoch conflict");
        };
        assert_eq!(current.revision.workspace_epoch, 2);
        assert_eq!(
            fs::read_to_string(root.path().join("notes.txt")).unwrap(),
            "v1"
        );
    }

    #[test]
    fn git_switch_rejects_config_injection_ref_names() {
        let root = tempfile::tempdir().unwrap();
        let runtime = HostProjectRuntime::new();
        let project_id = ProjectId::new("git");
        runtime
            .handle(ProjectRequest::Register {
                project_id: project_id.clone(),
                root: HostPath::from_path(root.path()).unwrap(),
            })
            .unwrap();
        let error = runtime
            .handle(ProjectRequest::Git {
                project_id,
                operation: yttt_protocol::ProjectGitOperation::Switch {
                    name: "-c".to_string(),
                    track_remote: false,
                },
            })
            .unwrap_err();
        assert_eq!(error.code, yttt_protocol::FailureCode::InvalidRequest);
    }
}
