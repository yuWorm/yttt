use std::{
    io,
    path::{Path, PathBuf},
};

use crate::config::paths::AppConfigPaths;
pub use yttt_layout::template::{
    BuiltinAgent, DefaultLayoutKind, DefaultLayoutTemplate, DefaultProjectTemplate,
};
use yttt_layout::template::{TemplateError, parse_template};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultLayoutSource {
    BuiltIn,
    ConfigFile(PathBuf),
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum LayoutLoadWarning {
    #[error("failed to create global default layout directory {path}: {message}")]
    GlobalDefaultCreate { path: PathBuf, message: String },
    #[error("failed to read global default layout {path}: {message}")]
    GlobalDefaultRead { path: PathBuf, message: String },
    #[error("failed to parse global default layout {path}: {message}")]
    GlobalDefaultParse { path: PathBuf, message: String },
    #[error("invalid global default layout {path}: {message}")]
    GlobalDefaultValidation { path: PathBuf, message: String },
    #[error("failed to write global default layout {path}: {message}")]
    GlobalDefaultWrite { path: PathBuf, message: String },
    #[error("failed to read personal layout {path}: {message}")]
    PersonalOverrideRead { path: PathBuf, message: String },
    #[error("failed to parse personal layout {path}: {message}")]
    PersonalOverrideParse { path: PathBuf, message: String },
    #[error("invalid personal layout {path}: {message}")]
    PersonalOverrideValidation { path: PathBuf, message: String },
    #[error("stale personal tab override {tab_id} in {path}")]
    StaleOverrideTab { path: PathBuf, tab_id: String },
    #[error("stale personal pane override {pane_id} in {path}")]
    StaleOverridePane { path: PathBuf, pane_id: String },
}

#[derive(Clone, Debug, PartialEq)]
pub struct DefaultLayoutState {
    template: DefaultLayoutTemplate,
    source: DefaultLayoutSource,
    warnings: Vec<LayoutLoadWarning>,
    path: PathBuf,
}

impl DefaultLayoutState {
    pub fn load(paths: &AppConfigPaths) -> Self {
        Self::load_with_file_system(paths, &StdLayoutFileSystem)
    }

    pub fn template(&self) -> &DefaultLayoutTemplate {
        &self.template
    }

    pub fn source(&self) -> &DefaultLayoutSource {
        &self.source
    }

    pub fn warnings(&self) -> &[LayoutLoadWarning] {
        &self.warnings
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn reload(&mut self) -> Result<(), LayoutLoadWarning> {
        self.reload_with_file_system(&StdLayoutFileSystem)
    }

    pub fn save(&mut self, template: DefaultLayoutTemplate) -> Result<(), LayoutLoadWarning> {
        self.save_with_file_system(template, &StdLayoutFileSystem)
    }

    pub fn reset(&mut self) -> Result<(), LayoutLoadWarning> {
        self.reset_with_file_system(&StdLayoutFileSystem)
    }

    fn load_with_file_system(paths: &AppConfigPaths, file_system: &dyn LayoutFileSystem) -> Self {
        let path = paths.default_layout_file();
        let (template, source, warnings) = load_template_or_builtin(file_system, &path);
        Self {
            template,
            source,
            warnings,
            path,
        }
    }

    fn reload_with_file_system(
        &mut self,
        file_system: &dyn LayoutFileSystem,
    ) -> Result<(), LayoutLoadWarning> {
        let (template, source, warnings) = load_template_or_builtin(file_system, &self.path);

        if let Some(warning) = warnings.first().cloned() {
            self.warnings = warnings;
            return Err(warning);
        }

        self.template = template;
        self.source = source;
        self.warnings.clear();
        Ok(())
    }

    fn save_with_file_system(
        &mut self,
        template: DefaultLayoutTemplate,
        file_system: &dyn LayoutFileSystem,
    ) -> Result<(), LayoutLoadWarning> {
        template
            .validate()
            .map_err(|error| LayoutLoadWarning::GlobalDefaultValidation {
                path: self.path.clone(),
                message: error.to_string(),
            })?;
        write_template_atomic(file_system, &self.path, &template)?;
        self.template = template;
        self.source = DefaultLayoutSource::ConfigFile(self.path.clone());
        self.warnings.clear();
        Ok(())
    }

    fn reset_with_file_system(
        &mut self,
        file_system: &dyn LayoutFileSystem,
    ) -> Result<(), LayoutLoadWarning> {
        self.save_with_file_system(DefaultLayoutTemplate::builtin(), file_system)
    }
}

fn load_template_or_builtin(
    file_system: &dyn LayoutFileSystem,
    path: &Path,
) -> (
    DefaultLayoutTemplate,
    DefaultLayoutSource,
    Vec<LayoutLoadWarning>,
) {
    let builtin = DefaultLayoutTemplate::builtin();
    let source = match file_system.read_to_string(path) {
        Ok(source) => source,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return (builtin, DefaultLayoutSource::BuiltIn, Vec::new());
        }
        Err(error) => {
            return (
                builtin,
                DefaultLayoutSource::BuiltIn,
                vec![LayoutLoadWarning::GlobalDefaultRead {
                    path: path.to_path_buf(),
                    message: error.to_string(),
                }],
            );
        }
    };

    let template = match parse_template(&source) {
        Ok(template) => template,
        Err(TemplateError::Parse(message)) => {
            return (
                builtin,
                DefaultLayoutSource::BuiltIn,
                vec![LayoutLoadWarning::GlobalDefaultParse {
                    path: path.to_path_buf(),
                    message,
                }],
            );
        }
        Err(TemplateError::Validation(message)) => {
            return (
                builtin,
                DefaultLayoutSource::BuiltIn,
                vec![LayoutLoadWarning::GlobalDefaultValidation {
                    path: path.to_path_buf(),
                    message,
                }],
            );
        }
    };

    (
        template,
        DefaultLayoutSource::ConfigFile(path.to_path_buf()),
        Vec::new(),
    )
}

fn write_template_atomic(
    file_system: &dyn LayoutFileSystem,
    path: &Path,
    template: &DefaultLayoutTemplate,
) -> Result<(), LayoutLoadWarning> {
    let source = toml::to_string_pretty(template).map_err(|error| {
        LayoutLoadWarning::GlobalDefaultWrite {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    if let Some(parent) = path.parent() {
        file_system.create_dir_all(parent).map_err(|error| {
            LayoutLoadWarning::GlobalDefaultCreate {
                path: parent.to_path_buf(),
                message: error.to_string(),
            }
        })?;
    }

    file_system
        .write_atomic(path, &source)
        .map_err(|error| LayoutLoadWarning::GlobalDefaultWrite {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

trait LayoutFileSystem {
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn write_atomic(&self, path: &Path, source: &str) -> io::Result<()>;
}

struct StdLayoutFileSystem;

impl LayoutFileSystem for StdLayoutFileSystem {
    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        crate::config::storage::create_dir_all(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        crate::config::storage::read_to_string(path)
    }

    fn write_atomic(&self, path: &Path, source: &str) -> io::Result<()> {
        crate::config::atomic_write(path, source.as_bytes())
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
    use crate::config::paths::AppConfigPaths;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Operation {
        Read,
        Write,
    }

    #[derive(Default)]
    struct FakeFileSystem {
        files: RefCell<HashMap<PathBuf, String>>,
        failure: RefCell<Option<(Operation, String)>>,
    }

    impl FakeFileSystem {
        fn fail(&self, operation: Operation, message: &str) {
            *self.failure.borrow_mut() = Some((operation, message.to_string()));
        }

        fn take_failure(&self, operation: Operation) -> io::Result<()> {
            let mut failure = self.failure.borrow_mut();
            if failure.as_ref().is_some_and(|(kind, _)| *kind == operation) {
                let (_, message) = failure.take().unwrap();
                Err(io::Error::new(io::ErrorKind::PermissionDenied, message))
            } else {
                Ok(())
            }
        }

        fn source(&self, path: &Path) -> Option<String> {
            self.files.borrow().get(path).cloned()
        }
    }

    impl LayoutFileSystem for FakeFileSystem {
        fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
            Ok(())
        }

        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.take_failure(Operation::Read)?;
            self.source(path)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing file"))
        }

        fn write_atomic(&self, path: &Path, source: &str) -> io::Result<()> {
            self.take_failure(Operation::Write)?;
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), source.to_string());
            Ok(())
        }
    }

    #[test]
    fn default_layout_state_missing_file_uses_builtin_without_writing() {
        let paths = AppConfigPaths::from_config_dir("/config");
        let fs = FakeFileSystem::default();

        let state = DefaultLayoutState::load_with_file_system(&paths, &fs);

        assert_eq!(state.template(), &DefaultLayoutTemplate::builtin());
        assert_eq!(state.source(), &DefaultLayoutSource::BuiltIn);
        assert!(state.warnings().is_empty());
        assert!(fs.source(&paths.default_layout_file()).is_none());
    }

    #[test]
    fn default_layout_state_read_failure_is_not_treated_as_missing() {
        let paths = AppConfigPaths::from_config_dir("/config");
        let fs = FakeFileSystem::default();
        fs.fail(Operation::Read, "read denied");

        let state = DefaultLayoutState::load_with_file_system(&paths, &fs);

        assert_eq!(state.template(), &DefaultLayoutTemplate::builtin());
        assert!(matches!(
            state.warnings(),
            [LayoutLoadWarning::GlobalDefaultRead { .. }]
        ));
        assert!(fs.source(&paths.default_layout_file()).is_none());
    }

    #[test]
    fn default_layout_state_write_failure_preserves_file_and_cache() {
        let paths = AppConfigPaths::from_config_dir("/config");
        let fs = FakeFileSystem::default();
        let mut state = DefaultLayoutState::load_with_file_system(&paths, &fs);
        let original_source = fs.source(&paths.default_layout_file());
        let original_state = state.clone();
        let mut updated = DefaultLayoutTemplate::builtin();
        updated.tabs[0].title = "Updated".to_string();
        fs.fail(Operation::Write, "write denied");

        let error = state.save_with_file_system(updated, &fs).unwrap_err();

        assert_eq!(state, original_state);
        assert_eq!(fs.source(&paths.default_layout_file()), original_source);
        assert!(matches!(
            error,
            LayoutLoadWarning::GlobalDefaultWrite { .. }
        ));
    }

    #[test]
    fn default_layout_state_reset_write_failure_preserves_file_and_cache() {
        let paths = AppConfigPaths::from_config_dir("/config");
        let fs = FakeFileSystem::default();
        let mut state = DefaultLayoutState::load_with_file_system(&paths, &fs);
        let mut changed = DefaultLayoutTemplate::builtin();
        changed.tabs[0].title = "Changed".to_string();
        state.save_with_file_system(changed, &fs).unwrap();
        let original_source = fs.source(&paths.default_layout_file()).unwrap();
        let original_state = state.clone();
        fs.fail(Operation::Write, "write denied");

        let error = state.reset_with_file_system(&fs).unwrap_err();

        assert_eq!(state, original_state);
        assert_eq!(
            fs.source(&paths.default_layout_file()).unwrap(),
            original_source
        );
        assert!(matches!(
            error,
            LayoutLoadWarning::GlobalDefaultWrite { .. }
        ));
    }
}
