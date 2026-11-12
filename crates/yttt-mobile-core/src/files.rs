//! The right drawer's data: a project's files a directory at a time, one
//! file's text to edit and save, and the Agent sessions recorded for it.

use yttt_protocol::project::ProjectEntryKind;

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct FileEntry {
    pub name: String,
    /// Relative to the project root, with `/` between segments.
    pub path: String,
    pub kind: FileKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum FileKind {
    Directory,
    File,
    SymlinkFile,
    SymlinkDirectory,
}

impl FileKind {
    pub fn is_directory(self) -> bool {
        matches!(self, Self::Directory | Self::SymlinkDirectory)
    }
}

impl From<ProjectEntryKind> for FileKind {
    fn from(kind: ProjectEntryKind) -> Self {
        match kind {
            ProjectEntryKind::Directory => Self::Directory,
            ProjectEntryKind::File => Self::File,
            ProjectEntryKind::SymlinkFile => Self::SymlinkFile,
            ProjectEntryKind::SymlinkDirectory => Self::SymlinkDirectory,
        }
    }
}

/// A file opened to edit. The core keeps its fingerprint to save against.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct OpenedFile {
    pub path: String,
    pub text: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum SaveOutcome {
    Saved,
    /// The file changed on the Host since it was opened or saved: the user
    /// overwrites it (save with `force`) or reloads it (open it again).
    Conflict,
}

/// An Agent session the Host found for a project.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct SessionEntry {
    pub provider: String,
    pub id: String,
    pub title: String,
    pub model: Option<String>,
    pub updated_at_ms: u64,
}

/// The providers whose histories the desktop lists.
pub const SESSION_PROVIDERS: [&str; 5] = ["codex", "claude", "grok", "pi", "omp"];
