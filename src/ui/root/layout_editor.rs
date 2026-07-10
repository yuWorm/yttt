use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::{model::ids::ProjectId, ui::editor::CodeEditorState};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProjectLayoutEditorFormat {
    ProjectConfig,
    PersonalPatch,
    PersonalReplace,
    InvalidPersonal,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutEditorTarget {
    Default,
    Project {
        project_id: ProjectId,
        path: PathBuf,
        format: ProjectLayoutEditorFormat,
    },
}

impl LayoutEditorTarget {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Project { format, .. } => match format {
                ProjectLayoutEditorFormat::ProjectConfig => "project_config",
                ProjectLayoutEditorFormat::PersonalPatch => "personal_patch",
                ProjectLayoutEditorFormat::PersonalReplace => "personal_replace",
                ProjectLayoutEditorFormat::InvalidPersonal => "invalid_personal",
            },
        }
    }

    pub fn input_scope_id(&self) -> &'static str {
        match self {
            Self::Default => "editor.default_layout",
            Self::Project { .. } => "editor.project_layout",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LayoutEditorSession {
    target: LayoutEditorTarget,
    editor: CodeEditorState,
}

impl LayoutEditorSession {
    pub fn new(target: LayoutEditorTarget, editor: CodeEditorState) -> Self {
        Self { target, editor }
    }

    pub fn target(&self) -> &LayoutEditorTarget {
        &self.target
    }

    pub fn editor(&self) -> &CodeEditorState {
        &self.editor
    }

    pub fn editor_mut(&mut self) -> &mut CodeEditorState {
        &mut self.editor
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayoutEditorWriteError {
    #[error("failed to create layout directory {path}: {message}")]
    CreateDirectory { path: PathBuf, message: String },
    #[error("failed to write layout file {path}: {message}")]
    Write { path: PathBuf, message: String },
    #[error("failed to replace layout file {path}: {message}")]
    Rename { path: PathBuf, message: String },
}

pub(crate) fn write_layout_file_atomic(
    path: &Path,
    source: &str,
) -> Result<(), LayoutEditorWriteError> {
    write_layout_file_atomic_with_file_system(path, source, &StdLayoutEditorFileSystem)
}

fn write_layout_file_atomic_with_file_system(
    path: &Path,
    source: &str,
    file_system: &dyn LayoutEditorFileSystem,
) -> Result<(), LayoutEditorWriteError> {
    if let Some(parent) = path.parent() {
        file_system.create_dir_all(parent).map_err(|error| {
            LayoutEditorWriteError::CreateDirectory {
                path: parent.to_path_buf(),
                message: error.to_string(),
            }
        })?;
    }
    let temp_path = atomic_temp_path(path);
    file_system
        .write(&temp_path, source)
        .map_err(|error| LayoutEditorWriteError::Write {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    file_system
        .sync(&temp_path)
        .map_err(|error| LayoutEditorWriteError::Write {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    file_system
        .rename(&temp_path, path)
        .map_err(|error| LayoutEditorWriteError::Rename {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

fn atomic_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("layout.toml");
    path.with_file_name(format!(".{file_name}.tmp"))
}

trait LayoutEditorFileSystem {
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn write(&self, path: &Path, source: &str) -> io::Result<()>;
    fn sync(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
}

struct StdLayoutEditorFileSystem;

impl LayoutEditorFileSystem for StdLayoutEditorFileSystem {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn write(&self, path: &Path, source: &str) -> io::Result<()> {
        fs::write(path, source)
    }

    fn sync(&self, path: &Path) -> io::Result<()> {
        fs::OpenOptions::new().read(true).open(path)?.sync_all()
    }

    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        fs::rename(from, to)
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cell::RefCell,
        collections::HashMap,
        io,
        path::{Path, PathBuf},
    };

    use super::*;

    #[derive(Default)]
    struct FakeLayoutEditorFileSystem {
        files: RefCell<HashMap<PathBuf, String>>,
        fail_rename: RefCell<bool>,
    }

    impl LayoutEditorFileSystem for FakeLayoutEditorFileSystem {
        fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn write(&self, path: &Path, source: &str) -> io::Result<()> {
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), source.to_string());
            Ok(())
        }

        fn sync(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            if *self.fail_rename.borrow() {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    "rename denied",
                ));
            }
            let source = self.files.borrow_mut().remove(from).unwrap();
            self.files.borrow_mut().insert(to.to_path_buf(), source);
            Ok(())
        }
    }

    #[test]
    fn atomic_layout_editor_write_rename_failure_preserves_target() {
        let path = Path::new("/config/layout.toml");
        let file_system = FakeLayoutEditorFileSystem::default();
        file_system
            .files
            .borrow_mut()
            .insert(path.to_path_buf(), "old".to_string());
        *file_system.fail_rename.borrow_mut() = true;

        let error =
            write_layout_file_atomic_with_file_system(path, "new", &file_system).unwrap_err();

        assert_eq!(file_system.files.borrow().get(path).unwrap(), "old");
        assert!(matches!(
            error,
            LayoutEditorWriteError::Rename {
                path: error_path,
                message,
            } if error_path == path && message == "rename denied"
        ));
    }
}
