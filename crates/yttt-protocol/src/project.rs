use serde::{Deserialize, Serialize};
use yttt_core::model::ids::ProjectId;

use crate::path::{HostPath, PathSegment, ProjectRelativePath};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformPath {
    Unix(#[serde(with = "serde_bytes")] Vec<u8>),
    Windows(Vec<u16>),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum PlatformArgument {
    Unix(#[serde(with = "serde_bytes")] Vec<u8>),
    Windows(Vec<u16>),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRevision {
    pub workspace_epoch: u64,
    pub revision_number: u64,
    pub content_sha256: [u8; 32],
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFileFingerprint {
    pub exists: bool,
    pub byte_len: u64,
    pub modified_nanos: Option<u128>,
    pub content_hash: u64,
    pub revision: ContentRevision,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectFileState {
    Missing,
    Present(ProjectFileFingerprint),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectFileContent {
    pub relative_path: ProjectRelativePath,
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
    pub name: PathSegment,
    pub relative_path: ProjectRelativePath,
    pub kind: ProjectEntryKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectDirectory {
    pub relative_directory: ProjectRelativePath,
    pub entries: Vec<ProjectEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectEntryMutation {
    pub relative_path: ProjectRelativePath,
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
    #[serde(with = "serde_bytes")]
    pub stdout: Vec<u8>,
    #[serde(with = "serde_bytes")]
    pub stderr: Vec<u8>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GitAccess {
    Read,
    Mutate,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum GitOperationError {
    #[error("git reference is not allowed: {0}")]
    InvalidReference(String),
    #[error("git path is not valid UTF-8")]
    PathNotUtf8,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectGitOperation {
    Status {
        work_tree: Option<ProjectRelativePath>,
    },
    Diff {
        staged: bool,
        ignore_whitespace: bool,
    },
    DiffUntracked {
        relative_path: ProjectRelativePath,
        ignore_whitespace: bool,
    },
    ListFiles {
        cached: bool,
        others: bool,
        exclude_standard: bool,
    },
    ListRefs {
        reference: String,
    },
    Switch {
        name: String,
        track_remote: bool,
    },
    Init {
        quiet: bool,
        work_tree: Option<ProjectRelativePath>,
    },
}

impl ProjectGitOperation {
    pub fn access(&self) -> GitAccess {
        match self {
            Self::Switch { .. } | Self::Init { .. } => GitAccess::Mutate,
            Self::Status { .. }
            | Self::Diff { .. }
            | Self::DiffUntracked { .. }
            | Self::ListFiles { .. }
            | Self::ListRefs { .. } => GitAccess::Read,
        }
    }

    pub fn optional_locks(&self) -> bool {
        matches!(self, Self::Status { .. })
    }

    pub fn work_tree(&self) -> Option<&ProjectRelativePath> {
        match self {
            Self::Status { work_tree } | Self::Init { work_tree, .. } => work_tree.as_ref(),
            Self::Diff { .. }
            | Self::DiffUntracked { .. }
            | Self::ListFiles { .. }
            | Self::ListRefs { .. }
            | Self::Switch { .. } => None,
        }
    }

    pub fn argv(&self, null_device: &str) -> Result<Vec<String>, GitOperationError> {
        let args = match self {
            Self::Status { .. } => vec![
                "status".to_string(),
                "--porcelain=v1".to_string(),
                "-b".to_string(),
                "--ignored=matching".to_string(),
            ],
            Self::Diff {
                staged,
                ignore_whitespace,
            } => {
                let mut args = vec!["diff".to_string()];
                if *staged {
                    args.push("--cached".to_string());
                }
                args.extend(["--no-ext-diff".to_string(), "--no-color".to_string()]);
                if *ignore_whitespace {
                    args.push("-w".to_string());
                }
                args.push("--".to_string());
                args
            }
            Self::DiffUntracked {
                relative_path,
                ignore_whitespace,
            } => {
                let path = relative_path
                    .to_utf8()
                    .map_err(|_| GitOperationError::PathNotUtf8)?;
                let mut args = vec![
                    "diff".to_string(),
                    "--no-index".to_string(),
                    "--no-ext-diff".to_string(),
                    "--no-color".to_string(),
                ];
                if *ignore_whitespace {
                    args.push("-w".to_string());
                }
                args.extend(["--".to_string(), null_device.to_string(), path]);
                args
            }
            Self::ListFiles {
                cached,
                others,
                exclude_standard,
            } => {
                let mut args = vec!["ls-files".to_string()];
                if *cached {
                    args.push("--cached".to_string());
                }
                if *others {
                    args.push("--others".to_string());
                }
                if *exclude_standard {
                    args.push("--exclude-standard".to_string());
                }
                args.extend(["-z".to_string(), "--".to_string()]);
                args
            }
            Self::ListRefs { reference } => {
                if !is_allowed_git_ref_prefix(reference) {
                    return Err(GitOperationError::InvalidReference(reference.clone()));
                }
                vec![
                    "for-each-ref".to_string(),
                    "--format=%(refname:short)\t%(HEAD)".to_string(),
                    reference.clone(),
                ]
            }
            Self::Switch { name, track_remote } => {
                if !is_safe_git_ref_name(name) {
                    return Err(GitOperationError::InvalidReference(name.clone()));
                }
                let mut args = vec!["switch".to_string()];
                if *track_remote {
                    args.push("--track".to_string());
                }
                args.extend(["--".to_string(), name.clone()]);
                args
            }
            Self::Init { quiet, .. } => {
                let mut args = vec!["init".to_string()];
                if *quiet {
                    args.push("--quiet".to_string());
                }
                args
            }
        };
        debug_assert!(
            git_argv_is_safe(&args),
            "structured Git operations must never emit unsafe git flags"
        );
        Ok(args)
    }
}

pub fn is_allowed_git_ref_prefix(reference: &str) -> bool {
    matches!(reference, "refs/heads" | "refs/remotes")
}

pub fn is_safe_git_ref_name(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with('-')
        && !name.ends_with('.')
        && !name.ends_with('/')
        && !name.contains("..")
        && !name.contains("@{")
        && !name.contains("//")
        && !name
            .chars()
            .any(|character| character.is_control() || " ~^:?*[\\]".contains(character))
}

pub fn git_argv_is_safe(args: &[String]) -> bool {
    // Everything after `--` is an operand git never parses as an option, so a tracked
    // file legitimately named `-cache.txt` must not be mistaken for `-c`.
    !args
        .iter()
        .take_while(|arg| arg.as_str() != "--")
        .any(|arg| {
            is_disallowed_git_flag(arg)
                || arg == "--exec-path"
                || arg.starts_with("--exec-path=")
                || arg == "--git-dir"
                || arg.starts_with("--git-dir=")
                || arg == "--work-tree"
                || arg.starts_with("--work-tree=")
                || arg == "--namespace"
                || arg.starts_with("--namespace=")
                || arg == "--config"
                || arg.starts_with("--config=")
        })
}

fn is_disallowed_git_flag(arg: &str) -> bool {
    (arg == "-c" || (arg.starts_with("-c") && !arg.starts_with("--")))
        || (arg == "-C" || (arg.starts_with("-C") && !arg.starts_with("--")))
}

#[cfg(test)]
mod git_allowlist_tests {
    use super::*;

    #[test]
    fn structured_operations_never_emit_config_or_directory_overrides() {
        let operations = [
            ProjectGitOperation::Status { work_tree: None },
            ProjectGitOperation::Diff {
                staged: true,
                ignore_whitespace: true,
            },
            ProjectGitOperation::DiffUntracked {
                relative_path: ProjectRelativePath::from_utf8("new.txt").unwrap(),
                ignore_whitespace: false,
            },
            ProjectGitOperation::ListFiles {
                cached: true,
                others: true,
                exclude_standard: true,
            },
            ProjectGitOperation::ListRefs {
                reference: "refs/heads".to_string(),
            },
            ProjectGitOperation::Switch {
                name: "feature".to_string(),
                track_remote: true,
            },
            ProjectGitOperation::Init {
                quiet: true,
                work_tree: Some(ProjectRelativePath::from_utf8("repo").unwrap()),
            },
        ];
        for operation in operations {
            let args = operation.argv("/dev/null").unwrap();
            assert!(git_argv_is_safe(&args), "{args:?}");
            assert!(!args.iter().any(|arg| arg == "-c" || arg == "-C"));
        }
    }

    #[test]
    fn git_config_and_alias_injection_are_rejected() {
        assert!(!git_argv_is_safe(&[
            "-c".to_string(),
            "alias.x=!touch pwned".to_string(),
            "x".to_string(),
        ]));
        assert!(!git_argv_is_safe(&["-ccore.sshCommand=id".to_string()]));
        assert!(!git_argv_is_safe(&[
            "-C".to_string(),
            "/tmp".to_string(),
            "status".to_string(),
        ]));
        assert!(!git_argv_is_safe(&["--git-dir=/tmp/other".to_string()]));
        assert!(!git_argv_is_safe(&[
            "--work-tree".to_string(),
            "/tmp".to_string(),
        ]));
        assert!(!git_argv_is_safe(&["--exec-path=/tmp/bin".to_string()]));
        assert!(git_argv_is_safe(&[
            "diff".to_string(),
            "--cached".to_string(),
            "--no-ext-diff".to_string(),
            "--".to_string(),
        ]));
    }

    #[test]
    fn operands_after_the_separator_are_not_treated_as_flags() {
        let args = ProjectGitOperation::DiffUntracked {
            relative_path: ProjectRelativePath::from_utf8("-cache.txt").unwrap(),
            ignore_whitespace: false,
        }
        .argv("/dev/null")
        .unwrap();
        assert!(git_argv_is_safe(&args), "{args:?}");
        assert!(!git_argv_is_safe(&[
            "-c".to_string(),
            "alias.x=!touch pwned".to_string(),
            "--".to_string(),
            "safe.txt".to_string(),
        ]));
    }

    #[test]
    fn switch_and_init_are_mutate_and_status_is_read() {
        assert_eq!(
            ProjectGitOperation::Switch {
                name: "main".to_string(),
                track_remote: false,
            }
            .access(),
            GitAccess::Mutate
        );
        assert_eq!(
            ProjectGitOperation::Init {
                quiet: true,
                work_tree: None,
            }
            .access(),
            GitAccess::Mutate
        );
        assert_eq!(
            ProjectGitOperation::Status { work_tree: None }.access(),
            GitAccess::Read
        );
        assert!(ProjectGitOperation::Status { work_tree: None }.optional_locks());
    }

    #[test]
    fn list_refs_rejects_arbitrary_prefixes() {
        let error = ProjectGitOperation::ListRefs {
            reference: "refs/replace".to_string(),
        }
        .argv("/dev/null")
        .unwrap_err();
        assert!(matches!(error, GitOperationError::InvalidReference(_)));
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectChange {
    pub project_id: ProjectId,
    pub registration_epoch: u64,
    pub relative_paths: Vec<ProjectRelativePath>,
    pub refresh_status: bool,
    pub refresh_tree: bool,
    pub refresh_all: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectRequest {
    Register {
        view_id: String,
        project_id: ProjectId,
        root: HostPath,
    },
    RegisterSsh {
        view_id: String,
        project_id: ProjectId,
        connection_id: String,
        root: ProjectRelativePath,
    },
    Close {
        view_id: String,
        project_id: ProjectId,
        registration_epoch: u64,
    },
    Observe {
        project_id: ProjectId,
        view_id: String,
    },
    ScanDirectory {
        project_id: ProjectId,
        relative_directory: ProjectRelativePath,
        show_hidden: bool,
    },
    ReadFile {
        project_id: ProjectId,
        relative_path: ProjectRelativePath,
    },
    SaveFile {
        project_id: ProjectId,
        relative_path: ProjectRelativePath,
        text: String,
        mode: ProjectSaveMode,
    },
    CreateEntry {
        project_id: ProjectId,
        relative_parent: ProjectRelativePath,
        input: String,
    },
    RenameEntry {
        project_id: ProjectId,
        relative_path: ProjectRelativePath,
        new_name: String,
    },
    DeleteEntry {
        project_id: ProjectId,
        relative_path: ProjectRelativePath,
    },
    PasteEntry {
        source_project_id: ProjectId,
        source_relative_path: ProjectRelativePath,
        destination_project_id: ProjectId,
        destination_relative_directory: ProjectRelativePath,
        mode: ProjectPasteMode,
    },
    Git {
        project_id: ProjectId,
        operation: ProjectGitOperation,
    },
}

impl ProjectRequest {
    pub fn required_capability(&self) -> crate::Capability {
        match self {
            Self::ScanDirectory { .. }
            | Self::ReadFile { .. }
            | Self::Observe { .. }
            | Self::Close { .. } => crate::Capability::ProjectRead,
            Self::Git { operation, .. } => match operation.access() {
                GitAccess::Read => crate::Capability::GitRead,
                GitAccess::Mutate => crate::Capability::GitMutate,
            },
            Self::Register { .. }
            | Self::RegisterSsh { .. }
            | Self::SaveFile { .. }
            | Self::CreateEntry { .. }
            | Self::RenameEntry { .. }
            | Self::DeleteEntry { .. }
            | Self::PasteEntry { .. } => crate::Capability::ProjectMutate,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProjectResponse {
    Registered {
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
