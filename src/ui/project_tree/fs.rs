use std::{
    ffi::OsString,
    fs,
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
