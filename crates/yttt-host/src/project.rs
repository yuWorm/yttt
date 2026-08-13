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
use yttt_protocol::project::{
    PlatformArgument, PlatformPath, ProjectChange, ProjectDirectory, ProjectEntry,
    ProjectEntryKind, ProjectEntryMutation, ProjectFileContent, ProjectFileFingerprint,
    ProjectFileState, ProjectGitOutput, ProjectPasteMode, ProjectRequest, ProjectResponse,
    ProjectSaveMode, ProjectSaveResult,
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

pub struct HostProjectRuntime {
    roots: RwLock<HashMap<ProjectId, RegisteredProject>>,
    next_registration_epoch: AtomicU64,
    events: broadcast::Sender<ProjectChange>,
}

impl HostProjectRuntime {
    pub fn new() -> Self {
        let (events, _) = broadcast::channel(PROJECT_EVENT_CAPACITY);
        Self {
            roots: RwLock::new(HashMap::new()),
            next_registration_epoch: AtomicU64::new(0),
            events,
        }
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
                let root = platform_path(root)?;
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
                    canonical_root: path_to_platform(&root),
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
                let root = RemotePathBuf::new(root).map_err(|error| error.to_string())?;
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
                    canonical_root: PlatformPath::Unix(root.as_str().as_bytes().to_vec()),
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
                let relative_directory = platform_path(relative_directory)?;
                let snapshot = scan_project_directory(&root, &relative_directory, show_hidden)
                    .map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Directory(ProjectDirectory {
                    relative_directory: path_to_platform(&snapshot.relative_directory),
                    entries: snapshot
                        .entries
                        .into_iter()
                        .map(|entry| ProjectEntry {
                            name: os_string_to_platform(entry.name),
                            relative_path: path_to_platform(&entry.relative_path),
                            kind: entry_kind(entry.kind),
                        })
                        .collect(),
                }))
            }
            ProjectRequest::ReadFile {
                project_id,
                relative_path,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_path = platform_path(relative_path)?;
                let loaded =
                    read_project_file(&root, &relative_path).map_err(|error| error.to_string())?;
                Ok(ProjectResponse::File(ProjectFileContent {
                    canonical_path: path_to_platform(&loaded.canonical_path),
                    relative_path: path_to_platform(&loaded.relative_path),
                    text: loaded.text,
                    fingerprint: fingerprint_to_wire(&loaded.fingerprint),
                }))
            }
            ProjectRequest::SaveFile {
                project_id,
                relative_path,
                text,
                mode,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_path = platform_path(relative_path)?;
                let expected;
                let mode = match mode {
                    ProjectSaveMode::Check(fingerprint) => {
                        expected = fingerprint_from_wire(fingerprint)?;
                        SaveMode::Check(&expected)
                    }
                    ProjectSaveMode::Force => SaveMode::Force,
                };
                let result = save_project_file(&root, &relative_path, &text, mode)
                    .map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Save(match result {
                    SaveProjectFileOutcome::Saved(fingerprint) => {
                        ProjectSaveResult::Saved(fingerprint_to_wire(&fingerprint))
                    }
                    SaveProjectFileOutcome::Conflict(state) => {
                        ProjectSaveResult::Conflict(file_state_to_wire(state))
                    }
                }))
            }
            ProjectRequest::CreateEntry {
                project_id,
                relative_parent,
                input,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_parent = platform_path(relative_parent)?;
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
                let relative_path = platform_path(relative_path)?;
                let mutation = rename_project_entry(&root, &relative_path, &new_name)
                    .map_err(|error| error.to_string())?;
                Ok(ProjectResponse::Mutation(mutation_to_wire(mutation)))
            }
            ProjectRequest::DeleteEntry {
                project_id,
                relative_path,
            } => {
                let root = self.local_root(&project_id)?;
                let relative_path = platform_path(relative_path)?;
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
                let source_relative_path = platform_path(source_relative_path)?;
                let destination_relative_directory = platform_path(destination_relative_directory)?;
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
                args,
                optional_locks,
            } => {
                let root = self.local_root(&project_id)?;
                let args = args
                    .into_iter()
                    .map(platform_argument)
                    .collect::<Result<Vec<_>, _>>()?;
                let mut command = git_command();
                command
                    .args(args)
                    .current_dir(root)
                    .stdin(Stdio::null())
                    .stderr(Stdio::piped());
                if optional_locks {
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

    fn local_root(&self, project_id: &ProjectId) -> Result<PathBuf, HostProjectError> {
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
                    .filter_map(|path| path.strip_prefix(&callback_root).ok().map(path_to_platform))
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

fn fingerprint_to_wire(fingerprint: &DiskFingerprint) -> ProjectFileFingerprint {
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
    })
}

fn file_state_to_wire(state: CurrentDiskState) -> ProjectFileState {
    match state {
        CurrentDiskState::Missing => ProjectFileState::Missing,
        CurrentDiskState::Present(fingerprint) => {
            ProjectFileState::Present(fingerprint_to_wire(&fingerprint))
        }
    }
}

fn mutation_to_wire(mutation: CoreEntryMutation) -> ProjectEntryMutation {
    ProjectEntryMutation {
        relative_path: path_to_platform(&mutation.relative_path),
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

#[cfg(unix)]
fn platform_path(path: PlatformPath) -> Result<PathBuf, String> {
    use std::os::unix::ffi::OsStringExt as _;
    match path {
        PlatformPath::Unix(bytes) => Ok(PathBuf::from(OsString::from_vec(bytes))),
        PlatformPath::Windows(_) => Err("received a Windows path on a Unix Host".to_string()),
    }
}

#[cfg(windows)]
fn platform_path(path: PlatformPath) -> Result<PathBuf, String> {
    use std::os::windows::ffi::OsStringExt as _;
    match path {
        PlatformPath::Windows(wide) => Ok(PathBuf::from(OsString::from_wide(&wide))),
        PlatformPath::Unix(_) => Err("received a Unix path on a Windows Host".to_string()),
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_path(_path: PlatformPath) -> Result<PathBuf, String> {
    Err("project paths are unsupported on this platform".to_string())
}

#[cfg(unix)]
fn path_to_platform(path: &Path) -> PlatformPath {
    use std::os::unix::ffi::OsStrExt as _;
    PlatformPath::Unix(path.as_os_str().as_bytes().to_vec())
}

#[cfg(windows)]
fn path_to_platform(path: &Path) -> PlatformPath {
    use std::os::windows::ffi::OsStrExt as _;
    PlatformPath::Windows(path.as_os_str().encode_wide().collect())
}

#[cfg(not(any(unix, windows)))]
fn path_to_platform(path: &Path) -> PlatformPath {
    PlatformPath::Unix(path.to_string_lossy().as_bytes().to_vec())
}

#[cfg(unix)]
fn os_string_to_platform(value: OsString) -> PlatformArgument {
    use std::os::unix::ffi::OsStringExt as _;
    PlatformArgument::Unix(value.into_vec())
}

#[cfg(windows)]
fn os_string_to_platform(value: OsString) -> PlatformArgument {
    use std::os::windows::ffi::OsStrExt as _;
    PlatformArgument::Windows(value.encode_wide().collect())
}

#[cfg(not(any(unix, windows)))]
fn os_string_to_platform(value: OsString) -> PlatformArgument {
    PlatformArgument::Unix(value.to_string_lossy().as_bytes().to_vec())
}

#[cfg(unix)]
fn platform_argument(value: PlatformArgument) -> Result<OsString, String> {
    use std::os::unix::ffi::OsStringExt as _;
    match value {
        PlatformArgument::Unix(bytes) => Ok(OsString::from_vec(bytes)),
        PlatformArgument::Windows(_) => {
            Err("received a Windows argument on a Unix Host".to_string())
        }
    }
}

#[cfg(windows)]
fn platform_argument(value: PlatformArgument) -> Result<OsString, String> {
    use std::os::windows::ffi::OsStringExt as _;
    match value {
        PlatformArgument::Windows(wide) => Ok(OsString::from_wide(&wide)),
        PlatformArgument::Unix(_) => Err("received a Unix argument on a Windows Host".to_string()),
    }
}

#[cfg(not(any(unix, windows)))]
fn platform_argument(_value: PlatformArgument) -> Result<OsString, String> {
    Err("project arguments are unsupported on this platform".to_string())
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
                root: "/first".to_string(),
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
                root: "/second".to_string(),
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
}
