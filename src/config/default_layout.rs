use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::config::paths::AppConfigPaths;
use crate::model::layout::{
    LayoutError, LayoutNode, PaneConfig, PaneKind, ProjectConfig, ProjectLayout, TabConfig,
};

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
    #[error("failed to replace global default layout {path}: {message}")]
    GlobalDefaultRename { path: PathBuf, message: String },
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

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DefaultLayoutTemplate {
    pub project: DefaultProjectTemplate,
    #[serde(default)]
    pub tabs: Vec<TabConfig>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct DefaultProjectTemplate {
    pub default_tab: Option<String>,
}

impl DefaultLayoutTemplate {
    pub fn builtin() -> Self {
        Self {
            project: DefaultProjectTemplate {
                default_tab: Some("shell".to_string()),
            },
            tabs: vec![TabConfig {
                id: "shell".to_string(),
                title: "Shell".to_string(),
                layout: LayoutNode::Pane(PaneConfig {
                    id: "shell".to_string(),
                    title: "Shell".to_string(),
                    command: "$SHELL".to_string(),
                    kind: PaneKind::Shell,
                    notify_on_exit: false,
                    detector: None,
                }),
            }],
        }
    }

    pub fn materialize(&self, project_name: impl Into<String>) -> ProjectLayout {
        ProjectLayout {
            project: ProjectConfig {
                name: project_name.into(),
                default_tab: self.project.default_tab.clone(),
            },
            tabs: self.tabs.clone(),
        }
    }

    pub fn validate(&self) -> Result<(), LayoutError> {
        self.materialize("Project").validate()
    }
}

impl DefaultLayoutState {
    pub fn load_or_create(paths: &AppConfigPaths) -> Self {
        Self::load_or_create_with_file_system(paths, &StdLayoutFileSystem)
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

    fn load_or_create_with_file_system(
        paths: &AppConfigPaths,
        file_system: &dyn LayoutFileSystem,
    ) -> Self {
        let path = paths.default_layout_file();
        let builtin = DefaultLayoutTemplate::builtin();

        if !file_system.exists(&path) {
            return match write_template_atomic(file_system, &path, &builtin) {
                Ok(()) => Self {
                    template: builtin,
                    source: DefaultLayoutSource::ConfigFile(path.clone()),
                    warnings: Vec::new(),
                    path,
                },
                Err(warning) => Self {
                    template: builtin,
                    source: DefaultLayoutSource::BuiltIn,
                    warnings: vec![warning],
                    path,
                },
            };
        }

        match read_template(file_system, &path) {
            Ok(template) => Self {
                template,
                source: DefaultLayoutSource::ConfigFile(path.clone()),
                warnings: Vec::new(),
                path,
            },
            Err(warning) => Self {
                template: builtin,
                source: DefaultLayoutSource::BuiltIn,
                warnings: vec![warning],
                path,
            },
        }
    }

    fn reload_with_file_system(
        &mut self,
        file_system: &dyn LayoutFileSystem,
    ) -> Result<(), LayoutLoadWarning> {
        let result = if file_system.exists(&self.path) {
            read_template(file_system, &self.path)
        } else {
            let builtin = DefaultLayoutTemplate::builtin();
            write_template_atomic(file_system, &self.path, &builtin).map(|()| builtin)
        };

        match result {
            Ok(template) => {
                self.template = template;
                self.source = DefaultLayoutSource::ConfigFile(self.path.clone());
                self.warnings.clear();
                Ok(())
            }
            Err(warning) => {
                self.warnings = vec![warning.clone()];
                Err(warning)
            }
        }
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

fn read_template(
    file_system: &dyn LayoutFileSystem,
    path: &Path,
) -> Result<DefaultLayoutTemplate, LayoutLoadWarning> {
    let source =
        file_system
            .read_to_string(path)
            .map_err(|error| LayoutLoadWarning::GlobalDefaultRead {
                path: path.to_path_buf(),
                message: error.to_string(),
            })?;
    let template: DefaultLayoutTemplate =
        toml::from_str(&source).map_err(|error| LayoutLoadWarning::GlobalDefaultParse {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    template
        .validate()
        .map_err(|error| LayoutLoadWarning::GlobalDefaultValidation {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    Ok(template)
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

    let temp_path = atomic_temp_path(path);
    file_system.write(&temp_path, &source).map_err(|error| {
        LayoutLoadWarning::GlobalDefaultWrite {
            path: path.to_path_buf(),
            message: error.to_string(),
        }
    })?;
    file_system
        .sync(&temp_path)
        .map_err(|error| LayoutLoadWarning::GlobalDefaultWrite {
            path: path.to_path_buf(),
            message: error.to_string(),
        })?;
    file_system
        .rename(&temp_path, path)
        .map_err(|error| LayoutLoadWarning::GlobalDefaultRename {
            path: path.to_path_buf(),
            message: error.to_string(),
        })
}

fn atomic_temp_path(path: &Path) -> PathBuf {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("default-layout.toml");
    path.with_file_name(format!(".{file_name}.tmp"))
}

trait LayoutFileSystem {
    fn exists(&self, path: &Path) -> bool;
    fn create_dir_all(&self, path: &Path) -> io::Result<()>;
    fn read_to_string(&self, path: &Path) -> io::Result<String>;
    fn write(&self, path: &Path, source: &str) -> io::Result<()>;
    fn sync(&self, path: &Path) -> io::Result<()>;
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
}

struct StdLayoutFileSystem;

impl LayoutFileSystem for StdLayoutFileSystem {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn create_dir_all(&self, path: &Path) -> io::Result<()> {
        fs::create_dir_all(path)
    }

    fn read_to_string(&self, path: &Path) -> io::Result<String> {
        fs::read_to_string(path)
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
    use crate::config::paths::AppConfigPaths;

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    enum Operation {
        CreateDirectory,
        Read,
        Write,
        Sync,
        Rename,
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
        fn exists(&self, path: &Path) -> bool {
            self.files.borrow().contains_key(path)
        }

        fn create_dir_all(&self, _path: &Path) -> io::Result<()> {
            self.take_failure(Operation::CreateDirectory)
        }

        fn read_to_string(&self, path: &Path) -> io::Result<String> {
            self.take_failure(Operation::Read)?;
            self.source(path)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing file"))
        }

        fn write(&self, path: &Path, source: &str) -> io::Result<()> {
            self.take_failure(Operation::Write)?;
            self.files
                .borrow_mut()
                .insert(path.to_path_buf(), source.to_string());
            Ok(())
        }

        fn sync(&self, _path: &Path) -> io::Result<()> {
            self.take_failure(Operation::Sync)
        }

        fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
            self.take_failure(Operation::Rename)?;
            let source = self
                .files
                .borrow_mut()
                .remove(from)
                .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "missing temp file"))?;
            self.files.borrow_mut().insert(to.to_path_buf(), source);
            Ok(())
        }
    }

    #[test]
    fn default_layout_state_create_failure_uses_builtin_with_exact_warning() {
        let paths = AppConfigPaths::from_config_dir("/config");
        let fs = FakeFileSystem::default();
        fs.fail(Operation::CreateDirectory, "create denied");

        let state = DefaultLayoutState::load_or_create_with_file_system(&paths, &fs);

        assert_eq!(state.template(), &DefaultLayoutTemplate::builtin());
        assert_eq!(state.source(), &DefaultLayoutSource::BuiltIn);
        assert_eq!(
            state.warnings(),
            &[LayoutLoadWarning::GlobalDefaultCreate {
                path: paths.config_dir().to_path_buf(),
                message: "create denied".to_string(),
            }]
        );
    }

    #[test]
    fn default_layout_state_rename_failure_preserves_file_and_cache() {
        let paths = AppConfigPaths::from_config_dir("/config");
        let fs = FakeFileSystem::default();
        let mut state = DefaultLayoutState::load_or_create_with_file_system(&paths, &fs);
        let original_source = fs.source(&paths.default_layout_file()).unwrap();
        let original_state = state.clone();
        let mut updated = DefaultLayoutTemplate::builtin();
        updated.tabs[0].title = "Updated".to_string();
        fs.fail(Operation::Rename, "rename denied");

        let error = state.save_with_file_system(updated, &fs).unwrap_err();

        assert_eq!(state, original_state);
        assert_eq!(
            fs.source(&paths.default_layout_file()).unwrap(),
            original_source
        );
        assert_eq!(
            error,
            LayoutLoadWarning::GlobalDefaultRename {
                path: paths.default_layout_file(),
                message: "rename denied".to_string(),
            }
        );
    }

    #[test]
    fn default_layout_state_reset_write_failure_preserves_file_and_cache() {
        let paths = AppConfigPaths::from_config_dir("/config");
        let fs = FakeFileSystem::default();
        let mut state = DefaultLayoutState::load_or_create_with_file_system(&paths, &fs);
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
        assert_eq!(
            error,
            LayoutLoadWarning::GlobalDefaultWrite {
                path: paths.default_layout_file(),
                message: "write denied".to_string(),
            }
        );
    }
}
