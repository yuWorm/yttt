use std::{
    future::Future,
    pin::Pin,
    sync::atomic::{AtomicU64, Ordering},
};

use russh_sftp::{
    client::{SftpSession, error::Error as ProtocolError},
    protocol::{FileAttributes, FileType, OpenFlags, StatusCode},
};
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use yttt_core::model::project::{RemotePathBuf, RemoteRelativePathBuf};

static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteEntryKind {
    Directory,
    File,
    SymlinkFile,
    SymlinkDirectory,
}

impl RemoteEntryKind {
    pub fn is_directory(self) -> bool {
        matches!(self, Self::Directory | Self::SymlinkDirectory)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDirectoryEntry {
    pub name: String,
    pub relative_path: RemoteRelativePathBuf,
    pub kind: RemoteEntryKind,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteDirectorySnapshot {
    pub relative_directory: RemoteRelativePathBuf,
    pub entries: Vec<RemoteDirectoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFingerprint {
    pub byte_len: u64,
    pub modified_seconds: Option<u32>,
    pub content_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteLoadedFile {
    pub canonical_path: RemotePathBuf,
    pub relative_path: RemoteRelativePathBuf,
    pub bytes: Vec<u8>,
    pub fingerprint: RemoteFingerprint,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteFileState {
    Missing,
    Present(RemoteFingerprint),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteSaveOutcome {
    Saved(RemoteFingerprint),
    Conflict(RemoteFileState),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteEntryMutation {
    pub relative_path: RemoteRelativePathBuf,
    pub kind: RemoteEntryKind,
}

#[derive(Clone, Debug)]
pub(crate) enum SftpOperation {
    ResolveHome,
    ScanDirectory {
        relative_directory: RemoteRelativePathBuf,
        show_hidden: bool,
    },
    ReadFile {
        relative_path: RemoteRelativePathBuf,
        max_bytes: u64,
    },
    SaveFile {
        relative_path: RemoteRelativePathBuf,
        bytes: Vec<u8>,
        expected: Option<RemoteFingerprint>,
        force: bool,
        max_bytes: u64,
    },
    CreateEntry {
        relative_path: RemoteRelativePathBuf,
        directory: bool,
    },
    RenameEntry {
        relative_path: RemoteRelativePathBuf,
        new_name: String,
    },
    DeleteEntry {
        relative_path: RemoteRelativePathBuf,
    },
}

#[derive(Debug)]
pub(crate) enum SftpResponse {
    Path(RemotePathBuf),
    Directory(RemoteDirectorySnapshot),
    File(RemoteLoadedFile),
    Save(RemoteSaveOutcome),
    Mutation(RemoteEntryMutation),
    Deleted,
}

pub(crate) async fn execute(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    operation: SftpOperation,
) -> Result<SftpResponse, SftpError> {
    match operation {
        SftpOperation::ResolveHome => resolve_home(sftp).await.map(SftpResponse::Path),
        SftpOperation::ScanDirectory {
            relative_directory,
            show_hidden,
        } => scan_directory(sftp, root, relative_directory, show_hidden)
            .await
            .map(SftpResponse::Directory),
        SftpOperation::ReadFile {
            relative_path,
            max_bytes,
        } => read_file(sftp, root, relative_path, max_bytes)
            .await
            .map(SftpResponse::File),
        SftpOperation::SaveFile {
            relative_path,
            bytes,
            expected,
            force,
            max_bytes,
        } => save_file(sftp, root, relative_path, bytes, expected, force, max_bytes)
            .await
            .map(SftpResponse::Save),
        SftpOperation::CreateEntry {
            relative_path,
            directory,
        } => create_entry(sftp, root, relative_path, directory)
            .await
            .map(SftpResponse::Mutation),
        SftpOperation::RenameEntry {
            relative_path,
            new_name,
        } => rename_entry(sftp, root, relative_path, new_name)
            .await
            .map(SftpResponse::Mutation),
        SftpOperation::DeleteEntry { relative_path } => {
            delete_entry(sftp, root, relative_path).await?;
            Ok(SftpResponse::Deleted)
        }
    }
}

async fn resolve_home(sftp: &SftpSession) -> Result<RemotePathBuf, SftpError> {
    let canonical = sftp.canonicalize(".").await.map_err(protocol_error)?;
    RemotePathBuf::new(canonical).map_err(|error| SftpError::InvalidPath(error.to_string()))
}

async fn scan_directory(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative_directory: RemoteRelativePathBuf,
    show_hidden: bool,
) -> Result<RemoteDirectorySnapshot, SftpError> {
    let path = resolve_directory(sftp, root, &relative_directory).await?;

    let mut entries = Vec::new();
    for entry in sftp.read_dir(path.as_str()).await.map_err(protocol_error)? {
        let name = entry.file_name();
        if !show_hidden && name.starts_with('.') {
            continue;
        }
        let relative_path = relative_directory
            .join(&name)
            .map_err(|error| SftpError::InvalidPath(error.to_string()))?;
        let metadata = entry.metadata();
        let kind = if metadata.is_symlink() {
            match sftp.metadata(entry.path()).await {
                Ok(target) if target.is_dir() => RemoteEntryKind::SymlinkDirectory,
                _ => RemoteEntryKind::SymlinkFile,
            }
        } else if metadata.is_dir() {
            RemoteEntryKind::Directory
        } else {
            RemoteEntryKind::File
        };
        entries.push(RemoteDirectoryEntry {
            name,
            relative_path,
            kind,
        });
    }
    entries.sort_by(|left, right| {
        usize::from(!left.kind.is_directory())
            .cmp(&usize::from(!right.kind.is_directory()))
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
            .then_with(|| left.name.cmp(&right.name))
    });
    Ok(RemoteDirectorySnapshot {
        relative_directory,
        entries,
    })
}

async fn read_file(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative_path: RemoteRelativePathBuf,
    max_bytes: u64,
) -> Result<RemoteLoadedFile, SftpError> {
    let canonical_path = resolve_existing(sftp, root, &relative_path, false).await?;
    let metadata = sftp
        .metadata(canonical_path.as_str())
        .await
        .map_err(protocol_error)?;
    if !metadata.is_regular() {
        return Err(SftpError::NotFile(relative_path));
    }
    if metadata.len() > max_bytes {
        return Err(SftpError::FileTooLarge {
            path: relative_path,
            size: metadata.len(),
            limit: max_bytes,
        });
    }
    let bytes = read_limited(sftp, canonical_path.as_str(), &relative_path, max_bytes).await?;
    let metadata = sftp
        .metadata(canonical_path.as_str())
        .await
        .map_err(protocol_error)?;
    let fingerprint = fingerprint(&bytes, &metadata);
    Ok(RemoteLoadedFile {
        canonical_path,
        relative_path,
        bytes,
        fingerprint,
    })
}

async fn save_file(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative_path: RemoteRelativePathBuf,
    bytes: Vec<u8>,
    expected: Option<RemoteFingerprint>,
    force: bool,
    max_bytes: u64,
) -> Result<RemoteSaveOutcome, SftpError> {
    if bytes.len() as u64 > max_bytes {
        return Err(SftpError::FileTooLarge {
            path: relative_path,
            size: bytes.len() as u64,
            limit: max_bytes,
        });
    }
    let requested_parent = resolve_directory(sftp, root, &relative_path.parent()).await?;
    let requested_name = relative_path
        .file_name()
        .ok_or_else(|| SftpError::InvalidPath(relative_path.to_string()))?
        .to_string();
    let requested_target = join_absolute(&requested_parent, &requested_name)?;
    let before = current_file_state(sftp, root, &relative_path, max_bytes).await?;
    if !force && !expected_matches(expected.as_ref(), &before) {
        return Ok(RemoteSaveOutcome::Conflict(before));
    }
    let (parent_path, file_name, target) = match &before {
        RemoteFileState::Present(_) => {
            let target = resolve_existing(sftp, root, &relative_path, false).await?;
            let parent = absolute_parent(&target)?;
            let name = target
                .file_name()
                .ok_or_else(|| SftpError::InvalidPath(target.to_string()))?
                .to_string();
            (parent, name, target)
        }
        RemoteFileState::Missing => (requested_parent, requested_name, requested_target),
    };

    let (temporary, backup) = temporary_paths(sftp, &parent_path, &file_name).await?;
    let mut temporary_file = sftp
        .open_with_flags(
            temporary.as_str(),
            OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
        )
        .await
        .map_err(protocol_error)?;
    if let Err(error) = temporary_file.write_all(&bytes).await {
        let _ = sftp.remove_file(temporary.as_str()).await;
        return Err(SftpError::Protocol(error.to_string()));
    }
    if let Err(error) = temporary_file.shutdown().await {
        let _ = sftp.remove_file(temporary.as_str()).await;
        return Err(SftpError::Protocol(error.to_string()));
    }
    if let RemoteFileState::Present(_) = before {
        let metadata = match sftp.metadata(target.as_str()).await {
            Ok(metadata) => metadata,
            Err(error) => {
                let _ = sftp.remove_file(temporary.as_str()).await;
                return Err(protocol_error(error));
            }
        };
        if let Some(permissions) = metadata.permissions {
            let mut attributes = FileAttributes::empty();
            attributes.permissions = Some(permissions);
            if let Err(error) = sftp.set_metadata(temporary.as_str(), attributes).await {
                let _ = sftp.remove_file(temporary.as_str()).await;
                return Err(protocol_error(error));
            }
        }
    }

    let after_upload = current_file_state(sftp, root, &relative_path, max_bytes).await?;
    if after_upload != before {
        let _ = sftp.remove_file(temporary.as_str()).await;
        return Ok(RemoteSaveOutcome::Conflict(after_upload));
    }

    if matches!(before, RemoteFileState::Present(_)) {
        if let Err(error) = sftp.rename(target.as_str(), backup.as_str()).await {
            let _ = sftp.remove_file(temporary.as_str()).await;
            return Err(protocol_error(error));
        }
        if let Err(error) = sftp.rename(temporary.as_str(), target.as_str()).await {
            let _ = sftp.rename(backup.as_str(), target.as_str()).await;
            let _ = sftp.remove_file(temporary.as_str()).await;
            return Err(protocol_error(error));
        }
        let _ = sftp.remove_file(backup.as_str()).await;
    } else if let Err(error) = sftp.rename(temporary.as_str(), target.as_str()).await {
        let _ = sftp.remove_file(temporary.as_str()).await;
        return Err(protocol_error(error));
    }

    let metadata = sftp
        .metadata(target.as_str())
        .await
        .map_err(protocol_error)?;
    Ok(RemoteSaveOutcome::Saved(fingerprint(&bytes, &metadata)))
}

async fn create_entry(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative_path: RemoteRelativePathBuf,
    directory: bool,
) -> Result<RemoteEntryMutation, SftpError> {
    if relative_path.as_str().is_empty() {
        return Err(SftpError::ProjectRootMutation);
    }
    let root = canonical_root(sftp, root).await?;
    let components = relative_path.as_str().split('/').collect::<Vec<_>>();
    let (name, parents) = components
        .split_last()
        .ok_or(SftpError::ProjectRootMutation)?;
    let mut parent = root;
    for component in parents {
        parent = join_absolute(&parent, component)?;
        match sftp.symlink_metadata(parent.as_str()).await {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(SftpError::NotDirectory(relative_path.clone())),
            Err(error) if is_not_found(&error) => {
                sftp.create_dir(parent.as_str())
                    .await
                    .map_err(protocol_error)?;
            }
            Err(error) => return Err(protocol_error(error)),
        }
    }
    let path = join_absolute(&parent, name)?;
    if sftp
        .try_exists(path.as_str())
        .await
        .map_err(protocol_error)?
    {
        return Err(SftpError::AlreadyExists(relative_path));
    }
    let kind = if directory {
        sftp.create_dir(path.as_str())
            .await
            .map_err(protocol_error)?;
        RemoteEntryKind::Directory
    } else {
        let mut file = sftp
            .open_with_flags(
                path.as_str(),
                OpenFlags::CREATE | OpenFlags::EXCLUDE | OpenFlags::WRITE,
            )
            .await
            .map_err(protocol_error)?;
        file.shutdown()
            .await
            .map_err(|error| SftpError::Protocol(error.to_string()))?;
        RemoteEntryKind::File
    };
    Ok(RemoteEntryMutation {
        relative_path,
        kind,
    })
}

async fn rename_entry(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative_path: RemoteRelativePathBuf,
    new_name: String,
) -> Result<RemoteEntryMutation, SftpError> {
    if relative_path.as_str().is_empty() {
        return Err(SftpError::ProjectRootMutation);
    }
    let destination_relative = relative_path
        .parent()
        .join(&new_name)
        .map_err(|error| SftpError::InvalidPath(error.to_string()))?;
    if destination_relative == relative_path {
        let source = resolve_entry_no_follow(sftp, root, &relative_path).await?;
        let metadata = sftp
            .symlink_metadata(source.as_str())
            .await
            .map_err(protocol_error)?;
        return Ok(RemoteEntryMutation {
            relative_path,
            kind: entry_kind(sftp, source.as_str(), &metadata).await,
        });
    }
    let source = resolve_entry_no_follow(sftp, root, &relative_path).await?;
    let destination_parent = resolve_directory(sftp, root, &relative_path.parent()).await?;
    let destination = join_absolute(&destination_parent, &new_name)?;
    if sftp
        .try_exists(destination.as_str())
        .await
        .map_err(protocol_error)?
    {
        return Err(SftpError::AlreadyExists(destination_relative));
    }
    let metadata = sftp
        .symlink_metadata(source.as_str())
        .await
        .map_err(protocol_error)?;
    let kind = entry_kind(sftp, source.as_str(), &metadata).await;
    sftp.rename(source.as_str(), destination.as_str())
        .await
        .map_err(protocol_error)?;
    Ok(RemoteEntryMutation {
        relative_path: destination_relative,
        kind,
    })
}

async fn delete_entry(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative_path: RemoteRelativePathBuf,
) -> Result<(), SftpError> {
    if relative_path.as_str().is_empty() {
        return Err(SftpError::ProjectRootMutation);
    }
    let path = resolve_entry_no_follow(sftp, root, &relative_path).await?;
    remove_recursively(sftp, path.as_str()).await
}

fn remove_recursively<'a>(
    sftp: &'a SftpSession,
    path: &'a str,
) -> Pin<Box<dyn Future<Output = Result<(), SftpError>> + Send + 'a>> {
    Box::pin(async move {
        let metadata = sftp.symlink_metadata(path).await.map_err(protocol_error)?;
        if metadata.is_dir() && !metadata.is_symlink() {
            for entry in sftp.read_dir(path).await.map_err(protocol_error)? {
                let child = entry.path();
                remove_recursively(sftp, &child).await?;
            }
            sftp.remove_dir(path).await.map_err(protocol_error)
        } else {
            sftp.remove_file(path).await.map_err(protocol_error)
        }
    })
}

async fn current_file_state(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative_path: &RemoteRelativePathBuf,
    max_bytes: u64,
) -> Result<RemoteFileState, SftpError> {
    let parent = resolve_directory(sftp, root, &relative_path.parent()).await?;
    let file_name = relative_path
        .file_name()
        .ok_or_else(|| SftpError::NotFile(relative_path.clone()))?;
    let joined = join_absolute(&parent, file_name)?;
    if !sftp
        .try_exists(joined.as_str())
        .await
        .map_err(protocol_error)?
    {
        return Ok(RemoteFileState::Missing);
    }
    let path = resolve_existing(sftp, root, relative_path, false).await?;
    let metadata = sftp.metadata(path.as_str()).await.map_err(protocol_error)?;
    if !metadata.is_regular() {
        return Err(SftpError::NotFile(relative_path.clone()));
    }
    if metadata.len() > max_bytes {
        return Err(SftpError::FileTooLarge {
            path: relative_path.clone(),
            size: metadata.len(),
            limit: max_bytes,
        });
    }
    let bytes = read_limited(sftp, path.as_str(), relative_path, max_bytes).await?;
    let metadata = sftp.metadata(path.as_str()).await.map_err(protocol_error)?;
    Ok(RemoteFileState::Present(fingerprint(&bytes, &metadata)))
}

async fn read_limited(
    sftp: &SftpSession,
    path: &str,
    relative_path: &RemoteRelativePathBuf,
    max_bytes: u64,
) -> Result<Vec<u8>, SftpError> {
    let file = sftp.open(path).await.map_err(protocol_error)?;
    let mut bytes = Vec::new();
    file.take(max_bytes + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| SftpError::Protocol(error.to_string()))?;
    if bytes.len() as u64 > max_bytes {
        return Err(SftpError::FileTooLarge {
            path: relative_path.clone(),
            size: bytes.len() as u64,
            limit: max_bytes,
        });
    }
    Ok(bytes)
}

async fn canonical_root(
    sftp: &SftpSession,
    root: &RemotePathBuf,
) -> Result<RemotePathBuf, SftpError> {
    let canonical = sftp
        .canonicalize(root.as_str())
        .await
        .map_err(protocol_error)?;
    RemotePathBuf::new(canonical).map_err(|error| SftpError::InvalidPath(error.to_string()))
}

async fn resolve_directory(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative: &RemoteRelativePathBuf,
) -> Result<RemotePathBuf, SftpError> {
    let canonical_root = canonical_root(sftp, root).await?;
    let mut current = canonical_root.clone();
    for component in relative
        .as_str()
        .split('/')
        .filter(|component| !component.is_empty())
    {
        let next = join_absolute(&current, component)?;
        let metadata = sftp
            .symlink_metadata(next.as_str())
            .await
            .map_err(protocol_error)?;
        if metadata.is_symlink() {
            return Err(SftpError::SymlinkDirectory(relative.clone()));
        }
        if !metadata.is_dir() {
            return Err(SftpError::NotDirectory(relative.clone()));
        }
        let canonical = sftp
            .canonicalize(next.as_str())
            .await
            .map_err(protocol_error)?;
        current = RemotePathBuf::new(canonical)
            .map_err(|error| SftpError::InvalidPath(error.to_string()))?;
        if !is_within(&canonical_root, &current) {
            return Err(SftpError::PathOutsideRoot(relative.clone()));
        }
    }
    Ok(current)
}

async fn resolve_entry_no_follow(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative: &RemoteRelativePathBuf,
) -> Result<RemotePathBuf, SftpError> {
    if relative.as_str().is_empty() {
        return Err(SftpError::ProjectRootMutation);
    }
    let parent = resolve_directory(sftp, root, &relative.parent()).await?;
    let name = relative.file_name().ok_or(SftpError::ProjectRootMutation)?;
    let path = join_absolute(&parent, name)?;
    sftp.symlink_metadata(path.as_str())
        .await
        .map_err(protocol_error)?;
    Ok(path)
}

async fn resolve_existing(
    sftp: &SftpSession,
    root: &RemotePathBuf,
    relative: &RemoteRelativePathBuf,
    allow_root: bool,
) -> Result<RemotePathBuf, SftpError> {
    if relative.as_str().is_empty() {
        return if allow_root {
            canonical_root(sftp, root).await
        } else {
            Err(SftpError::ProjectRootMutation)
        };
    }
    let canonical_root = canonical_root(sftp, root).await?;
    let parent = resolve_directory(sftp, root, &relative.parent()).await?;
    let requested = join_absolute(
        &parent,
        relative.file_name().ok_or(SftpError::ProjectRootMutation)?,
    )?;
    let canonical = sftp
        .canonicalize(requested.as_str())
        .await
        .map_err(protocol_error)?;
    let canonical =
        RemotePathBuf::new(canonical).map_err(|error| SftpError::InvalidPath(error.to_string()))?;
    if !is_within(&canonical_root, &canonical) {
        return Err(SftpError::PathOutsideRoot(relative.clone()));
    }
    Ok(canonical)
}

fn is_within(root: &RemotePathBuf, path: &RemotePathBuf) -> bool {
    path == root
        || root.as_str() == "/"
        || path
            .as_str()
            .strip_prefix(root.as_str())
            .is_some_and(|suffix| suffix.starts_with('/'))
}

fn join_absolute(parent: &RemotePathBuf, name: &str) -> Result<RemotePathBuf, SftpError> {
    let relative = RemoteRelativePathBuf::new(name)
        .map_err(|error| SftpError::InvalidPath(error.to_string()))?;
    if relative.as_str().contains('/') || relative.as_str().is_empty() {
        return Err(SftpError::InvalidPath(name.to_string()));
    }
    Ok(parent.join_relative(&relative))
}

fn absolute_parent(path: &RemotePathBuf) -> Result<RemotePathBuf, SftpError> {
    let parent =
        path.as_str().rsplit_once('/').map_or(
            "/",
            |(parent, _)| if parent.is_empty() { "/" } else { parent },
        );
    RemotePathBuf::new(parent.to_string())
        .map_err(|error| SftpError::InvalidPath(error.to_string()))
}

fn expected_matches(expected: Option<&RemoteFingerprint>, current: &RemoteFileState) -> bool {
    match (expected, current) {
        (Some(expected), RemoteFileState::Present(current)) => expected == current,
        (None, RemoteFileState::Missing) => true,
        _ => false,
    }
}

fn fingerprint(bytes: &[u8], metadata: &FileAttributes) -> RemoteFingerprint {
    use std::hash::{DefaultHasher, Hash, Hasher};

    let mut hasher = DefaultHasher::new();
    bytes.hash(&mut hasher);
    RemoteFingerprint {
        byte_len: bytes.len() as u64,
        modified_seconds: metadata.mtime,
        content_hash: hasher.finish(),
    }
}

async fn entry_kind(sftp: &SftpSession, path: &str, metadata: &FileAttributes) -> RemoteEntryKind {
    match metadata.file_type() {
        FileType::Dir => RemoteEntryKind::Directory,
        FileType::Symlink => match sftp.metadata(path).await {
            Ok(target) if target.is_dir() => RemoteEntryKind::SymlinkDirectory,
            _ => RemoteEntryKind::SymlinkFile,
        },
        FileType::File | FileType::Other => RemoteEntryKind::File,
    }
}

async fn temporary_paths(
    sftp: &SftpSession,
    parent: &RemotePathBuf,
    file_name: &str,
) -> Result<(RemotePathBuf, RemotePathBuf), SftpError> {
    for _ in 0..16 {
        let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let temporary = join_absolute(parent, &format!(".{file_name}.yttt-{sequence}.tmp"))?;
        let backup = join_absolute(parent, &format!(".{file_name}.yttt-{sequence}.bak"))?;
        let temporary_exists = sftp
            .try_exists(temporary.as_str())
            .await
            .map_err(protocol_error)?;
        let backup_exists = sftp
            .try_exists(backup.as_str())
            .await
            .map_err(protocol_error)?;
        if !temporary_exists && !backup_exists {
            return Ok((temporary, backup));
        }
    }
    Err(SftpError::TemporaryPathUnavailable)
}

fn is_not_found(error: &ProtocolError) -> bool {
    matches!(
        error,
        ProtocolError::Status(status) if status.status_code == StatusCode::NoSuchFile
    )
}

fn protocol_error(error: impl ToString) -> SftpError {
    SftpError::Protocol(error.to_string())
}

#[derive(Clone, Debug, Error, PartialEq, Eq)]
pub enum SftpError {
    #[error("SSH connection is not connected")]
    NotConnected,
    #[error("SFTP request timed out")]
    TimedOut,
    #[error("SFTP runtime stopped")]
    RuntimeStopped,
    #[error("invalid remote path: {0}")]
    InvalidPath(String),
    #[error("remote path is outside the project root: {0}")]
    PathOutsideRoot(RemoteRelativePathBuf),
    #[error("remote path is not a directory: {0}")]
    NotDirectory(RemoteRelativePathBuf),
    #[error("remote project tree does not traverse symlink directories: {0}")]
    SymlinkDirectory(RemoteRelativePathBuf),
    #[error("remote path is not a regular file: {0}")]
    NotFile(RemoteRelativePathBuf),
    #[error("remote file is {size} bytes, exceeding the {limit} byte limit: {path}")]
    FileTooLarge {
        path: RemoteRelativePathBuf,
        size: u64,
        limit: u64,
    },
    #[error("the remote project root cannot be changed")]
    ProjectRootMutation,
    #[error("remote project entry already exists: {0}")]
    AlreadyExists(RemoteRelativePathBuf),
    #[error("could not allocate a temporary remote save path")]
    TemporaryPathUnavailable,
    #[error("SFTP protocol error: {0}")]
    Protocol(String),
    #[error("unexpected SFTP response")]
    UnexpectedResponse,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn containment_uses_component_boundaries() {
        let root = RemotePathBuf::new("/srv/app").unwrap();
        assert!(is_within(&root, &RemotePathBuf::new("/srv/app").unwrap()));
        assert!(is_within(
            &root,
            &RemotePathBuf::new("/srv/app/src/main.rs").unwrap()
        ));
        assert!(!is_within(
            &root,
            &RemotePathBuf::new("/srv/application/secret").unwrap()
        ));
    }

    #[test]
    fn expected_state_requires_exact_fingerprint() {
        let expected = RemoteFingerprint {
            byte_len: 4,
            modified_seconds: Some(7),
            content_hash: 9,
        };
        assert!(expected_matches(
            Some(&expected),
            &RemoteFileState::Present(expected.clone())
        ));
        assert!(!expected_matches(
            Some(&expected),
            &RemoteFileState::Missing
        ));
        assert!(expected_matches(None, &RemoteFileState::Missing));
    }
}
