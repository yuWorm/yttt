use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{HostPath, PathSegment, ProjectRelativePath, project::ProjectFileFingerprint};

/// The largest stable identifier accepted for a workspace or an idempotent operation.
pub const MAX_WORKSPACE_ID_BYTES: usize = 64;
pub const MAX_OPERATION_ID_BYTES: usize = 128;

pub const MAX_DRAFT_CONTENT_BYTES: usize = 6 * 1024 * 1024;
pub const MAX_WORKSPACE_DRAFT_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_WORKSPACE_MANIFEST_BYTES: usize = 1024 * 1024;
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

/// A short, path-safe key for a bounded idempotent commit record.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct WorkspaceOperationId(String);

impl WorkspaceOperationId {
    /// Creates an operation identifier containing only ASCII letters, digits, `_`, and `-`.
    pub fn new(value: impl Into<String>) -> Result<Self, WorkspaceValidationError> {
        let value = value.into();
        validate_identifier(&value, MAX_OPERATION_ID_BYTES, "workspace operation ID")?;
        Ok(Self(value))
    }

    /// Returns the validated identifier text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for WorkspaceOperationId {
    type Error = WorkspaceValidationError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<WorkspaceOperationId> for String {
    fn from(value: WorkspaceOperationId) -> Self {
        value.0
    }
}

/// A pure serialized UI snapshot. The Host intentionally does not interpret its schema.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WorkspaceSnapshot(#[serde(deserialize_with = "deserialize_snapshot")] Value);

impl WorkspaceSnapshot {
    /// Creates a snapshot only when its top-level JSON value is an object.
    pub fn new(value: Value) -> Result<Self, WorkspaceValidationError> {
        if value.is_object() {
            Ok(Self(value))
        } else {
            Err(WorkspaceValidationError::SnapshotMustBeObject)
        }
    }

    /// Returns the opaque JSON object supplied by the UI.
    pub fn as_value(&self) -> &Value {
        &self.0
    }

    /// Consumes this wrapper and returns the opaque JSON object.
    pub fn into_value(self) -> Value {
        self.0
    }
}

fn deserialize_snapshot<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(
            "workspace snapshot must be a JSON object",
        ))
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

/// All recoverable draft items for one workspace.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDraft {
    pub contents: Vec<DraftContentRevision>,
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceSummary {
    pub workspace_id: WorkspaceId,
    pub name: String,
    pub revision: u64,
    pub saved_millis: u64,
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
/// A content-addressed revision of a profile configuration document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfigRevision {
    pub content_sha256: [u8; 32],
}

/// A profile configuration document stored below that profile's configuration root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceConfig {
    pub relative_path: ProjectRelativePath,
    pub revision: WorkspaceConfigRevision,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
}

/// A directory entry returned while choosing a Host project root.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceDirectoryEntry {
    pub name: PathSegment,
    pub path: HostPath,
    pub kind: WorkspaceDirectoryEntryKind,
}

/// The non-following filesystem kind of a browsed Host entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceDirectoryEntryKind {
    /// A regular file.
    File,
    /// A directory that can be browsed or selected as a project root.
    Directory,
    /// A symbolic link, reported without following it.
    Symlink,
    /// A device, socket, FIFO, or another non-file filesystem object.
    Other,
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

/// The Host environment needed by a remote client before selecting projects.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceEnvironment {
    pub environment_id: String,
    pub config_root: HostPath,
    pub home: HostPath,
    pub platform: String,
    pub shell: Option<String>,
    pub shell_candidates: Vec<String>,
}

/// A remote workspace request served by the Host's single workspace writer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
    /// Opens the current snapshot, revision, and transient control owner for one workspace.
    Open {
        workspace_id: WorkspaceId,
    },
    /// Commits a snapshot with compare-and-swap revision checking and bounded idempotency replay.
    Commit {
        workspace_id: WorkspaceId,
        expected_revision: u64,
        operation_id: WorkspaceOperationId,
        snapshot: WorkspaceSnapshot,
        drafts: Vec<DraftRef>,
    },
    /// Persist an immutable body before publishing a manifest that references it.
    PutDraft {
        workspace_id: WorkspaceId,
        reference: DraftRef,
        #[serde(with = "serde_bytes")]
        content: Vec<u8>,
    },
    /// Read one published body, bounded independently of the manifest.
    GetDraft {
        workspace_id: WorkspaceId,
        reference: DraftRef,
    },
}

impl WorkspaceRequest {
    pub fn is_mutation(&self) -> bool {
        !matches!(
            self,
            Self::Environment
                | Self::List
                | Self::AgentSessions { .. }
                | Self::Browse { .. }
                | Self::ReadConfig { .. }
                | Self::ListConfig { .. }
                | Self::Open { .. }
                | Self::GetDraft { .. }
        )
    }
}

/// A remote workspace response produced by the Host's workspace service.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkspaceResponse {
    /// Returns the Host environment for project selection and terminal defaults.
    Environment(WorkspaceEnvironment),
    Workspaces(Vec<WorkspaceSummary>),
    Registered(WorkspaceSummary),
    /// Returns one bounded Host directory listing.
    AgentSessions(Vec<AgentSessionSummary>),
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
    /// Returns a durable workspace snapshot and its monotonic revision.
    Opened {
        snapshot: WorkspaceSnapshot,
        revision: u64,
        drafts: Vec<DraftRef>,
    },
    /// Confirms a persisted snapshot commit and its resulting revision.
    Committed {
        workspace_id: WorkspaceId,
        revision: u64,
        snapshot: WorkspaceSnapshot,
    },
    DraftStored {
        reference: DraftRef,
    },
    Draft {
        reference: DraftRef,
        #[serde(with = "serde_bytes")]
        content: Vec<u8>,
    },
}

/// Validation failures raised while constructing strongly validated workspace wire values.
#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum WorkspaceValidationError {
    /// The supplied workspace or operation identifier is not a bounded safe identifier.
    #[error("{kind} must be 1 to {maximum} ASCII letters, digits, `_`, or `-`")]
    InvalidIdentifier { kind: &'static str, maximum: usize },
    /// The supplied UI snapshot is not a JSON object.
    #[error("workspace snapshot must be a JSON object")]
    SnapshotMustBeObject,
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
