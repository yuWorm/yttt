use std::fmt;

use serde::{Deserialize, Serialize};
use yttt_core::model::ids::ProjectId;
use zeroize::Zeroize;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SensitiveBytes(#[serde(with = "serde_bytes")] Vec<u8>);

impl SensitiveBytes {
    pub fn new(bytes: Vec<u8>) -> Self {
        Self(bytes)
    }

    pub fn expose(&self) -> &[u8] {
        &self.0
    }

    pub fn into_inner(mut self) -> Vec<u8> {
        std::mem::take(&mut self.0)
    }
}

impl fmt::Debug for SensitiveBytes {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SensitiveBytes([redacted])")
    }
}

impl Drop for SensitiveBytes {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshEndpoint {
    pub host: String,
    pub port: u16,
    pub username: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredSshCredential {
    pub id: String,
    pub effective_user: String,
    pub private_key_identity: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshAuthentication {
    Auto {
        identity_file: Option<String>,
        passphrase: Option<SensitiveBytes>,
        credential: Option<StoredSshCredential>,
    },
    Agent,
    Password {
        secret: SensitiveBytes,
        save_as: Option<String>,
    },
    StoredPassword(StoredSshCredential),
    PrivateKey {
        path: String,
        passphrase: Option<SensitiveBytes>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnectSpec {
    pub connection_id: String,
    pub endpoint: SshEndpoint,
    pub authentication: SshAuthentication,
    pub reconnect: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HostKeyDecision {
    AcceptOnce,
    AcceptAndStore,
    Reject,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialChallengeKind {
    Password,
    Passphrase {
        key_path: String,
    },
    KeyboardInteractive {
        prompts: Vec<String>,
    },
    HostKey {
        host: String,
        port: u16,
        algorithm: String,
        fingerprint: String,
        previous_fingerprint: Option<String>,
    },
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CredentialChallenge {
    pub challenge_id: u64,
    pub connection_id: String,
    pub kind: CredentialChallengeKind,
    pub attempt: u16,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum CredentialAnswer {
    Secret(SensitiveBytes),
    KeyboardInteractive(Vec<SensitiveBytes>),
    HostKey(HostKeyDecision),
    Cancelled,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum SshConnectionState {
    Disconnected,
    Connecting,
    VerifyingHostKey,
    Authenticating,
    Connected,
    Reconnecting,
    Failed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshConnectionStatus {
    pub connection_id: String,
    pub epoch: u64,
    pub state: SshConnectionState,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteFileKind {
    File,
    Directory,
    SymlinkFile,
    SymlinkDirectory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileEntry {
    pub name: String,
    pub relative_path: String,
    pub kind: RemoteFileKind,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileFingerprint {
    pub byte_len: u64,
    pub modified_seconds: Option<u32>,
    pub content_hash: u64,
    pub revision: crate::project::ContentRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteDirectory {
    pub relative_directory: String,
    pub entries: Vec<RemoteFileEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteFileContent {
    pub relative_path: String,
    #[serde(with = "serde_bytes")]
    pub bytes: Vec<u8>,
    pub fingerprint: RemoteFileFingerprint,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteFileState {
    Missing,
    Present(RemoteFileFingerprint),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteSaveResult {
    Saved(RemoteFileFingerprint),
    Conflict(RemoteFileState),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteEntryMutation {
    pub relative_path: String,
    pub kind: RemoteFileKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteFileRequest {
    ResolveHome {
        connection_id: String,
    },
    BrowseDirectory {
        connection_id: String,
        root: String,
        relative_directory: String,
        show_hidden: bool,
    },
    ScanDirectory {
        project_id: ProjectId,
        relative_directory: String,
        show_hidden: bool,
    },
    Read {
        project_id: ProjectId,
        relative_path: String,
        maximum_bytes: u64,
    },
    Save {
        project_id: ProjectId,
        relative_path: String,
        expected: Option<RemoteFileFingerprint>,
        force: bool,
        maximum_bytes: u64,
        #[serde(with = "serde_bytes")]
        bytes: Vec<u8>,
    },
    Create {
        project_id: ProjectId,
        relative_path: String,
        directory: bool,
    },
    Rename {
        project_id: ProjectId,
        relative_path: String,
        new_name: String,
    },
    Delete {
        project_id: ProjectId,
        relative_path: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteFileResponse {
    Home(String),
    Directory(RemoteDirectory),
    File(RemoteFileContent),
    Save(RemoteSaveResult),
    Mutation(RemoteEntryMutation),
    Deleted,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RemoteHostCommand {
    Git {
        operation: crate::project::ProjectGitOperation,
    },
    Privileged {
        program: String,
        args: Vec<String>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCommandRequest {
    pub project_id: ProjectId,
    pub command: RemoteHostCommand,
}

impl RemoteCommandRequest {
    pub fn required_capability(&self) -> crate::Capability {
        match &self.command {
            RemoteHostCommand::Git { operation } => match operation.access() {
                crate::project::GitAccess::Read => crate::Capability::GitRead,
                crate::project::GitAccess::Mutate => crate::Capability::GitMutate,
            },
            RemoteHostCommand::Privileged { .. } => crate::Capability::RemoteCommandPrivileged,
        }
    }
}

impl RemoteFileRequest {
    pub fn required_capability(&self) -> crate::Capability {
        match self {
            Self::ResolveHome { .. }
            | Self::BrowseDirectory { .. }
            | Self::ScanDirectory { .. }
            | Self::Read { .. } => crate::Capability::ProjectRead,
            Self::Save { .. } | Self::Create { .. } | Self::Rename { .. } | Self::Delete { .. } => {
                crate::Capability::ProjectMutate
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemoteCommandResponse {
    pub exit_status: u32,
    #[serde(with = "serde_bytes")]
    pub stdout: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub stderr: Vec<u8>,
}
