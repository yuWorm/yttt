use std::{
    collections::hash_map::DefaultHasher,
    fs::{self, File, OpenOptions},
    hash::{Hash, Hasher},
    io::{Read, Write},
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

pub const MAX_PROJECT_FILE_BYTES: u64 = 10 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DiskFingerprint {
    pub exists: bool,
    pub byte_len: u64,
    pub modified: Option<SystemTime>,
    pub content_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoadedProjectFile {
    pub canonical_path: PathBuf,
    pub relative_path: PathBuf,
    pub text: String,
    pub fingerprint: DiskFingerprint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CurrentDiskState {
    Missing,
    Present(DiskFingerprint),
}

#[derive(Clone, Copy, Debug)]
pub enum SaveMode<'a> {
    Check(&'a DiskFingerprint),
    Force,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SaveProjectFileOutcome {
    Saved(DiskFingerprint),
    Conflict(CurrentDiskState),
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectFileIoError {
    #[error("path is outside the project: {path}")]
    PathOutsideProject { path: PathBuf },
    #[error("project file is not a regular file: {path}")]
    NotAFile { path: PathBuf },
    #[error("project file is {size} bytes, exceeding the {limit} byte limit: {path}")]
    FileTooLarge {
        path: PathBuf,
        size: u64,
        limit: u64,
    },
    #[error("project file contains NUL bytes and is treated as binary: {path}")]
    BinaryContent { path: PathBuf },
    #[error("project file is not valid UTF-8: {path}")]
    InvalidUtf8 { path: PathBuf },
    #[error("failed to access project file {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("remote project file error at {path}: {message}")]
    Remote { path: PathBuf, message: String },
}

pub fn read_project_file(
    root: &Path,
    relative_path: &Path,
) -> Result<LoadedProjectFile, ProjectFileIoError> {
    let relative_path = normalize_relative_path(relative_path)?;
    let canonical_root = fs::canonicalize(root).map_err(|source| ProjectFileIoError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let requested_path = canonical_root.join(&relative_path);
    let canonical_path =
        fs::canonicalize(&requested_path).map_err(|source| ProjectFileIoError::Io {
            path: requested_path.clone(),
            source,
        })?;

    if !canonical_path.starts_with(&canonical_root) {
        return Err(ProjectFileIoError::PathOutsideProject {
            path: relative_path,
        });
    }

    let mut file = File::open(&canonical_path).map_err(|source| ProjectFileIoError::Io {
        path: canonical_path.clone(),
        source,
    })?;
    let metadata = file.metadata().map_err(|source| ProjectFileIoError::Io {
        path: canonical_path.clone(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ProjectFileIoError::NotAFile {
            path: relative_path,
        });
    }
    if metadata.len() > MAX_PROJECT_FILE_BYTES {
        return Err(ProjectFileIoError::FileTooLarge {
            path: relative_path,
            size: metadata.len(),
            limit: MAX_PROJECT_FILE_BYTES,
        });
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_PROJECT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ProjectFileIoError::Io {
            path: canonical_path.clone(),
            source,
        })?;
    if bytes.len() as u64 > MAX_PROJECT_FILE_BYTES {
        return Err(ProjectFileIoError::FileTooLarge {
            path: relative_path,
            size: bytes.len() as u64,
            limit: MAX_PROJECT_FILE_BYTES,
        });
    }
    if bytes.contains(&0) {
        return Err(ProjectFileIoError::BinaryContent {
            path: relative_path,
        });
    }

    let text = String::from_utf8(bytes.clone()).map_err(|_| ProjectFileIoError::InvalidUtf8 {
        path: relative_path.clone(),
    })?;
    let metadata = file.metadata().map_err(|source| ProjectFileIoError::Io {
        path: canonical_path.clone(),
        source,
    })?;

    let relative_path = canonical_path
        .strip_prefix(&canonical_root)
        .expect("contained canonical project path must strip its root")
        .to_path_buf();

    Ok(LoadedProjectFile {
        canonical_path,
        relative_path,
        text,
        fingerprint: fingerprint_for_bytes(&bytes, &metadata),
    })
}

pub fn project_relative_path(
    root: &Path,
    canonical_path: &Path,
) -> Result<PathBuf, ProjectFileIoError> {
    let canonical_root = fs::canonicalize(root).map_err(|source| ProjectFileIoError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let relative_path = canonical_path.strip_prefix(&canonical_root).map_err(|_| {
        ProjectFileIoError::PathOutsideProject {
            path: canonical_path.to_path_buf(),
        }
    })?;
    normalize_relative_path(relative_path)
}

pub fn save_project_file(
    root: &Path,
    relative_path: &Path,
    text: &str,
    mode: SaveMode<'_>,
) -> Result<SaveProjectFileOutcome, ProjectFileIoError> {
    save_project_file_with_fs(root, relative_path, text, mode, &StdAtomicFileSystem)
}

trait AtomicFileSystem {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()>;
}

struct StdAtomicFileSystem;

impl AtomicFileSystem for StdAtomicFileSystem {
    fn rename(&self, from: &Path, to: &Path) -> std::io::Result<()> {
        fs::rename(from, to)
    }
}

fn save_project_file_with_fs(
    root: &Path,
    relative_path: &Path,
    text: &str,
    mode: SaveMode<'_>,
    file_system: &impl AtomicFileSystem,
) -> Result<SaveProjectFileOutcome, ProjectFileIoError> {
    let relative_path = normalize_relative_path(relative_path)?;
    if text.len() as u64 > MAX_PROJECT_FILE_BYTES {
        return Err(ProjectFileIoError::FileTooLarge {
            path: relative_path,
            size: text.len() as u64,
            limit: MAX_PROJECT_FILE_BYTES,
        });
    }
    if text.as_bytes().contains(&0) {
        return Err(ProjectFileIoError::BinaryContent {
            path: relative_path,
        });
    }

    let canonical_root = fs::canonicalize(root).map_err(|source| ProjectFileIoError::Io {
        path: root.to_path_buf(),
        source,
    })?;
    let target_path = resolve_save_target(&canonical_root, &relative_path)?;
    let current_state = current_disk_state(&target_path, &relative_path)?;
    if let SaveMode::Check(expected) = mode {
        let matches_expected = match &current_state {
            CurrentDiskState::Missing => !expected.exists,
            CurrentDiskState::Present(current) => current == expected,
        };
        if !matches_expected {
            return Ok(SaveProjectFileOutcome::Conflict(current_state));
        }
    }

    let original_permissions = match fs::metadata(&target_path) {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(source) => {
            return Err(ProjectFileIoError::Io {
                path: target_path,
                source,
            });
        }
    };
    let parent = target_path
        .parent()
        .expect("validated project file target must have a parent");
    let temp_path = parent.join(format!(".yttt-save-{}.tmp", uuid::Uuid::new_v4()));

    let write_result = (|| -> std::io::Result<()> {
        let mut temp = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        temp.write_all(text.as_bytes())?;
        temp.sync_all()?;
        drop(temp);
        if let Some(permissions) = original_permissions {
            fs::set_permissions(&temp_path, permissions)?;
        }
        file_system.rename(&temp_path, &target_path)
    })();

    if let Err(source) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(ProjectFileIoError::Io {
            path: target_path,
            source,
        });
    }

    match current_disk_state(&target_path, &relative_path)? {
        CurrentDiskState::Present(fingerprint) => Ok(SaveProjectFileOutcome::Saved(fingerprint)),
        CurrentDiskState::Missing => Err(ProjectFileIoError::Io {
            path: target_path,
            source: std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "saved file disappeared before it could be fingerprinted",
            ),
        }),
    }
}

fn resolve_save_target(
    canonical_root: &Path,
    relative_path: &Path,
) -> Result<PathBuf, ProjectFileIoError> {
    let requested_path = canonical_root.join(relative_path);
    match fs::symlink_metadata(&requested_path) {
        Ok(_) => {
            let canonical_target =
                fs::canonicalize(&requested_path).map_err(|source| ProjectFileIoError::Io {
                    path: requested_path.clone(),
                    source,
                })?;
            if !canonical_target.starts_with(canonical_root) {
                return Err(ProjectFileIoError::PathOutsideProject {
                    path: relative_path.to_path_buf(),
                });
            }
            let metadata =
                fs::metadata(&canonical_target).map_err(|source| ProjectFileIoError::Io {
                    path: canonical_target.clone(),
                    source,
                })?;
            if !metadata.is_file() {
                return Err(ProjectFileIoError::NotAFile {
                    path: relative_path.to_path_buf(),
                });
            }
            Ok(canonical_target)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let Some(parent) = requested_path.parent() else {
                return Err(ProjectFileIoError::PathOutsideProject {
                    path: relative_path.to_path_buf(),
                });
            };
            let canonical_parent =
                fs::canonicalize(parent).map_err(|source| ProjectFileIoError::Io {
                    path: parent.to_path_buf(),
                    source,
                })?;
            if !canonical_parent.starts_with(canonical_root) {
                return Err(ProjectFileIoError::PathOutsideProject {
                    path: relative_path.to_path_buf(),
                });
            }
            Ok(canonical_parent.join(
                requested_path
                    .file_name()
                    .expect("normalized relative file path must have a name"),
            ))
        }
        Err(source) => Err(ProjectFileIoError::Io {
            path: requested_path,
            source,
        }),
    }
}

fn current_disk_state(
    target_path: &Path,
    relative_path: &Path,
) -> Result<CurrentDiskState, ProjectFileIoError> {
    let mut file = match File::open(target_path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(CurrentDiskState::Missing);
        }
        Err(source) => {
            return Err(ProjectFileIoError::Io {
                path: target_path.to_path_buf(),
                source,
            });
        }
    };
    let metadata = file.metadata().map_err(|source| ProjectFileIoError::Io {
        path: target_path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() {
        return Err(ProjectFileIoError::NotAFile {
            path: relative_path.to_path_buf(),
        });
    }
    if metadata.len() > MAX_PROJECT_FILE_BYTES {
        return Err(ProjectFileIoError::FileTooLarge {
            path: relative_path.to_path_buf(),
            size: metadata.len(),
            limit: MAX_PROJECT_FILE_BYTES,
        });
    }

    let mut bytes = Vec::with_capacity(metadata.len() as usize);
    Read::by_ref(&mut file)
        .take(MAX_PROJECT_FILE_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| ProjectFileIoError::Io {
            path: target_path.to_path_buf(),
            source,
        })?;
    if bytes.len() as u64 > MAX_PROJECT_FILE_BYTES {
        return Err(ProjectFileIoError::FileTooLarge {
            path: relative_path.to_path_buf(),
            size: bytes.len() as u64,
            limit: MAX_PROJECT_FILE_BYTES,
        });
    }
    let metadata = file.metadata().map_err(|source| ProjectFileIoError::Io {
        path: target_path.to_path_buf(),
        source,
    })?;
    Ok(CurrentDiskState::Present(fingerprint_for_bytes(
        &bytes, &metadata,
    )))
}

fn normalize_relative_path(path: &Path) -> Result<PathBuf, ProjectFileIoError> {
    let mut normalized = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Normal(component) => normalized.push(component),
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => {
                return Err(ProjectFileIoError::PathOutsideProject {
                    path: path.to_path_buf(),
                });
            }
        }
    }

    if normalized.as_os_str().is_empty() {
        return Err(ProjectFileIoError::PathOutsideProject {
            path: path.to_path_buf(),
        });
    }

    Ok(normalized)
}

fn fingerprint_for_bytes(bytes: &[u8], metadata: &fs::Metadata) -> DiskFingerprint {
    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    DiskFingerprint {
        exists: true,
        byte_len: bytes.len() as u64,
        modified: metadata.modified().ok(),
        content_hash: hasher.finish(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct FailingFileSystem;

    impl AtomicFileSystem for FailingFileSystem {
        fn rename(&self, _from: &Path, _to: &Path) -> std::io::Result<()> {
            Err(std::io::Error::other("injected rename failure"))
        }
    }

    #[test]
    fn atomic_write_rename_failure_preserves_target_and_cleans_temp_file() {
        let root = tempfile::tempdir().unwrap();
        fs::write(root.path().join("notes.txt"), "old").unwrap();

        let result = save_project_file_with_fs(
            root.path(),
            Path::new("notes.txt"),
            "new",
            SaveMode::Force,
            &FailingFileSystem,
        );

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(root.path().join("notes.txt")).unwrap(),
            "old"
        );
        assert_eq!(fs::read_dir(root.path()).unwrap().count(), 1);
    }
}
