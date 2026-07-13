use std::{
    ffi::{OsStr, OsString},
    fs::{self, OpenOptions},
    io,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectTreeEntryKind {
    Directory,
    File,
    SymlinkFile,
    SymlinkDirectory,
}

impl ProjectTreeEntryKind {
    pub fn is_directory(self) -> bool {
        matches!(self, Self::Directory | Self::SymlinkDirectory)
    }

    pub fn is_traversable(self) -> bool {
        matches!(self, Self::Directory)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectTreeEntry {
    pub name: OsString,
    pub relative_path: PathBuf,
    pub kind: ProjectTreeEntryKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DirectorySnapshot {
    pub relative_directory: PathBuf,
    pub entries: Vec<ProjectTreeEntry>,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectTreeFsError {
    #[error("path is outside the project: {path}")]
    PathOutsideProject { path: PathBuf },
    #[error("project tree path is not a directory: {path}")]
    NotDirectory { path: PathBuf },
    #[error("project tree does not traverse symlink directories: {path}")]
    SymlinkDirectory { path: PathBuf },
    #[error("failed to scan project tree path {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectEntryPasteMode {
    Copy,
    Cut,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectEntryMutation {
    pub relative_path: PathBuf,
    pub kind: ProjectTreeEntryKind,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectEntryFsError {
    #[error("invalid project entry name: {input:?}")]
    InvalidEntryName { input: String },
    #[error("the project root cannot be changed")]
    ProjectRootMutation,
    #[error("path is outside the project: {path}")]
    PathOutsideProject { path: PathBuf },
    #[error("project entry already exists: {path}")]
    AlreadyExists { path: PathBuf },
    #[error("project entry is not a directory: {path}")]
    NotDirectory { path: PathBuf },
    #[error("cannot paste a directory inside itself: {path}")]
    DestinationInsideSource { path: PathBuf },
    #[error("failed to {operation} project entry {path}: {source}")]
    Io {
        operation: &'static str,
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn create_project_entry(
    root: &Path,
    relative_parent: &Path,
    input: &str,
) -> Result<ProjectEntryMutation, ProjectEntryFsError> {
    let directory = input.ends_with('/');
    let entry_input = if directory {
        input.trim_end_matches('/')
    } else {
        input
    };
    let relative_entry = normalize_entry_input(entry_input)?;
    let canonical_root = canonical_project_root(root)?;
    let relative_parent = normalize_mutation_path(relative_parent, true)?;
    let parent = resolve_existing_directory(&canonical_root, &relative_parent)?;
    let (nested_parent, name) = split_relative_entry(&relative_entry, input)?;
    let destination_parent = ensure_relative_directories(&canonical_root, &parent, &nested_parent)?;
    let destination = destination_parent.join(name);
    let relative_path = relative_parent.join(&relative_entry);
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(ProjectEntryFsError::AlreadyExists {
            path: relative_path,
        });
    }

    if directory {
        fs::create_dir(&destination)
            .map_err(|source| mutation_io("create directory", &relative_path, source))?;
    } else {
        OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|source| mutation_io("create file", &relative_path, source))?;
    }

    Ok(ProjectEntryMutation {
        relative_path,
        kind: if directory {
            ProjectTreeEntryKind::Directory
        } else {
            ProjectTreeEntryKind::File
        },
    })
}

pub fn rename_project_entry(
    root: &Path,
    relative_path: &Path,
    new_name: &str,
) -> Result<ProjectEntryMutation, ProjectEntryFsError> {
    let relative_path = normalize_mutation_path(relative_path, false)?;
    let new_path = normalize_entry_input(new_name)?;
    if new_path.components().count() != 1 {
        return Err(ProjectEntryFsError::InvalidEntryName {
            input: new_name.to_string(),
        });
    }
    let canonical_root = canonical_project_root(root)?;
    let source = resolve_existing_entry(&canonical_root, &relative_path)?;
    let kind = entry_kind(&source.absolute_path, &source.metadata);
    let relative_parent = relative_path.parent().unwrap_or(Path::new(""));
    let destination_parent = resolve_existing_directory(&canonical_root, relative_parent)?;
    let destination = destination_parent.join(&new_path);
    let destination_relative = relative_parent.join(new_path);
    if destination == source.absolute_path {
        return Ok(ProjectEntryMutation {
            relative_path,
            kind,
        });
    }
    if fs::symlink_metadata(&destination).is_ok() {
        return Err(ProjectEntryFsError::AlreadyExists {
            path: destination_relative,
        });
    }
    fs::rename(&source.absolute_path, &destination)
        .map_err(|error| mutation_io("rename", &relative_path, error))?;
    Ok(ProjectEntryMutation {
        relative_path: destination_relative,
        kind,
    })
}

pub fn delete_project_entry(root: &Path, relative_path: &Path) -> Result<(), ProjectEntryFsError> {
    let relative_path = normalize_mutation_path(relative_path, false)?;
    let canonical_root = canonical_project_root(root)?;
    let source = resolve_existing_entry(&canonical_root, &relative_path)?;
    remove_entry(&source.absolute_path, &source.metadata)
        .map_err(|error| mutation_io("delete", &relative_path, error))
}

pub fn paste_project_entry(
    source_root: &Path,
    source_relative_path: &Path,
    destination_root: &Path,
    destination_relative_directory: &Path,
    mode: ProjectEntryPasteMode,
) -> Result<ProjectEntryMutation, ProjectEntryFsError> {
    let source_relative_path = normalize_mutation_path(source_relative_path, false)?;
    let destination_relative_directory =
        normalize_mutation_path(destination_relative_directory, true)?;
    let source_root = canonical_project_root(source_root)?;
    let destination_root = canonical_project_root(destination_root)?;
    let source = resolve_existing_entry(&source_root, &source_relative_path)?;
    let destination_directory =
        resolve_existing_directory(&destination_root, &destination_relative_directory)?;
    let source_name = source_relative_path
        .file_name()
        .ok_or(ProjectEntryFsError::ProjectRootMutation)?;
    let source_kind = entry_kind(&source.absolute_path, &source.metadata);
    let mut destination = destination_directory.join(source_name);
    let same_root = source_root == destination_root;

    if mode == ProjectEntryPasteMode::Cut && same_root && destination == source.absolute_path {
        return Ok(ProjectEntryMutation {
            relative_path: source_relative_path,
            kind: source_kind,
        });
    }
    destination =
        available_paste_destination(&destination, source_name, source_kind.is_directory(), mode)?;
    let destination_name = destination
        .file_name()
        .expect("paste destination always has a file name");
    let destination_relative_path = destination_relative_directory.join(destination_name);

    if source.metadata.is_dir() {
        let canonical_source = fs::canonicalize(&source.absolute_path)
            .map_err(|error| mutation_io("inspect", &source_relative_path, error))?;
        if destination_directory.starts_with(&canonical_source) {
            return Err(ProjectEntryFsError::DestinationInsideSource {
                path: destination_relative_path,
            });
        }
    }

    if mode == ProjectEntryPasteMode::Cut && same_root {
        fs::rename(&source.absolute_path, &destination)
            .map_err(|error| mutation_io("move", &source_relative_path, error))?;
    } else {
        if let Err(error) = copy_entry(&source.absolute_path, &destination) {
            let _ = remove_partial_copy(&destination);
            return Err(mutation_io("copy", &source_relative_path, error));
        }
        if mode == ProjectEntryPasteMode::Cut
            && let Err(error) = remove_entry(&source.absolute_path, &source.metadata)
        {
            return Err(mutation_io(
                "remove copied source",
                &source_relative_path,
                error,
            ));
        }
    }

    Ok(ProjectEntryMutation {
        relative_path: destination_relative_path,
        kind: source_kind,
    })
}

pub fn scan_project_directory(
    root: &Path,
    relative_directory: &Path,
    show_hidden: bool,
) -> Result<DirectorySnapshot, ProjectTreeFsError> {
    let relative_directory = normalize_relative_directory(relative_directory)?;
    let canonical_root = fs::canonicalize(root).map_err(|source| ProjectTreeFsError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let requested_path = canonical_root.join(&relative_directory);

    if !relative_directory.as_os_str().is_empty() {
        let link_metadata =
            fs::symlink_metadata(&requested_path).map_err(|source| ProjectTreeFsError::Io {
                path: requested_path.clone(),
                source,
            })?;
        if link_metadata.file_type().is_symlink()
            && fs::metadata(&requested_path).is_ok_and(|metadata| metadata.is_dir())
        {
            return Err(ProjectTreeFsError::SymlinkDirectory {
                path: relative_directory,
            });
        }
    }

    let canonical_directory =
        fs::canonicalize(&requested_path).map_err(|source| ProjectTreeFsError::Io {
            path: requested_path.clone(),
            source,
        })?;
    if !canonical_directory.starts_with(&canonical_root) {
        return Err(ProjectTreeFsError::PathOutsideProject {
            path: relative_directory,
        });
    }
    if !fs::metadata(&canonical_directory)
        .map_err(|source| ProjectTreeFsError::Io {
            path: canonical_directory.clone(),
            source,
        })?
        .is_dir()
    {
        return Err(ProjectTreeFsError::NotDirectory {
            path: relative_directory,
        });
    }

    let mut entries = Vec::new();
    let read_dir = fs::read_dir(&canonical_directory).map_err(|source| ProjectTreeFsError::Io {
        path: canonical_directory.clone(),
        source,
    })?;
    for entry in read_dir {
        let entry = entry.map_err(|source| ProjectTreeFsError::Io {
            path: canonical_directory.clone(),
            source,
        })?;
        let name = entry.file_name();
        if !show_hidden && name.to_string_lossy().starts_with('.') {
            continue;
        }

        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|source| ProjectTreeFsError::Io {
                path: entry.path(),
                source,
            })?;
        let kind = if metadata.file_type().is_symlink() {
            if fs::metadata(entry.path()).is_ok_and(|target| target.is_dir()) {
                ProjectTreeEntryKind::SymlinkDirectory
            } else {
                ProjectTreeEntryKind::SymlinkFile
            }
        } else if metadata.is_dir() {
            ProjectTreeEntryKind::Directory
        } else {
            ProjectTreeEntryKind::File
        };
        entries.push(ProjectTreeEntry {
            relative_path: relative_directory.join(&name),
            name,
            kind,
        });
    }

    entries.sort_by(|left, right| {
        let left_group = usize::from(!left.kind.is_directory());
        let right_group = usize::from(!right.kind.is_directory());
        left_group
            .cmp(&right_group)
            .then_with(|| {
                left.name
                    .to_string_lossy()
                    .to_lowercase()
                    .cmp(&right.name.to_string_lossy().to_lowercase())
            })
            .then_with(|| left.name.cmp(&right.name))
    });

    Ok(DirectorySnapshot {
        relative_directory,
        entries,
    })
}

fn normalize_relative_directory(path: &Path) -> Result<PathBuf, ProjectTreeFsError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ProjectTreeFsError::PathOutsideProject {
                    path: path.to_path_buf(),
                });
            }
        }
    }
    Ok(normalized)
}

struct ResolvedEntry {
    absolute_path: PathBuf,
    metadata: fs::Metadata,
}

fn canonical_project_root(root: &Path) -> Result<PathBuf, ProjectEntryFsError> {
    let canonical_root =
        fs::canonicalize(root).map_err(|error| mutation_io("open project", root, error))?;
    if !fs::metadata(&canonical_root)
        .map_err(|error| mutation_io("inspect project", root, error))?
        .is_dir()
    {
        return Err(ProjectEntryFsError::NotDirectory {
            path: root.to_path_buf(),
        });
    }
    Ok(canonical_root)
}

fn normalize_entry_input(input: &str) -> Result<PathBuf, ProjectEntryFsError> {
    if input.is_empty() {
        return Err(ProjectEntryFsError::InvalidEntryName {
            input: input.to_string(),
        });
    }
    let normalized =
        normalize_mutation_path(Path::new(input), false).map_err(|error| match error {
            ProjectEntryFsError::ProjectRootMutation
            | ProjectEntryFsError::PathOutsideProject { .. } => {
                ProjectEntryFsError::InvalidEntryName {
                    input: input.to_string(),
                }
            }
            error => error,
        })?;
    if normalized
        .components()
        .any(|component| component.as_os_str().is_empty())
    {
        return Err(ProjectEntryFsError::InvalidEntryName {
            input: input.to_string(),
        });
    }
    Ok(normalized)
}

fn normalize_mutation_path(path: &Path, allow_empty: bool) -> Result<PathBuf, ProjectEntryFsError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ProjectEntryFsError::PathOutsideProject {
                    path: path.to_path_buf(),
                });
            }
        }
    }
    if !allow_empty && normalized.as_os_str().is_empty() {
        return Err(ProjectEntryFsError::ProjectRootMutation);
    }
    Ok(normalized)
}

fn split_relative_entry<'a>(
    relative_entry: &'a Path,
    input: &str,
) -> Result<(PathBuf, &'a OsStr), ProjectEntryFsError> {
    let Some(name) = relative_entry.file_name() else {
        return Err(ProjectEntryFsError::InvalidEntryName {
            input: input.to_string(),
        });
    };
    Ok((
        relative_entry
            .parent()
            .unwrap_or(Path::new(""))
            .to_path_buf(),
        name,
    ))
}

fn resolve_existing_directory(
    canonical_root: &Path,
    relative_path: &Path,
) -> Result<PathBuf, ProjectEntryFsError> {
    let requested = canonical_root.join(relative_path);
    let canonical = fs::canonicalize(&requested)
        .map_err(|error| mutation_io("open directory", relative_path, error))?;
    if !canonical.starts_with(canonical_root) {
        return Err(ProjectEntryFsError::PathOutsideProject {
            path: relative_path.to_path_buf(),
        });
    }
    if !fs::metadata(&canonical)
        .map_err(|error| mutation_io("inspect directory", relative_path, error))?
        .is_dir()
    {
        return Err(ProjectEntryFsError::NotDirectory {
            path: relative_path.to_path_buf(),
        });
    }
    Ok(canonical)
}

fn ensure_relative_directories(
    canonical_root: &Path,
    base: &Path,
    relative_path: &Path,
) -> Result<PathBuf, ProjectEntryFsError> {
    let mut current = base.to_path_buf();
    for component in relative_path.components() {
        let Component::Normal(name) = component else {
            return Err(ProjectEntryFsError::PathOutsideProject {
                path: relative_path.to_path_buf(),
            });
        };
        let candidate = current.join(name);
        match fs::symlink_metadata(&candidate) {
            Ok(metadata) => {
                if !metadata.is_dir() && !metadata.file_type().is_symlink() {
                    return Err(ProjectEntryFsError::NotDirectory {
                        path: relative_path.to_path_buf(),
                    });
                }
                let canonical = fs::canonicalize(&candidate)
                    .map_err(|error| mutation_io("open directory", relative_path, error))?;
                if !canonical.starts_with(canonical_root) {
                    return Err(ProjectEntryFsError::PathOutsideProject {
                        path: relative_path.to_path_buf(),
                    });
                }
                if !fs::metadata(&canonical)
                    .map_err(|error| mutation_io("inspect directory", relative_path, error))?
                    .is_dir()
                {
                    return Err(ProjectEntryFsError::NotDirectory {
                        path: relative_path.to_path_buf(),
                    });
                }
                current = canonical;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                fs::create_dir(&candidate).map_err(|error| {
                    mutation_io("create parent directory", relative_path, error)
                })?;
                current = candidate;
            }
            Err(error) => {
                return Err(mutation_io("inspect directory", relative_path, error));
            }
        }
    }
    Ok(current)
}

fn resolve_existing_entry(
    canonical_root: &Path,
    relative_path: &Path,
) -> Result<ResolvedEntry, ProjectEntryFsError> {
    let parent_relative = relative_path.parent().unwrap_or(Path::new(""));
    let parent = resolve_existing_directory(canonical_root, parent_relative)?;
    let name = relative_path
        .file_name()
        .ok_or(ProjectEntryFsError::ProjectRootMutation)?;
    let absolute_path = parent.join(name);
    let metadata = fs::symlink_metadata(&absolute_path)
        .map_err(|error| mutation_io("open", relative_path, error))?;
    Ok(ResolvedEntry {
        absolute_path,
        metadata,
    })
}

fn entry_kind(path: &Path, metadata: &fs::Metadata) -> ProjectTreeEntryKind {
    if metadata.file_type().is_symlink() {
        if fs::metadata(path).is_ok_and(|target| target.is_dir()) {
            ProjectTreeEntryKind::SymlinkDirectory
        } else {
            ProjectTreeEntryKind::SymlinkFile
        }
    } else if metadata.is_dir() {
        ProjectTreeEntryKind::Directory
    } else {
        ProjectTreeEntryKind::File
    }
}

fn available_paste_destination(
    initial: &Path,
    source_name: &OsStr,
    directory: bool,
    mode: ProjectEntryPasteMode,
) -> Result<PathBuf, ProjectEntryFsError> {
    if fs::symlink_metadata(initial).is_err() {
        return Ok(initial.to_path_buf());
    }
    if mode == ProjectEntryPasteMode::Cut {
        return Err(ProjectEntryFsError::AlreadyExists {
            path: initial.to_path_buf(),
        });
    }
    for sequence in 1.. {
        let candidate = initial.with_file_name(copy_name(source_name, directory, sequence));
        if fs::symlink_metadata(&candidate).is_err() {
            return Ok(candidate);
        }
    }
    unreachable!("copy sequence is unbounded")
}

fn copy_name(source_name: &OsStr, directory: bool, sequence: usize) -> OsString {
    let source_path = Path::new(source_name);
    let suffix = if sequence == 1 {
        " copy".to_string()
    } else {
        format!(" copy {sequence}")
    };
    if directory {
        return format!("{}{suffix}", source_name.to_string_lossy()).into();
    }
    let stem = source_path
        .file_stem()
        .unwrap_or(source_name)
        .to_string_lossy();
    match source_path.extension() {
        Some(extension) => format!("{stem}{suffix}.{}", extension.to_string_lossy()).into(),
        None => format!("{stem}{suffix}").into(),
    }
}

fn copy_entry(source: &Path, destination: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return copy_symlink(source, destination);
    }
    if metadata.is_dir() {
        fs::create_dir(destination)?;
        for entry in fs::read_dir(source)? {
            let entry = entry?;
            copy_entry(&entry.path(), &destination.join(entry.file_name()))?;
        }
        fs::set_permissions(destination, metadata.permissions())?;
        return Ok(());
    }
    fs::copy(source, destination)?;
    Ok(())
}

#[cfg(unix)]
fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(fs::read_link(source)?, destination)
}

#[cfg(windows)]
fn copy_symlink(source: &Path, destination: &Path) -> io::Result<()> {
    let target = fs::read_link(source)?;
    if fs::metadata(source).is_ok_and(|metadata| metadata.is_dir()) {
        std::os::windows::fs::symlink_dir(target, destination)
    } else {
        std::os::windows::fs::symlink_file(target, destination)
    }
}

#[cfg(not(any(unix, windows)))]
fn copy_symlink(_source: &Path, _destination: &Path) -> io::Result<()> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "copying symbolic links is unsupported on this platform",
    ))
}

fn remove_entry(path: &Path, metadata: &fs::Metadata) -> io::Result<()> {
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn remove_partial_copy(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => remove_entry(path, &metadata),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

fn mutation_io(operation: &'static str, path: &Path, source: io::Error) -> ProjectEntryFsError {
    ProjectEntryFsError::Io {
        operation,
        path: path.to_path_buf(),
        source,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_entry_uses_trailing_slash_as_directory_contract() {
        let project = tempdir().unwrap();

        let file = create_project_entry(project.path(), Path::new(""), "src/lib.rs").unwrap();
        assert_eq!(file.relative_path, Path::new("src/lib.rs"));
        assert_eq!(file.kind, ProjectTreeEntryKind::File);
        assert!(project.path().join("src/lib.rs").is_file());

        let directory =
            create_project_entry(project.path(), Path::new(""), "assets/icons/").unwrap();
        assert_eq!(directory.relative_path, Path::new("assets/icons"));
        assert_eq!(directory.kind, ProjectTreeEntryKind::Directory);
        assert!(project.path().join("assets/icons").is_dir());

        let plain = create_project_entry(project.path(), Path::new(""), "README").unwrap();
        assert_eq!(plain.kind, ProjectTreeEntryKind::File);
        assert!(project.path().join("README").is_file());
    }

    #[test]
    fn create_entry_rejects_escape_and_overwrite() {
        let project = tempdir().unwrap();
        fs::write(project.path().join("existing.txt"), "keep").unwrap();

        assert!(matches!(
            create_project_entry(project.path(), Path::new(""), "../outside.txt"),
            Err(ProjectEntryFsError::InvalidEntryName { .. })
        ));
        assert!(matches!(
            create_project_entry(project.path(), Path::new(""), "existing.txt"),
            Err(ProjectEntryFsError::AlreadyExists { .. })
        ));
        assert_eq!(
            fs::read_to_string(project.path().join("existing.txt")).unwrap(),
            "keep"
        );
        assert!(
            !project
                .path()
                .parent()
                .unwrap()
                .join("outside.txt")
                .exists()
        );
    }

    #[test]
    fn rename_and_delete_preserve_entry_kind_and_contents() {
        let project = tempdir().unwrap();
        fs::create_dir(project.path().join("folder")).unwrap();
        fs::write(project.path().join("folder/original.txt"), "contents").unwrap();

        let renamed = rename_project_entry(
            project.path(),
            Path::new("folder/original.txt"),
            "renamed.txt",
        )
        .unwrap();
        assert_eq!(renamed.relative_path, Path::new("folder/renamed.txt"));
        assert_eq!(renamed.kind, ProjectTreeEntryKind::File);
        assert!(!project.path().join("folder/original.txt").exists());
        assert_eq!(
            fs::read_to_string(project.path().join("folder/renamed.txt")).unwrap(),
            "contents"
        );

        delete_project_entry(project.path(), Path::new("folder")).unwrap();
        assert!(!project.path().join("folder").exists());
    }

    #[test]
    fn copy_paste_chooses_non_destructive_names() {
        let project = tempdir().unwrap();
        fs::write(project.path().join("notes.txt"), "notes").unwrap();

        let first = paste_project_entry(
            project.path(),
            Path::new("notes.txt"),
            project.path(),
            Path::new(""),
            ProjectEntryPasteMode::Copy,
        )
        .unwrap();
        assert_eq!(first.relative_path, Path::new("notes copy.txt"));
        assert_eq!(
            fs::read_to_string(project.path().join(&first.relative_path)).unwrap(),
            "notes"
        );

        let second = paste_project_entry(
            project.path(),
            Path::new("notes.txt"),
            project.path(),
            Path::new(""),
            ProjectEntryPasteMode::Copy,
        )
        .unwrap();
        assert_eq!(second.relative_path, Path::new("notes copy 2.txt"));
        assert!(project.path().join("notes.txt").exists());
    }

    #[test]
    fn paste_copies_directories_and_cut_moves_entries() {
        let project = tempdir().unwrap();
        fs::create_dir_all(project.path().join("source/nested")).unwrap();
        fs::write(project.path().join("source/nested/data.txt"), "data").unwrap();
        fs::create_dir(project.path().join("target")).unwrap();

        let copied = paste_project_entry(
            project.path(),
            Path::new("source"),
            project.path(),
            Path::new("target"),
            ProjectEntryPasteMode::Copy,
        )
        .unwrap();
        assert_eq!(copied.relative_path, Path::new("target/source"));
        assert_eq!(
            fs::read_to_string(project.path().join("target/source/nested/data.txt")).unwrap(),
            "data"
        );

        let moved = paste_project_entry(
            project.path(),
            Path::new("source/nested/data.txt"),
            project.path(),
            Path::new(""),
            ProjectEntryPasteMode::Cut,
        )
        .unwrap();
        assert_eq!(moved.relative_path, Path::new("data.txt"));
        assert!(!project.path().join("source/nested/data.txt").exists());
        assert_eq!(
            fs::read_to_string(project.path().join("data.txt")).unwrap(),
            "data"
        );
    }

    #[test]
    fn paste_rejects_copying_directory_inside_itself() {
        let project = tempdir().unwrap();
        fs::create_dir_all(project.path().join("source/nested")).unwrap();

        assert!(matches!(
            paste_project_entry(
                project.path(),
                Path::new("source"),
                project.path(),
                Path::new("source/nested"),
                ProjectEntryPasteMode::Copy,
            ),
            Err(ProjectEntryFsError::DestinationInsideSource { .. })
        ));
        assert!(!project.path().join("source/nested/source").exists());
    }
}
