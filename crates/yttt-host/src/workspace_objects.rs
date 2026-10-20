use sha2::{Digest as _, Sha256};
use std::{
    collections::HashSet,
    fs, io,
    path::{Path, PathBuf},
};
use yttt_protocol::workspace::{
    AgentStateRef, DraftRef, MAX_AGENT_STATE_BYTES, MAX_DRAFT_CONTENT_BYTES,
    MAX_WORKSPACE_AGENT_STATE_BYTES, MAX_WORKSPACE_DRAFT_BYTES, WorkspaceId,
};

pub(crate) trait ObjectReference {
    const DIRECTORY: &'static str;
    const MAX_CONTENT_BYTES: usize;
    const MAX_WORKSPACE_BYTES: usize;
    const MAX_STAGED_BYTES: usize = Self::MAX_WORKSPACE_BYTES;
    fn identity(&self) -> (&str, &str, &str);
    fn digest(&self) -> &[u8; 32];
    fn bytes(&self) -> u64;
}

impl ObjectReference for DraftRef {
    const DIRECTORY: &'static str = "drafts";
    const MAX_CONTENT_BYTES: usize = MAX_DRAFT_CONTENT_BYTES;
    const MAX_WORKSPACE_BYTES: usize = MAX_WORKSPACE_DRAFT_BYTES;
    fn identity(&self) -> (&str, &str, &str) {
        (self.document_id.as_str(), "", "")
    }
    fn digest(&self) -> &[u8; 32] {
        &self.content_sha256
    }
    fn bytes(&self) -> u64 {
        self.bytes
    }
}

impl ObjectReference for AgentStateRef {
    const DIRECTORY: &'static str = "agent-states";
    const MAX_CONTENT_BYTES: usize = MAX_AGENT_STATE_BYTES;
    const MAX_WORKSPACE_BYTES: usize = MAX_WORKSPACE_AGENT_STATE_BYTES;
    // Keep the published generation intact while its replacement is staged.
    const MAX_STAGED_BYTES: usize = MAX_WORKSPACE_AGENT_STATE_BYTES * 2;
    fn identity(&self) -> (&str, &str, &str) {
        (self.project_id.as_str(), &self.tab_id, &self.pane_id)
    }
    fn digest(&self) -> &[u8; 32] {
        &self.content_sha256
    }
    fn bytes(&self) -> u64 {
        self.bytes
    }
}

pub(crate) struct WorkspaceObjects<R> {
    root: PathBuf,
    reference: std::marker::PhantomData<R>,
}

impl<R: ObjectReference> WorkspaceObjects<R> {
    pub fn new(state_root: &Path) -> io::Result<Self> {
        let root = state_root.join(R::DIRECTORY);
        fs::create_dir_all(&root)?;
        check_directory(&root)?;
        Ok(Self {
            root,
            reference: std::marker::PhantomData,
        })
    }

    fn directory(&self, workspace: &WorkspaceId) -> PathBuf {
        self.root.join(workspace.as_str())
    }
    fn path(&self, workspace: &WorkspaceId, reference: &R) -> PathBuf {
        self.directory(workspace)
            .join(digest_name(reference.digest()))
    }

    pub fn put(&self, workspace: &WorkspaceId, reference: &R, content: &[u8]) -> io::Result<()> {
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
                return Err(invalid("workspace object directory contains a non-file"));
            }
            total = total
                .checked_add(metadata.len())
                .ok_or_else(|| invalid("workspace object quota overflow"))?;
        }
        if total > R::MAX_STAGED_BYTES as u64 {
            return Err(invalid("workspace staged objects exceed their byte limit"));
        }
        super::workspace::atomic_write(&path, content, uuid::Uuid::new_v4().as_u128() as u64)
    }

    pub fn read(&self, workspace: &WorkspaceId, reference: &R) -> io::Result<Vec<u8>> {
        check_directory(&self.directory(workspace))?;
        let path = self.path(workspace, reference);
        let metadata = fs::symlink_metadata(&path)?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(invalid("workspace object must be a regular file"));
        }
        let content = super::workspace::read_bounded_file(&path, R::MAX_CONTENT_BYTES)?;
        validate_content(reference, &content)?;
        Ok(content)
    }

    pub fn validate_references(&self, workspace: &WorkspaceId, references: &[R]) -> io::Result<()> {
        let mut documents = HashSet::new();
        let mut total = 0u64;
        for reference in references {
            if !documents.insert(reference.identity()) {
                return Err(invalid("duplicate workspace object identity"));
            }
            total = total
                .checked_add(reference.bytes())
                .ok_or_else(|| invalid("workspace object quota overflow"))?;
            if total > R::MAX_WORKSPACE_BYTES as u64 {
                return Err(invalid("workspace objects exceed their byte limit"));
            }
            self.read(workspace, reference)?;
        }
        Ok(())
    }

    /// Only obsolete published bodies are collected during a live session. Concurrent uploads
    /// may already be preparing the next manifest and must survive the current commit.
    pub fn collect_obsolete(&self, workspace: &WorkspaceId, old: &[R], current: &[R]) {
        for reference in old {
            if !current
                .iter()
                .any(|item| item.digest() == reference.digest())
            {
                let _ = fs::remove_file(self.path(workspace, reference));
            }
        }
    }

    /// Called only before accepting Clients, when there can be no in-flight publication.
    pub fn recover(&self, workspace: &WorkspaceId, current: &[R]) -> io::Result<()> {
        self.validate_references(workspace, current)?;
        let directory = self.directory(workspace);
        match fs::read_dir(&directory) {
            Ok(entries) => {
                let retained = current
                    .iter()
                    .map(|item| digest_name(item.digest()))
                    .collect::<HashSet<_>>();
                for entry in entries {
                    let entry = entry?;
                    if !entry.file_type()?.is_file() {
                        return Err(invalid("workspace object directory contains a non-file"));
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

    pub fn remove_workspace(&self, workspace: &WorkspaceId) -> io::Result<()> {
        let directory = self.directory(workspace);
        match fs::symlink_metadata(&directory) {
            Ok(_) => {
                check_directory(&directory)?;
                fs::remove_dir_all(directory)?;
                #[cfg(unix)]
                fs::File::open(&self.root)?.sync_all()?;
                Ok(())
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
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
        Err(invalid("workspace object path must be a real directory"))
    }
}
fn validate_content<R: ObjectReference>(reference: &R, content: &[u8]) -> io::Result<()> {
    if content.len() > R::MAX_CONTENT_BYTES || reference.bytes() != content.len() as u64 {
        return Err(invalid(
            "workspace object size mismatch or content limit exceeded",
        ));
    }
    std::str::from_utf8(content).map_err(io::Error::other)?;
    let digest: [u8; 32] = Sha256::digest(content).into();
    if &digest != reference.digest() {
        return Err(invalid("workspace object digest mismatch"));
    }
    Ok(())
}
