use sha2::{Digest as _, Sha256};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};
use yttt_protocol::workspace::{
    DraftRef, MAX_DRAFT_CONTENT_BYTES, MAX_WORKSPACE_DRAFT_BYTES, WorkspaceId,
};

pub(crate) struct DraftObjects {
    root: PathBuf,
}

impl DraftObjects {
    pub fn new(state_root: &Path) -> io::Result<Self> {
        let root = state_root.join("drafts");
        fs::create_dir_all(&root)?;
        check_directory(&root)?;
        Ok(Self { root })
    }

    fn directory(&self, workspace: &WorkspaceId) -> PathBuf {
        self.root.join(workspace.as_str())
    }
    fn path(&self, workspace: &WorkspaceId, reference: &DraftRef) -> PathBuf {
        self.directory(workspace)
            .join(digest_name(&reference.content_sha256))
    }

    pub fn put(
        &self,
        workspace: &WorkspaceId,
        reference: &DraftRef,
        content: &[u8],
    ) -> io::Result<()> {
        validate_content(reference, content)?;
        let directory = self.directory(workspace);
        fs::create_dir_all(&directory)?;
        check_directory(&directory)?;
        let path = self.path(workspace, reference);
        match fs::symlink_metadata(&path) {
            Ok(_) => return self.read(workspace, reference).map(|_| ()),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        let mut total = content.len() as u64;
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            let metadata = entry.metadata()?;
            if !entry.file_type()?.is_file() {
                return Err(invalid("draft directory contains a non-file"));
            }
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| invalid("draft quota overflow"))?;
        }
        if total > MAX_WORKSPACE_DRAFT_BYTES as u64 {
            return Err(invalid("workspace drafts exceed 64 MiB"));
        }
        super::workspace::atomic_write(&path, content, uuid::Uuid::new_v4().as_u128() as u64)
    }

    pub fn read(&self, workspace: &WorkspaceId, reference: &DraftRef) -> io::Result<Vec<u8>> {
        check_directory(&self.directory(workspace))?;
        let path = self.path(workspace, reference);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("draft body must be a regular file"));
        }
        let content = super::workspace::read_bounded_file(&path, MAX_DRAFT_CONTENT_BYTES)?;
        validate_content(reference, &content)?;
        Ok(content)
    }

    pub fn validate_references(
        &self,
        workspace: &WorkspaceId,
        references: &[DraftRef],
    ) -> io::Result<()> {
        let mut documents = HashSet::new();
        let mut total = 0u64;
        for reference in references {
            if !documents.insert(&reference.document_id) {
                return Err(invalid("duplicate draft document ID"));
            }
            total = total
                .checked_add(reference.bytes)
                .ok_or_else(|| invalid("draft quota overflow"))?;
            if total > MAX_WORKSPACE_DRAFT_BYTES as u64 {
                return Err(invalid("workspace drafts exceed 64 MiB"));
            }
            self.read(workspace, reference)?;
        }
        Ok(())
    }

    /// Only obsolete published bodies are collected during a live session. Concurrent uploads
    /// may already be preparing the next manifest and must survive the current commit.
    pub fn collect_obsolete(
        &self,
        workspace: &WorkspaceId,
        old: &[DraftRef],
        current: &[DraftRef],
    ) {
        for reference in old {
            if !current
                .iter()
                .any(|item| item.content_sha256 == reference.content_sha256)
            {
                let _ = fs::remove_file(self.path(workspace, reference));
            }
        }
    }

    /// Called only before accepting Clients, when there can be no in-flight publication.
    pub fn recover(&self, workspace: &WorkspaceId, current: &[DraftRef]) -> io::Result<()> {
        self.validate_references(workspace, current)?;
        let directory = self.directory(workspace);
        match fs::read_dir(&directory) {
            Ok(entries) => {
                let retained = current
                    .iter()
                    .map(|item| digest_name(&item.content_sha256))
                    .collect::<HashSet<_>>();
                for entry in entries {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        return Err(invalid("draft directory contains a non-file"));
                    }
                    if !retained.contains(&entry.file_name().to_string_lossy().into_owned()) {
                        fs::remove_file(entry.path())?;
                    }
                }
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound && current.is_empty() => Ok(()),
            Err(error) => Err(error),
        }
    }
}

fn digest_name(digest: &[u8; 32]) -> String {
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn check_directory(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        Ok(())
    } else {
        Err(invalid("draft path must be a real directory"))
    }
}
fn validate_content(reference: &DraftRef, content: &[u8]) -> io::Result<()> {
    if content.len() > MAX_DRAFT_CONTENT_BYTES || reference.bytes != content.len() as u64 {
        return Err(invalid("draft body size mismatch or exceeds 6 MiB"));
    }
    std::str::from_utf8(content).map_err(io::Error::other)?;
    let digest: [u8; 32] = Sha256::digest(content).into();
    if digest != reference.content_sha256 {
        return Err(invalid("draft body digest mismatch"));
    }
    Ok(())
}
