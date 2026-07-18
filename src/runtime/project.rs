use std::{
    ffi::{OsStr, OsString},
    fs,
    path::{Component, Path, PathBuf},
    sync::Arc,
    time::{Duration, UNIX_EPOCH},
};

use ignore::{
    Match, WalkBuilder,
    gitignore::{Gitignore, GitignoreBuilder},
};
use yttt_core::model::project::{RemotePathError, RemoteRelativePathBuf};
use yttt_ssh::{
    RemoteDirectorySnapshot, RemoteEntryKind, RemoteEntryMutation, RemoteFileState,
    RemoteFingerprint, RemoteLoadedFile, RemoteSaveOutcome, SftpError, SftpProject,
};

use super::git_status::{GitCommandOutput, ProjectGitExecutor, execute_local_git};

use crate::ui::{
    editor::{
        CurrentDiskState, DiskFingerprint, LoadedProjectFile, MAX_PROJECT_FILE_BYTES,
        ProjectFileIoError, SaveMode, SaveProjectFileOutcome, project_relative_path,
        read_project_file, save_project_file,
    },
    project_tree::{
        DirectorySnapshot, ProjectEntryFsError, ProjectEntryMutation, ProjectEntryPasteMode,
        ProjectTreeEntry, ProjectTreeEntryKind, ProjectTreeFsError, create_project_entry,
        delete_project_entry, paste_project_entry, rename_project_entry, scan_project_directory,
    },
};

#[derive(Clone)]
pub struct ProjectServices {
    backend: Arc<ProjectBackend>,
}

enum ProjectBackend {
    Local(LocalProjectServices),
    Ssh(SftpProject),
}

struct LocalProjectServices {
    root: PathBuf,
}

impl ProjectServices {
    pub fn local(root: impl Into<PathBuf>) -> Self {
        Self {
            backend: Arc::new(ProjectBackend::Local(LocalProjectServices {
                root: root.into(),
            })),
        }
    }

    pub fn ssh(project: SftpProject) -> Self {
        Self {
            backend: Arc::new(ProjectBackend::Ssh(project)),
        }
    }

    pub fn local_root(&self) -> Option<&Path> {
        match self.backend.as_ref() {
            ProjectBackend::Local(local) => Some(&local.root),
            ProjectBackend::Ssh(_) => None,
        }
    }

    pub fn document_path(&self, relative_path: &Path) -> Option<PathBuf> {
        match self.backend.as_ref() {
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
            ProjectBackend::Local(local) => project_relative_path(&local.root, document_path),
            ProjectBackend::Ssh(project) => {
                let root = PathBuf::from(project.root().as_str());
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
            ProjectBackend::Local(local) => {
                scan_project_directory(&local.root, relative_directory, show_hidden)
            }
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_directory)
                    .map_err(|message| tree_remote_error(relative_directory, message))?;
                project
                    .scan_directory(relative, show_hidden)
                    .map(remote_directory_snapshot)
                    .map_err(|error| map_tree_error(relative_directory, error))
            }
        }
    }

    pub fn searchable_files(&self, show_hidden: bool) -> Result<Vec<PathBuf>, String> {
        if let Some(paths) = searchable_git_files(self, show_hidden) {
            return Ok(paths);
        }

        match self.backend.as_ref() {
            ProjectBackend::Local(local) => searchable_local_files(&local.root, show_hidden),
            ProjectBackend::Ssh(_) => searchable_remote_files(self, show_hidden),
        }
    }

    pub fn read_file(&self, relative_path: &Path) -> Result<LoadedProjectFile, ProjectFileIoError> {
        match self.backend.as_ref() {
            ProjectBackend::Local(local) => read_project_file(&local.root, relative_path),
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_path)
                    .map_err(|message| file_remote_error(relative_path, message))?;
                let file = project
                    .read_file(relative, MAX_PROJECT_FILE_BYTES)
                    .map_err(|error| map_file_error(relative_path, error))?;
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
                project
                    .save_file(
                        relative,
                        text.as_bytes().to_vec(),
                        expected,
                        force,
                        MAX_PROJECT_FILE_BYTES,
                    )
                    .map(remote_save_outcome)
                    .map_err(|error| map_file_error(relative_path, error))
            }
        }
    }

    pub fn create_entry(
        &self,
        relative_directory: &Path,
        input: &str,
    ) -> Result<ProjectEntryMutation, ProjectEntryFsError> {
        match self.backend.as_ref() {
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
                project
                    .create_entry(relative_path, directory)
                    .map(remote_entry_mutation)
                    .map_err(|error| map_entry_error(relative_directory, error))
            }
        }
    }

    pub fn rename_entry(
        &self,
        relative_path: &Path,
        new_name: &str,
    ) -> Result<ProjectEntryMutation, ProjectEntryFsError> {
        match self.backend.as_ref() {
            ProjectBackend::Local(local) => {
                rename_project_entry(&local.root, relative_path, new_name)
            }
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_path).map_err(|_| {
                    ProjectEntryFsError::InvalidEntryName {
                        input: new_name.to_string(),
                    }
                })?;
                project
                    .rename_entry(relative, new_name.to_string())
                    .map(remote_entry_mutation)
                    .map_err(|error| map_entry_error(relative_path, error))
            }
        }
    }

    pub fn delete_entry(&self, relative_path: &Path) -> Result<(), ProjectEntryFsError> {
        match self.backend.as_ref() {
            ProjectBackend::Local(local) => delete_project_entry(&local.root, relative_path),
            ProjectBackend::Ssh(project) => {
                let relative = remote_relative(relative_path)
                    .map_err(|message| entry_remote_error(relative_path, message))?;
                project
                    .delete_entry(relative)
                    .map_err(|error| map_entry_error(relative_path, error))
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
                operation: "paste entries involving an SSH project",
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
                let output = project
                    .run_command("git", args)
                    .map_err(|error| error.to_string())?;
                Ok(GitCommandOutput {
                    success: output.success(),
                    exit_code: output.exit_status,
                    stdout: output.stdout,
                    stderr: output.stderr,
                })
            }
        }
    }

    fn null_device_path(&self) -> &'static str {
        match self.backend.as_ref() {
            ProjectBackend::Local(_) if cfg!(windows) => "NUL",
            ProjectBackend::Local(_) | ProjectBackend::Ssh(_) => "/dev/null",
        }
    }
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

fn remote_document_path(project: &SftpProject, relative: &RemoteRelativePathBuf) -> PathBuf {
    let mut path = PathBuf::from(project.root().as_str());
    for component in relative
        .as_str()
        .split('/')
        .filter(|component| !component.is_empty())
    {
        path.push(component);
    }
    path
}

fn remote_directory_snapshot(snapshot: RemoteDirectorySnapshot) -> DirectorySnapshot {
    DirectorySnapshot {
        relative_directory: pathbuf_from_remote(&snapshot.relative_directory),
        entries: snapshot
            .entries
            .into_iter()
            .map(|entry| ProjectTreeEntry {
                name: OsString::from(entry.name),
                relative_path: pathbuf_from_remote(&entry.relative_path),
                kind: project_entry_kind(entry.kind),
            })
            .collect(),
    }
}

fn remote_loaded_file(
    project: &SftpProject,
    file: RemoteLoadedFile,
) -> Result<LoadedProjectFile, ProjectFileIoError> {
    let relative_path = pathbuf_from_remote(&file.relative_path);
    let canonical_path = remote_document_path(project, &file.relative_path);
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
        relative_path: pathbuf_from_remote(&mutation.relative_path),
        kind: project_entry_kind(mutation.kind),
    }
}

fn project_entry_kind(kind: RemoteEntryKind) -> ProjectTreeEntryKind {
    match kind {
        RemoteEntryKind::Directory => ProjectTreeEntryKind::Directory,
        RemoteEntryKind::File => ProjectTreeEntryKind::File,
        RemoteEntryKind::SymlinkFile => ProjectTreeEntryKind::SymlinkFile,
        RemoteEntryKind::SymlinkDirectory => ProjectTreeEntryKind::SymlinkDirectory,
    }
}

fn remote_save_outcome(outcome: RemoteSaveOutcome) -> SaveProjectFileOutcome {
    match outcome {
        RemoteSaveOutcome::Saved(fingerprint) => {
            SaveProjectFileOutcome::Saved(disk_fingerprint(fingerprint))
        }
        RemoteSaveOutcome::Conflict(RemoteFileState::Missing) => {
            SaveProjectFileOutcome::Conflict(CurrentDiskState::Missing)
        }
        RemoteSaveOutcome::Conflict(RemoteFileState::Present(fingerprint)) => {
            SaveProjectFileOutcome::Conflict(CurrentDiskState::Present(disk_fingerprint(
                fingerprint,
            )))
        }
    }
}

fn remote_fingerprint(fingerprint: &DiskFingerprint) -> RemoteFingerprint {
    let modified_seconds = fingerprint.modified.and_then(|modified| {
        modified
            .duration_since(UNIX_EPOCH)
            .ok()
            .and_then(|duration| u32::try_from(duration.as_secs()).ok())
    });
    RemoteFingerprint {
        byte_len: fingerprint.byte_len,
        modified_seconds,
        content_hash: fingerprint.content_hash,
    }
}

fn disk_fingerprint(fingerprint: RemoteFingerprint) -> DiskFingerprint {
    DiskFingerprint {
        exists: true,
        byte_len: fingerprint.byte_len,
        modified: fingerprint
            .modified_seconds
            .map(|seconds| UNIX_EPOCH + Duration::from_secs(u64::from(seconds))),
        content_hash: fingerprint.content_hash,
    }
}

fn map_file_error(path: &Path, error: SftpError) -> ProjectFileIoError {
    match error {
        SftpError::PathOutsideRoot(_) => ProjectFileIoError::PathOutsideProject {
            path: path.to_path_buf(),
        },
        SftpError::NotFile(_) => ProjectFileIoError::NotAFile {
            path: path.to_path_buf(),
        },
        SftpError::FileTooLarge { size, limit, .. } => ProjectFileIoError::FileTooLarge {
            path: path.to_path_buf(),
            size,
            limit,
        },
        error => file_remote_error(path, error.to_string()),
    }
}

fn map_tree_error(path: &Path, error: SftpError) -> ProjectTreeFsError {
    match error {
        SftpError::PathOutsideRoot(_) => ProjectTreeFsError::PathOutsideProject {
            path: path.to_path_buf(),
        },
        SftpError::NotDirectory(_) => ProjectTreeFsError::NotDirectory {
            path: path.to_path_buf(),
        },
        SftpError::SymlinkDirectory(_) => ProjectTreeFsError::SymlinkDirectory {
            path: path.to_path_buf(),
        },
        error => tree_remote_error(path, error.to_string()),
    }
}

fn map_entry_error(path: &Path, error: SftpError) -> ProjectEntryFsError {
    match error {
        SftpError::ProjectRootMutation => ProjectEntryFsError::ProjectRootMutation,
        SftpError::PathOutsideRoot(_) => ProjectEntryFsError::PathOutsideProject {
            path: path.to_path_buf(),
        },
        SftpError::NotDirectory(_) => ProjectEntryFsError::NotDirectory {
            path: path.to_path_buf(),
        },
        SftpError::AlreadyExists(_) => ProjectEntryFsError::AlreadyExists {
            path: path.to_path_buf(),
        },
        error => entry_remote_error(path, error.to_string()),
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
        let services = ProjectServices::local(temp.path());

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
        let remote = RemoteFingerprint {
            byte_len: 42,
            modified_seconds: Some(123),
            content_hash: 99,
        };
        assert_eq!(
            remote_fingerprint(&disk_fingerprint(remote.clone())),
            remote
        );
    }
}
