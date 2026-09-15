use std::{
    io,
    path::{Path, PathBuf},
};

use crate::config::{
    atomic_write, paths::AppConfigPaths, scope::supports_project_override,
    settings::EditorSettings, storage,
};

pub const PROJECT_SETTINGS_FILE_NAME: &str = "settings.toml";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEditorSettingKey {
    TabSize,
    AutoDetectLanguage,
    DefaultLanguage,
}

impl ProjectEditorSettingKey {
    pub const ALL: [Self; 3] = [
        Self::TabSize,
        Self::AutoDetectLanguage,
        Self::DefaultLanguage,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TabSize => "editor.tab_size",
            Self::AutoDetectLanguage => "editor.auto_detect_language",
            Self::DefaultLanguage => "editor.default_language",
        }
    }

    pub fn from_str(key: &str) -> Result<Self, ProjectSettingsError> {
        if !supports_project_override(key) {
            return Err(ProjectSettingsError::UnsupportedKey {
                key: key.to_string(),
            });
        }
        match key {
            "editor.tab_size" => Ok(Self::TabSize),
            "editor.auto_detect_language" => Ok(Self::AutoDetectLanguage),
            "editor.default_language" => Ok(Self::DefaultLanguage),
            _ => Err(ProjectSettingsError::UnsupportedKey {
                key: key.to_string(),
            }),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectEditorSettingValue {
    TabSize(usize),
    AutoDetectLanguage(bool),
    DefaultLanguage(String),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectSettingSource {
    Host,
    Project,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct EffectiveProjectEditorSetting {
    pub key: ProjectEditorSettingKey,
    pub value: ProjectEditorSettingValue,
    pub source: ProjectSettingSource,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectEditorSettingsSnapshot {
    pub settings_file: PathBuf,
    pub effective: [EffectiveProjectEditorSetting; 3],
}

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProjectEditorOverrides {
    #[serde(skip_serializing_if = "Option::is_none")]
    tab_size: Option<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    auto_detect_language: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    default_language: Option<String>,
}

impl ProjectEditorOverrides {
    pub fn value(&self, key: ProjectEditorSettingKey) -> Option<ProjectEditorSettingValue> {
        match key {
            ProjectEditorSettingKey::TabSize => {
                self.tab_size.map(ProjectEditorSettingValue::TabSize)
            }
            ProjectEditorSettingKey::AutoDetectLanguage => self
                .auto_detect_language
                .map(ProjectEditorSettingValue::AutoDetectLanguage),
            ProjectEditorSettingKey::DefaultLanguage => self
                .default_language
                .clone()
                .map(ProjectEditorSettingValue::DefaultLanguage),
        }
    }

    pub fn effective(
        &self,
        host: &EditorSettings,
        key: ProjectEditorSettingKey,
    ) -> EffectiveProjectEditorSetting {
        let (value, source) = match self.value(key) {
            Some(value) => (value, ProjectSettingSource::Project),
            None => (
                match key {
                    ProjectEditorSettingKey::TabSize => {
                        ProjectEditorSettingValue::TabSize(host.tab_size)
                    }
                    ProjectEditorSettingKey::AutoDetectLanguage => {
                        ProjectEditorSettingValue::AutoDetectLanguage(host.auto_detect_language)
                    }
                    ProjectEditorSettingKey::DefaultLanguage => {
                        ProjectEditorSettingValue::DefaultLanguage(host.default_language.clone())
                    }
                },
                ProjectSettingSource::Host,
            ),
        };
        EffectiveProjectEditorSetting { key, value, source }
    }

    pub fn tab_size(&self, host: &EditorSettings) -> usize {
        self.tab_size.unwrap_or(host.tab_size)
    }

    pub fn auto_detect_language(&self, host: &EditorSettings) -> bool {
        self.auto_detect_language
            .unwrap_or(host.auto_detect_language)
    }

    pub fn default_language<'a>(&'a self, host: &'a EditorSettings) -> &'a str {
        self.default_language
            .as_deref()
            .unwrap_or(host.default_language.as_str())
    }

    pub fn is_empty(&self) -> bool {
        self.tab_size.is_none()
            && self.auto_detect_language.is_none()
            && self.default_language.is_none()
    }

    fn set(
        &mut self,
        key: ProjectEditorSettingKey,
        value: ProjectEditorSettingValue,
    ) -> Result<(), ProjectSettingsError> {
        match (key, value) {
            (ProjectEditorSettingKey::TabSize, ProjectEditorSettingValue::TabSize(value)) => {
                self.tab_size = Some(value);
            }
            (
                ProjectEditorSettingKey::AutoDetectLanguage,
                ProjectEditorSettingValue::AutoDetectLanguage(value),
            ) => {
                self.auto_detect_language = Some(value);
            }
            (
                ProjectEditorSettingKey::DefaultLanguage,
                ProjectEditorSettingValue::DefaultLanguage(value),
            ) => {
                self.default_language = Some(value);
            }
            (key, value) => {
                return Err(ProjectSettingsError::MismatchedValue {
                    key: key.as_str(),
                    value,
                });
            }
        }
        self.validate()
    }

    fn clear(&mut self, key: ProjectEditorSettingKey) {
        match key {
            ProjectEditorSettingKey::TabSize => self.tab_size = None,
            ProjectEditorSettingKey::AutoDetectLanguage => self.auto_detect_language = None,
            ProjectEditorSettingKey::DefaultLanguage => self.default_language = None,
        }
    }

    fn validate(&mut self) -> Result<(), ProjectSettingsError> {
        if self
            .tab_size
            .is_some_and(|tab_size| !(1..=16).contains(&tab_size))
        {
            return Err(ProjectSettingsError::InvalidValue {
                key: ProjectEditorSettingKey::TabSize.as_str(),
                message: "must be between 1 and 16".to_string(),
            });
        }
        if let Some(default_language) = &mut self.default_language {
            *default_language = default_language.trim().to_string();
            if default_language.is_empty() {
                return Err(ProjectSettingsError::InvalidValue {
                    key: ProjectEditorSettingKey::DefaultLanguage.as_str(),
                    message: "must not be empty".to_string(),
                });
            }
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
struct ProjectSettingsDocument {
    editor: ProjectEditorOverrides,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectSettingsError {
    #[error("project setting `{key}` is not allowed in project configuration")]
    UnsupportedKey { key: String },
    #[error("project setting `{key}` received an incompatible value: {value:?}")]
    MismatchedValue {
        key: &'static str,
        value: ProjectEditorSettingValue,
    },
    #[error("invalid value for project setting `{key}`: {message}")]
    InvalidValue { key: &'static str, message: String },
    #[error("project setting `{key}` changed since this draft was created")]
    Conflict { key: &'static str },
    #[error("this Client cannot modify Host-backed project settings")]
    WriteNotAllowed,
    #[error("project settings are read-only for {project_path}")]
    ReadOnly { project_path: PathBuf },
    #[error("failed to read project settings at {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse project settings at {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("failed to create project settings directory {path}: {source}")]
    CreateDirectory {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to serialize project settings at {path}: {source}")]
    Serialize {
        path: PathBuf,
        #[source]
        source: toml::ser::Error,
    },
    #[error("failed to write project settings at {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to reset project settings at {path}: {source}")]
    Remove {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn project_settings_file(paths: &AppConfigPaths, project_path: &Path) -> PathBuf {
    paths
        .project_layout_file(project_path)
        .with_file_name(PROJECT_SETTINGS_FILE_NAME)
}

fn project_settings_write_file(paths: &AppConfigPaths, project_path: &Path) -> Option<PathBuf> {
    paths
        .project_layout_write_file(project_path)
        .map(|path| path.with_file_name(PROJECT_SETTINGS_FILE_NAME))
}

fn load_project_overrides_from_file(
    project_path: &Path,
    path: &Path,
) -> Result<ProjectEditorOverrides, ProjectSettingsError> {
    let source = match storage::read_project_config(
        project_path,
        yttt_protocol::workspace::WorkspaceProjectConfigFile::Settings,
        path,
    ) {
        Ok(source) => source,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(ProjectEditorOverrides::default());
        }
        Err(source) => {
            return Err(ProjectSettingsError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    let mut document = toml::from_str::<ProjectSettingsDocument>(&source).map_err(|source| {
        ProjectSettingsError::Parse {
            path: path.to_path_buf(),
            source,
        }
    })?;
    document.editor.validate()?;
    Ok(document.editor)
}

pub fn load_project_overrides(
    paths: &AppConfigPaths,
    project_path: &Path,
) -> Result<ProjectEditorOverrides, ProjectSettingsError> {
    load_project_overrides_from_file(project_path, &project_settings_file(paths, project_path))
}

pub fn load_project_editor_settings_snapshot(
    paths: &AppConfigPaths,
    project_path: &Path,
    host: &EditorSettings,
) -> Result<ProjectEditorSettingsSnapshot, ProjectSettingsError> {
    let settings_file = project_settings_file(paths, project_path);
    let overrides = load_project_overrides_from_file(project_path, &settings_file)?;
    Ok(ProjectEditorSettingsSnapshot {
        settings_file,
        effective: ProjectEditorSettingKey::ALL.map(|key| overrides.effective(host, key)),
    })
}

pub fn save_project_override(
    paths: &AppConfigPaths,
    project_path: &Path,
    key: ProjectEditorSettingKey,
    value: Option<ProjectEditorSettingValue>,
    can_write_host: bool,
) -> Result<ProjectEditorOverrides, ProjectSettingsError> {
    save_project_override_with_baseline(paths, project_path, None, key, value, None, can_write_host)
}

fn save_project_override_with_baseline(
    paths: &AppConfigPaths,
    project_path: &Path,
    host: Option<&EditorSettings>,
    key: ProjectEditorSettingKey,
    value: Option<ProjectEditorSettingValue>,
    expected: Option<&EffectiveProjectEditorSetting>,
    can_write_host: bool,
) -> Result<ProjectEditorOverrides, ProjectSettingsError> {
    if !can_write_host {
        return Err(ProjectSettingsError::WriteNotAllowed);
    }
    if !supports_project_override(key.as_str()) {
        return Err(ProjectSettingsError::UnsupportedKey {
            key: key.as_str().to_string(),
        });
    }
    let path = project_settings_write_file(paths, project_path).ok_or_else(|| {
        ProjectSettingsError::ReadOnly {
            project_path: project_path.to_path_buf(),
        }
    })?;
    let mut overrides = load_project_overrides_from_file(project_path, &path)?;
    if let Some(expected) = expected {
        let Some(host) = host else {
            unreachable!("an effective project setting baseline requires Host editor settings");
        };
        if expected.key != key || overrides.effective(host, key) != *expected {
            return Err(ProjectSettingsError::Conflict { key: key.as_str() });
        }
    }
    match value {
        Some(value) => overrides.set(key, value)?,
        None => overrides.clear(key),
    }

    if overrides.is_empty() {
        match storage::remove_file(&path) {
            Ok(()) => {}
            Err(source) if source.kind() == io::ErrorKind::NotFound => {}
            Err(source) => return Err(ProjectSettingsError::Remove { path, source }),
        }
        return Ok(overrides);
    }

    let parent = path
        .parent()
        .expect("project settings paths must have a parent");
    storage::create_dir_all(parent).map_err(|source| ProjectSettingsError::CreateDirectory {
        path: parent.to_path_buf(),
        source,
    })?;
    let source = toml::to_string_pretty(&ProjectSettingsDocument {
        editor: overrides.clone(),
    })
    .map_err(|source| ProjectSettingsError::Serialize {
        path: path.clone(),
        source,
    })?;
    atomic_write(&path, source.as_bytes())
        .map_err(|source| ProjectSettingsError::Write { path, source })?;
    Ok(overrides)
}

pub fn save_effective_project_override(
    paths: &AppConfigPaths,
    project_path: &Path,
    host: &EditorSettings,
    key: ProjectEditorSettingKey,
    value: Option<ProjectEditorSettingValue>,
    can_write_host: bool,
) -> Result<EffectiveProjectEditorSetting, ProjectSettingsError> {
    let overrides = save_project_override(paths, project_path, key, value, can_write_host)?;
    Ok(overrides.effective(host, key))
}

pub fn save_effective_project_override_if_matches(
    paths: &AppConfigPaths,
    project_path: &Path,
    host: &EditorSettings,
    key: ProjectEditorSettingKey,
    value: Option<ProjectEditorSettingValue>,
    expected: &EffectiveProjectEditorSetting,
    can_write_host: bool,
) -> Result<EffectiveProjectEditorSetting, ProjectSettingsError> {
    let overrides = save_project_override_with_baseline(
        paths,
        project_path,
        Some(host),
        key,
        value,
        Some(expected),
        can_write_host,
    )?;
    Ok(overrides.effective(host, key))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_paths() -> (tempfile::TempDir, AppConfigPaths, PathBuf, PathBuf) {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let first = temp.path().join("first-project");
        let second = temp.path().join("second-project");
        let _ = project_settings_file(&paths, &first);
        let _ = project_settings_file(&paths, &second);
        storage::create_dir_all(&first).unwrap();
        storage::create_dir_all(&second).unwrap();
        (temp, paths, first, second)
    }

    #[test]
    fn rejects_unbounded_keys_and_invalid_values() {
        let (_temp, paths, project, _) = test_paths();
        let path = project_settings_file(&paths, &project);
        storage::create_dir_all(path.parent().unwrap()).unwrap();
        storage::write(&path, b"[editor]\nline_numbers = true\n").unwrap();

        assert!(matches!(
            load_project_overrides(&paths, &project),
            Err(ProjectSettingsError::Parse { .. })
        ));
        storage::write(&path, b"[editor]\ntab_size = \"four\"\n").unwrap();
        assert!(matches!(
            load_project_overrides(&paths, &project),
            Err(ProjectSettingsError::Parse { .. })
        ));
        storage::write(&path, b"[editor]\ntab_size = 17\n").unwrap();
        assert!(matches!(
            load_project_overrides(&paths, &project),
            Err(ProjectSettingsError::InvalidValue { .. })
        ));
        storage::remove_file(&path).unwrap();
        assert!(matches!(
            save_project_override(
                &paths,
                &project,
                ProjectEditorSettingKey::TabSize,
                Some(ProjectEditorSettingValue::TabSize(17)),
                true,
            ),
            Err(ProjectSettingsError::InvalidValue { .. })
        ));
        assert!(matches!(
            save_project_override(
                &paths,
                &project,
                ProjectEditorSettingKey::DefaultLanguage,
                Some(ProjectEditorSettingValue::DefaultLanguage(
                    "   ".to_string()
                )),
                true,
            ),
            Err(ProjectSettingsError::InvalidValue { .. })
        ));
        assert!(matches!(
            ProjectEditorSettingKey::from_str("editor.line_numbers"),
            Err(ProjectSettingsError::UnsupportedKey { .. })
        ));
    }

    #[test]
    fn project_values_override_host_and_reset_returns_to_host() {
        let (_temp, paths, project, _) = test_paths();
        let host = EditorSettings {
            tab_size: 8,
            auto_detect_language: true,
            default_language: "rust".to_string(),
            ..EditorSettings::default()
        };
        let overrides = save_project_override(
            &paths,
            &project,
            ProjectEditorSettingKey::TabSize,
            Some(ProjectEditorSettingValue::TabSize(2)),
            true,
        )
        .unwrap();

        assert_eq!(
            overrides.effective(&host, ProjectEditorSettingKey::TabSize),
            EffectiveProjectEditorSetting {
                key: ProjectEditorSettingKey::TabSize,
                value: ProjectEditorSettingValue::TabSize(2),
                source: ProjectSettingSource::Project,
            }
        );
        assert_eq!(overrides.tab_size(&host), 2);

        let reset = save_project_override(
            &paths,
            &project,
            ProjectEditorSettingKey::TabSize,
            None,
            true,
        )
        .unwrap();
        assert_eq!(
            reset.effective(&host, ProjectEditorSettingKey::TabSize),
            EffectiveProjectEditorSetting {
                key: ProjectEditorSettingKey::TabSize,
                value: ProjectEditorSettingValue::TabSize(8),
                source: ProjectSettingSource::Host,
            }
        );
        assert!(!storage::exists(project_settings_file(&paths, &project)));
    }

    #[test]
    fn baseline_aware_save_rejects_a_changed_project_override() {
        let (_temp, paths, project, _) = test_paths();
        let host = EditorSettings::default();
        save_project_override(
            &paths,
            &project,
            ProjectEditorSettingKey::TabSize,
            Some(ProjectEditorSettingValue::TabSize(2)),
            true,
        )
        .unwrap();
        let baseline = load_project_overrides(&paths, &project)
            .unwrap()
            .effective(&host, ProjectEditorSettingKey::TabSize);
        save_project_override(
            &paths,
            &project,
            ProjectEditorSettingKey::TabSize,
            Some(ProjectEditorSettingValue::TabSize(4)),
            true,
        )
        .unwrap();

        assert!(matches!(
            save_effective_project_override_if_matches(
                &paths,
                &project,
                &host,
                ProjectEditorSettingKey::TabSize,
                Some(ProjectEditorSettingValue::TabSize(8)),
                &baseline,
                true,
            ),
            Err(ProjectSettingsError::Conflict { .. })
        ));
        assert_eq!(
            load_project_overrides(&paths, &project)
                .unwrap()
                .effective(&host, ProjectEditorSettingKey::TabSize)
                .value,
            ProjectEditorSettingValue::TabSize(4)
        );
    }

    #[test]
    fn baseline_aware_save_rejects_a_changed_host_default_without_creating_a_file() {
        let (_temp, paths, project, _) = test_paths();
        let previous_host = EditorSettings {
            tab_size: 2,
            ..EditorSettings::default()
        };
        let baseline = load_project_overrides(&paths, &project)
            .unwrap()
            .effective(&previous_host, ProjectEditorSettingKey::TabSize);
        let current_host = EditorSettings {
            tab_size: 4,
            ..EditorSettings::default()
        };

        assert!(matches!(
            save_effective_project_override_if_matches(
                &paths,
                &project,
                &current_host,
                ProjectEditorSettingKey::TabSize,
                Some(ProjectEditorSettingValue::TabSize(8)),
                &baseline,
                true,
            ),
            Err(ProjectSettingsError::Conflict { .. })
        ));
        assert!(!storage::exists(project_settings_file(&paths, &project)));
    }

    #[test]
    fn project_files_are_isolated() {
        let (_temp, paths, first, second) = test_paths();
        let host = EditorSettings::default();
        save_project_override(
            &paths,
            &first,
            ProjectEditorSettingKey::DefaultLanguage,
            Some(ProjectEditorSettingValue::DefaultLanguage(
                "rust".to_string(),
            )),
            true,
        )
        .unwrap();

        let first_settings = load_project_overrides(&paths, &first).unwrap();
        let second_settings = load_project_overrides(&paths, &second).unwrap();
        assert_eq!(
            first_settings.effective(&host, ProjectEditorSettingKey::DefaultLanguage),
            EffectiveProjectEditorSetting {
                key: ProjectEditorSettingKey::DefaultLanguage,
                value: ProjectEditorSettingValue::DefaultLanguage("rust".to_string()),
                source: ProjectSettingSource::Project,
            }
        );
        assert_eq!(
            second_settings.effective(&host, ProjectEditorSettingKey::DefaultLanguage),
            EffectiveProjectEditorSetting {
                key: ProjectEditorSettingKey::DefaultLanguage,
                value: ProjectEditorSettingValue::DefaultLanguage(host.default_language),
                source: ProjectSettingSource::Host,
            }
        );
    }

    #[test]
    fn observers_cannot_write_project_overrides() {
        let (_temp, paths, project, _) = test_paths();
        let result = save_project_override(
            &paths,
            &project,
            ProjectEditorSettingKey::AutoDetectLanguage,
            Some(ProjectEditorSettingValue::AutoDetectLanguage(false)),
            false,
        );

        assert!(matches!(result, Err(ProjectSettingsError::WriteNotAllowed)));
        assert!(!storage::exists(project_settings_file(&paths, &project)));
    }
    #[test]
    fn overlay_policy_isolated_project_settings_from_project_tree() {
        let temp = tempfile::tempdir().unwrap();
        let profile = crate::config::profile::AppProfile::scoped(
            crate::model::ids::ProfileId::new("project-settings-overlay"),
            crate::config::profile::EnvironmentKind::Test,
            crate::config::profile::ProfilePersistence::Ephemeral,
            temp.path(),
            crate::config::profile::ProjectConfigPolicy::Overlay,
            crate::config::profile::HostConnectPolicy::ProfileDiscovery,
        );
        let paths = AppConfigPaths::from_profile(&profile);
        let project = temp.path().join("project");
        let path = project_settings_file(&paths, &project);
        storage::create_dir_all(&project).unwrap();

        save_project_override(
            &paths,
            &project,
            ProjectEditorSettingKey::AutoDetectLanguage,
            Some(ProjectEditorSettingValue::AutoDetectLanguage(false)),
            true,
        )
        .unwrap();

        assert_ne!(path, project.join(".yttt").join(PROJECT_SETTINGS_FILE_NAME));
        assert_eq!(
            load_project_overrides(&paths, &project)
                .unwrap()
                .value(ProjectEditorSettingKey::AutoDetectLanguage),
            Some(ProjectEditorSettingValue::AutoDetectLanguage(false))
        );
    }

    #[test]
    fn read_only_policy_loads_but_rejects_writes() {
        let temp = tempfile::tempdir().unwrap();
        let profile = crate::config::profile::AppProfile::scoped(
            crate::model::ids::ProfileId::new("project-settings-read-only"),
            crate::config::profile::EnvironmentKind::Test,
            crate::config::profile::ProfilePersistence::Ephemeral,
            temp.path(),
            crate::config::profile::ProjectConfigPolicy::ReadOnly,
            crate::config::profile::HostConnectPolicy::ProfileDiscovery,
        );
        let paths = AppConfigPaths::from_profile(&profile);
        let project = temp.path().join("project");
        let path = project_settings_file(&paths, &project);
        storage::create_dir_all(&project).unwrap();
        storage::create_dir_all(path.parent().unwrap()).unwrap();
        storage::write(&path, b"[editor]\ntab_size = 2\n").unwrap();

        assert_eq!(
            load_project_overrides(&paths, &project)
                .unwrap()
                .value(ProjectEditorSettingKey::TabSize),
            Some(ProjectEditorSettingValue::TabSize(2))
        );
        assert!(matches!(
            save_project_override(
                &paths,
                &project,
                ProjectEditorSettingKey::TabSize,
                Some(ProjectEditorSettingValue::TabSize(4)),
                true,
            ),
            Err(ProjectSettingsError::ReadOnly { .. })
        ));
    }
}
