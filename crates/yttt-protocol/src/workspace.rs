use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use yttt_core::model::{
    editor::DocumentId,
    ids::{ClientInstanceId, TerminalSessionId},
};

use crate::{HostPath, PathSegment, ProjectRelativePath, project::ProjectFileFingerprint};

/// The largest stable identifier accepted for a workspace.
pub const MAX_WORKSPACE_ID_BYTES: usize = 64;

pub const MAX_DRAFT_CONTENT_BYTES: usize = 6 * 1024 * 1024;
pub const MAX_WORKSPACE_DRAFT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_AGENT_STATE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_WORKSPACE_AGENT_STATE_BYTES: usize = 256 * 1024 * 1024;
/// A short, path-safe identifier for one persisted workspace.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WorkspaceId(String);

impl WorkspaceId {
    /// Creates a workspace identifier containing only ASCII letters, digits, `_`, and `-`.
    pub fn new(value: impl Into<String>) -> Result<Self, WorkspaceValidationError> {
        let value = value.into();
        validate_identifier(&value, MAX_WORKSPACE_ID_BYTES, "workspace ID")?;
        Ok(Self(value))
    }

    /// Returns the validated identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WorkspaceId {
    type Error = WorkspaceValidationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<WorkspaceId> for String {
    fn from(value: WorkspaceId) -> Self {
        value.0
    }
}

/// A content revision for one recoverable draft item.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftContentRevision {
    /// Client-defined revision used to order revisions of this draft item.
    pub revision: u64,
    /// The source state against which this UTF-8 content was drafted.
    pub base: DraftBase,
    /// Recoverable UTF-8 draft content.
    pub content: String,
}

/// The source state that makes a draft safely applicable after reconnecting.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftBase {
    /// A draft based on a Host project file and its observed fingerprint.
    File {
        path: HostPath,
        base_fingerprint: ProjectFileFingerprint,
    },
    /// A draft based on arbitrary JSON state owned and interpreted by the UI.
    Json { metadata: Value },
}

/// The Client allowed to edit one open document, and the grant it holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentLease {
    pub holder: ClientInstanceId,
    /// Advances with every grant of the document; a draft upload must name
    /// the epoch of the lease it was edited under.
    pub epoch: u64,
}

/// The edit lease and the latest uploaded draft of one document open in a workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentState {
    pub document: DocumentId,
    pub lease: Option<DocumentLease>,
    pub draft: Option<DraftRef>,
}

/// A document of a workspace this Client opened changed its lease or draft.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentUpdate {
    pub workspace_id: WorkspaceId,
    pub state: DocumentState,
}

/// Immutable body identity and source metadata; content is transferred separately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftRef {
    pub document_id: WorkspaceId,
    pub revision: u64,
    pub base: DraftBase,
    pub content_sha256: [u8; 32],
    pub bytes: u64,
}

/// One pane's immutable Agent state, stored separately from the workspace layout.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStateRef {
    pub project_id: yttt_core::model::ids::ProjectId,
    pub tab_id: String,
    pub pane_id: String,
    pub content_sha256: [u8; 32],
    pub bytes: u64,
}

impl AgentStateRef {
    pub fn for_content(
        project_id: yttt_core::model::ids::ProjectId,
        tab_id: String,
        pane_id: String,
        content: &[u8],
    ) -> Self {
        use sha2::{Digest as _, Sha256};
        Self {
            project_id,
            tab_id,
            pane_id,
            content_sha256: Sha256::digest(content).into(),
            bytes: content.len() as u64,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub revision: u64,
    pub saved_millis: u64,
    /// Closed workspaces remain history, not startup windows.
    #[serde(default = "default_restore_on_startup")]
    pub restore_on_startup: bool,
}

fn default_restore_on_startup() -> bool {
    false
}

impl DraftRef {
    pub fn for_document(document_key: &[u8], draft: &DraftContentRevision) -> Self {
        use sha2::{Digest as _, Sha256};
        use std::fmt::Write as _;
        let mut id = String::with_capacity(64);
        for byte in Sha256::digest(document_key) {
            write!(&mut id, "{byte:02x}").expect("writing to String");
        }
        Self {
            document_id: WorkspaceId::new(id).expect("hex digest is a valid ID"),
            revision: draft.revision,
            base: draft.base.clone(),
            content_sha256: Sha256::digest(draft.content.as_bytes()).into(),
            bytes: draft.content.len() as u64,
        }
    }
}
/// A content-addressed revision for one bounded configuration document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfigRevision {
    pub content_sha256: [u8; 32],
}

/// A configuration document stored below its bounded configuration root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub relative_path: ProjectRelativePath,
    pub revision: WorkspaceConfigRevision,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

/// The bounded set of project configuration documents a remote client may access.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceProjectConfigFile {
    Settings,
    Layout,
}

impl WorkspaceProjectConfigFile {
    pub const fn file_name(self) -> &'static str {
        match self {
            Self::Settings => "settings.toml",
            Self::Layout => "layout.toml",
        }
    }
}

/// Encodes a project path as a stable, path-safe overlay directory name.
pub fn encode_project_path(path: &Path) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";

    let value = path.to_string_lossy();
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.') {
            encoded.push(byte as char);
        } else {
            encoded.push('%');
            encoded.push(HEX[(byte >> 4) as usize] as char);
            encoded.push(HEX[(byte & 0x0f) as usize] as char);
        }
    }
    encoded
}

/// A directory entry returned while choosing a Host project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDirectoryEntry {
    pub name: PathSegment,
    pub path: HostPath,
    pub kind: WorkspaceDirectoryEntryKind,
}

/// The filesystem kind of a browsed Host entry.
///
/// Directory symlinks are classified by following their target so clients can offer them as
/// browsable project roots while retaining the link path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceDirectoryEntryKind {
    /// A regular file.
    File,
    /// A directory that can be browsed or selected as a project root.
    Directory,
    /// A symbolic link that does not resolve to a directory.
    Symlink,
    /// A device, socket, FIFO, or another non-file filesystem object.
    Other,
    /// A symbolic link to a directory that can be browsed or selected as a project root.
    SymlinkDirectory,
}

/// A single, bounded page of Host directory entries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDirectory {
    pub path: HostPath,
    pub entries: Vec<WorkspaceDirectoryEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentSessionSummary {
    pub provider: String,
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub transcript_path: Option<HostPath>,
    pub updated_at_ms: u64,
}

/// A native Agent session mutation performed by the Host that owns its history.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgentSessionOperation {
    Rename { title: String },
    Delete,
}

/// Project-configuration routing selected by the Host profile.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceProjectConfig {
    Project,
    ReadOnlyProject,
    Overlay { root: HostPath },
}

/// The Host environment needed by a remote client before selecting projects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEnvironment {
    pub environment_id: String,
    pub config_root: HostPath,
    pub project_config: WorkspaceProjectConfig,
    pub home: HostPath,
    pub platform: String,
    pub shell: Option<String>,
    pub shell_candidates: Vec<String>,
    #[serde(default)]
    pub workspace_history_initialized: bool,
}

/// A remote workspace request served by the Host's single workspace writer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WorkspaceRequest {
    /// Returns the Host home directory, operating-system platform, and preferred shell.
    Environment,
    List,
    Register {
        workspace_id: WorkspaceId,
        name: String,
    },
    InstallAgentHooks,
    AgentSessions {
        providers: Vec<String>,
        project_root: HostPath,
    },
    /// Renames or permanently deletes one provider-owned project session.
    MutateAgentSession {
        provider: String,
        session_id: String,
        project_root: HostPath,
        operation: AgentSessionOperation,
    },
    /// Lists a Host directory so the client can choose a project root.
    Browse {
        path: HostPath,
        include_hidden: bool,
    },
    /// Reads one profile-scoped configuration document; a missing document is a valid empty state.
    ReadConfig {
        relative_path: ProjectRelativePath,
    },
    ListConfig {
        relative_directory: ProjectRelativePath,
    },
    CreateConfigDirectory {
        relative_directory: ProjectRelativePath,
    },
    /// Removes an empty shared configuration directory, never the configuration root.
    RemoveConfigDirectory {
        relative_directory: ProjectRelativePath,
    },
    /// Moves a shared directory to an absent destination without replacing any files.
    RenameConfigDirectory {
        relative_from: ProjectRelativePath,
        relative_to: ProjectRelativePath,
    },
    DeleteConfig {
        relative_path: ProjectRelativePath,
        expected_revision: WorkspaceConfigRevision,
    },
    /// Atomically writes one profile-scoped configuration document if its revision still matches.
    WriteConfig {
        relative_path: ProjectRelativePath,
        expected_revision: Option<WorkspaceConfigRevision>,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    /// Reads one bounded project configuration document without requiring workspace control.
    ReadProjectConfig {
        project_root: HostPath,
        file: WorkspaceProjectConfigFile,
    },
    /// Atomically writes one bounded project configuration document when its revision matches.
    WriteProjectConfig {
        project_root: HostPath,
        file: WorkspaceProjectConfigFile,
        expected_revision: Option<WorkspaceConfigRevision>,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    /// Deletes one bounded project configuration document when its revision matches.
    DeleteProjectConfig {
        project_root: HostPath,
        file: WorkspaceProjectConfigFile,
        expected_revision: WorkspaceConfigRevision,
    },

    /// Returns the current layout, documents, and Agent states of one
    /// workspace and subscribes this Client's Session connection to its
    /// layout and document changes.
    Open {
        workspace_id: WorkspaceId,
    },
    /// Applies one layout intent in arrival order. Any Client may apply intents;
    /// an intent that carries `expected_revision` is rejected when it is stale.
    Apply {
        workspace_id: WorkspaceId,
        intent: yttt_core::model::host_workspace::WorkspaceIntent,
        #[serde(default)]
        expected_revision: Option<u64>,
    },
    /// Grants this Client the edit lease of an open document. A lease held by
    /// another Client is granted only with `takeover`; the previous holder
    /// is told through `DocumentChanged`.
    AcquireDocument {
        workspace_id: WorkspaceId,
        document: DocumentId,
        takeover: bool,
    },
    /// Replaces the document's draft. Only the holder of lease `epoch` may upload.
    PutDocumentDraft {
        workspace_id: WorkspaceId,
        document: DocumentId,
        epoch: u64,
        revision: u64,
        base: DraftBase,
        #[serde(with = "serde_bytes")]
        content: Vec<u8>,
    },
    /// Reads the document's current draft body.
    GetDocumentDraft {
        workspace_id: WorkspaceId,
        document: DocumentId,
    },
    /// Releases the lease of a document that has no unsaved changes and
    /// discards its draft. A stale `epoch` changes nothing.
    ReleaseDocument {
        workspace_id: WorkspaceId,
        document: DocumentId,
        epoch: u64,
    },
    /// Checks a saved OMP ID on the Host independently of project-scoped history.
    OmpSessionExists {
        session_id: String,
    },
    /// Changes only startup membership; the saved layout and drafts remain intact.
    SetRestoreOnStartup {
        workspace_id: WorkspaceId,
        expected_revision: u64,
        restore: bool,
    },
    /// Explicitly deletes a closed workspace and its draft and Agent state bodies.
    Forget {
        workspace_id: WorkspaceId,
        expected_revision: u64,
    },
    /// Reads a known workspace without manufacturing an empty missing record.
    OpenExisting {
        workspace_id: WorkspaceId,
    },
    /// Reads one Agent state the Host stored for a pane of this workspace.
    GetAgentState {
        workspace_id: WorkspaceId,
        reference: AgentStateRef,
    },
}

impl WorkspaceRequest {
    pub fn is_mutation(&self) -> bool {
        !matches!(
            self,
            Self::Environment
                | Self::List
                | Self::AgentSessions { .. }
                | Self::OmpSessionExists { .. }
                | Self::Browse { .. }
                | Self::ReadConfig { .. }
                | Self::ReadProjectConfig { .. }
                | Self::ListConfig { .. }
                | Self::Open { .. }
                | Self::OpenExisting { .. }
                | Self::GetDocumentDraft { .. }
                | Self::GetAgentState { .. }
        )
    }
}

/// A remote workspace response produced by the Host's workspace service.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum WorkspaceResponse {
    /// Returns the Host environment for project selection and terminal defaults.
    Environment(WorkspaceEnvironment),
    Workspaces(Vec<WorkspaceSummary>),
    Registered(WorkspaceSummary),
    /// Complete matching history, with recoverable provider/file failures.
    AgentSessions {
        sessions: Vec<AgentSessionSummary>,
        warnings: Vec<String>,
    },
    /// Confirms one provider-owned session mutation.
    AgentSessionMutated,
    AgentHooksInstalled,
    Directory(WorkspaceDirectory),
    /// Returns a configuration document, or `None` when it has not been created.
    Config(Option<WorkspaceConfig>),
    ConfigDirectory(Vec<ProjectRelativePath>),
    ConfigDirectoryCreated,
    ConfigDeleted,
    ConfigDirectoryRenamed,
    /// Confirms an atomic configuration write and its new content revision.
    ConfigWritten {
        relative_path: ProjectRelativePath,
        revision: WorkspaceConfigRevision,
    },
    /// Returns a workspace's layout, its documents, and the Agent states of
    /// its panes.
    ///
    /// A `WorkspaceChanged` or `DocumentChanged` event for this workspace may
    /// arrive before this response; a Client keeps whichever layout has the
    /// higher revision.
    Opened {
        layout: yttt_core::model::host_workspace::HostWorkspaceState,
        documents: Vec<DocumentState>,
        agent_states: Vec<AgentStateRef>,
    },
    /// Confirms an applied layout intent and the revision it produced.
    Applied {
        workspace_id: WorkspaceId,
        revision: u64,
    },
    /// The document's lease and draft after a document request.
    Document(DocumentState),
    DocumentDraft {
        reference: DraftRef,
        #[serde(with = "serde_bytes")]
        content: Vec<u8>,
    },
    OmpSessionExists(bool),
    Forgotten {
        workspace_id: WorkspaceId,
    },
    AgentState {
        reference: AgentStateRef,
        #[serde(with = "serde_bytes")]
        content: Vec<u8>,
    },
}

/// The complete layout of one workspace after an accepted change.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct WorkspaceLayoutUpdate {
    pub workspace_id: WorkspaceId,
    pub layout: yttt_core::model::host_workspace::HostWorkspaceState,
}

/// The Host dropped a pane's Agent state; a Client stops showing what it holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentStateCleared {
    pub workspace_id: WorkspaceId,
    pub terminal_session_id: TerminalSessionId,
}

/// Validation failures raised while constructing strongly validated workspace wire values.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkspaceValidationError {
    /// The supplied workspace identifier is not a bounded safe identifier.
    #[error("{kind} must be 1 to {maximum} ASCII letters, digits, `_`, or `-`")]
    InvalidIdentifier { kind: &'static str, maximum: usize },
}

fn validate_identifier(
    value: &str,
    maximum: usize,
    kind: &'static str,
) -> Result<(), WorkspaceValidationError> {
    if value.is_empty()
        || value.len() > maximum
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
    {
        return Err(WorkspaceValidationError::InvalidIdentifier { kind, maximum });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_session_mutation_serializes_and_requires_mutation_permission() {
        let operation = AgentSessionOperation::Rename {
            title: "Renamed session".to_string(),
        };
        let serialized = serde_json::to_value(&operation).unwrap();
        assert_eq!(
            serialized,
            serde_json::json!({ "Rename": { "title": "Renamed session" } })
        );
        assert_eq!(
            serde_json::from_value::<AgentSessionOperation>(serialized).unwrap(),
            operation
        );

        let request = WorkspaceRequest::MutateAgentSession {
            provider: "codex".to_string(),
            session_id: "session-123".to_string(),
            project_root: HostPath::from_path(&std::env::temp_dir()).unwrap(),
            operation,
        };
        assert!(request.is_mutation());
    }
}
