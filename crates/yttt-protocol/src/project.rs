use serde::{Deserialize, Serialize};
use yttt_core::model::ids::ProjectId;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformPath {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformArgument {
    Unix(Vec<u8>),
    Windows(Vec<u16>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFileFingerprint {
    pub exists: bool,
    pub byte_len: u64,
    pub modified_nanos: Option<u128>,
    pub content_hash: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectFileState {
    Missing,
    Present(ProjectFileFingerprint),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFileContent {
    pub canonical_path: PlatformPath,
    pub relative_path: PlatformPath,
    pub text: String,
    pub fingerprint: ProjectFileFingerprint,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectSaveMode {
    Check(ProjectFileFingerprint),
    Force,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectSaveResult {
    Saved(ProjectFileFingerprint),
    Conflict(ProjectFileState),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectEntryKind {
    Directory,
    File,
    SymlinkFile,
    SymlinkDirectory,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntry {
    pub name: PlatformArgument,
    pub relative_path: PlatformPath,
    pub kind: ProjectEntryKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDirectory {
    pub relative_directory: PlatformPath,
    pub entries: Vec<ProjectEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntryMutation {
    pub relative_path: PlatformPath,
    pub kind: ProjectEntryKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectPasteMode {
    Copy,
    Cut,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectGitOutput {
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectChange {
    pub project_id: ProjectId,
    pub registration_epoch: u64,
    pub relative_paths: Vec<PlatformPath>,
    pub refresh_status: bool,
    pub refresh_tree: bool,
    pub refresh_all: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectRequest {
    Register {
        project_id: ProjectId,
        root: PlatformPath,
    },
    RegisterSsh {
        project_id: ProjectId,
        connection_id: String,
        root: String,
    },
    Close {
        project_id: ProjectId,
        registration_epoch: u64,
    },
    ScanDirectory {
        project_id: ProjectId,
        relative_directory: PlatformPath,
        show_hidden: bool,
    },
    ReadFile {
        project_id: ProjectId,
        relative_path: PlatformPath,
    },
    SaveFile {
        project_id: ProjectId,
        relative_path: PlatformPath,
        text: String,
        mode: ProjectSaveMode,
    },
    CreateEntry {
        project_id: ProjectId,
        relative_parent: PlatformPath,
        input: String,
    },
    RenameEntry {
        project_id: ProjectId,
        relative_path: PlatformPath,
        new_name: String,
    },
    DeleteEntry {
        project_id: ProjectId,
        relative_path: PlatformPath,
    },
    PasteEntry {
        source_project_id: ProjectId,
        source_relative_path: PlatformPath,
        destination_project_id: ProjectId,
        destination_relative_directory: PlatformPath,
        mode: ProjectPasteMode,
    },
    Git {
        project_id: ProjectId,
        args: Vec<PlatformArgument>,
        optional_locks: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectResponse {
    Registered {
        canonical_root: PlatformPath,
        registration_epoch: u64,
        watch_error: Option<String>,
        null_device: String,
    },
    Closed,
    Directory(ProjectDirectory),
    File(ProjectFileContent),
    Save(ProjectSaveResult),
    Mutation(ProjectEntryMutation),
    Deleted,
    Git(ProjectGitOutput),
}
