use std::{
    fs::OpenOptions,
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use crate::{
    config::{
        default_layout::{DefaultLayoutState, LayoutLoadWarning},
        paths::{AppConfigPaths, canonicalize_path},
    },
    model::{
        ids::{ConnectionId, ProjectId},
        layout::{LayoutError, ProjectLayout},
        project::{ProjectDescriptor, ProjectLocation, RemotePathBuf},
    },
};

use yttt_layout::{
    compose::{Personal, PersonalSource, PersonalWarning, apply_personal},
    personal::{self as personal_layout, PersonalLayoutFileError},
    project::{ProjectLayoutError, project_name, resolve_layout_variables},
};
pub use yttt_layout::{
    overrides::{
        LayoutMerge, LayoutNodeOverride, LayoutOverride, MergeWarning, PaneOverride,
        ProjectOverride, TabOverride, merge_layouts,
    },
    personal::PersonalLayout,
    recent::{ProjectReferenceConfig, RecentProjectConfig, RecentProjectsConfig},
};

const PROJECT_LAYOUT_GUIDE_FILE_NAME: &str = "README.md";
const PROJECT_LAYOUT_GUIDE: &str = r#"# yttt Project Layout

This directory contains the shareable terminal layout for this project.
AI agents should edit `.yttt/layout.toml` when asked to create or update the project layout.
Do not add `version` or `mode`; those fields belong only to personal app-local layouts.

## Schema

- `[project]`
  - `name`: project display name.
  - `default_tab`: optional tab ID to select when the project opens.
- `[[tabs]]`
  - `id`: stable, unique tab ID.
  - `title`: visible tab title.
  - `cwd`: optional working directory, relative to the project root or prefixed with `<ProjectDir>`.
  - `startup`: optional `lazy` or `eager`; defaults to `lazy`.
  - `layout`: one pane or a recursive split.
- Pane node (`type = "pane"`)
  - Required: `id`, `title`, `command`.
  - Optional: `args`, `execution_mode`, `exit_behavior`, `kind`, `notify_on_exit`, `detector`.
- Split node (`type = "split"`)
  - Required: `direction`, `ratio`, `left`, `right`.
  - `direction`: `horizontal` or `vertical`.
  - `ratio`: number from `0.05` through `0.95`.
  - `left` and `right`: pane or split nodes.

Pane IDs must be unique within each tab. `default_tab` must match an existing tab ID.
`startup = "eager"` starts every pane in that tab when the project opens, even when another
tab is selected. Omitted or `startup = "lazy"` tabs start when first selected. The
`default_tab` always starts because it is initially selected.
`execution_mode = "shell"` starts a persistent interactive shell and queues `command` as shell
input. The shell remains open after that command finishes; `args` is ignored. Use
`execution_mode = "command"` with `args` for one-shot programs that should exit with the command.
`exit_behavior` applies to the persistent shell in shell mode and to the program in command mode;
it accepts `close`, `auto_restart`, or `manual_restart`. `kind` accepts `shell` or `agent`.

## Example

```toml
[project]
name = "Example"
default_tab = "shell"

[[tabs]]
id = "services"
title = "Services"
startup = "eager"
cwd = "<ProjectDir>"

[tabs.layout]
type = "split"
direction = "horizontal"
ratio = 0.6
left = { type = "pane", id = "server", title = "Server", command = "npm", args = ["run", "dev"], execution_mode = "command", exit_behavior = "auto_restart" }
right = { type = "pane", id = "worker", title = "Worker", command = "npm", args = ["run", "worker"], execution_mode = "command", exit_behavior = "auto_restart" }

[[tabs]]
id = "shell"
title = "Shell"
layout = { type = "pane", id = "shell", title = "Shell", command = "", execution_mode = "shell", exit_behavior = "manual_restart" }
```

## AI checklist

1. Inspect the repository's real scripts and tools before choosing commands.
2. Preserve stable tab and pane IDs when updating an existing layout.
3. Keep commands portable; prefer project-relative `cwd` values.
4. Use `startup = "eager"` only for services that must start with the project.
5. Produce valid TOML and keep split ratios within the supported range.
6. Do not invent services or commands that are not present in the repository.
"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LayoutSource {
    GlobalDefault(PathBuf),
    GlobalDefaultWithPersonalPatch { global: PathBuf, local: PathBuf },
    ProjectConfig(PathBuf),
    ProjectConfigWithPersonalPatch { project: PathBuf, local: PathBuf },
    PersonalReplace(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LoadedProjectLayout {
    pub layout: ProjectLayout,
    pub source: LayoutSource,
    pub warnings: Vec<LayoutLoadWarning>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectOpenConfig {
    pub descriptor: ProjectDescriptor,
    pub layout: ProjectLayout,
    pub layout_source: LayoutSource,
    pub warnings: Vec<LayoutLoadWarning>,
    pub recent_projects: RecentProjectsConfig,
}

#[derive(Debug, thiserror::Error)]
pub enum ProjectOpenError {
    #[error("failed to open project directory {path}: {source}")]
    OpenProjectDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("project path is not a directory: {0}")]
    NotDirectory(PathBuf),
    #[error("failed to read project layout at {path}: {source}")]
    ReadProjectLayout {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse project layout at {path}: {source}")]
    ParseProjectLayout {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("invalid project layout at {path}: {source}")]
    InvalidProjectLayout { path: PathBuf, source: LayoutError },
    #[error("failed to create app config directory {path}: {source}")]
    CreateConfigDirectory {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize default project layout at {path}: {source}")]
    SerializeDefaultLayout {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write default project layout at {path}: {source}")]
    WriteDefaultLayout {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize project layout at {path}: {source}")]
    SerializeProjectLayout {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write project layout at {path}: {source}")]
    WriteProjectLayout {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to write project layout guide at {path}: {source}")]
    WriteProjectLayoutGuide {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to serialize personal layout at {path}: {source}")]
    SerializePersonalLayout {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write personal layout at {path}: {source}")]
    WritePersonalLayout {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to remove personal layout at {path}: {source}")]
    RemovePersonalLayout {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse personal layout at {path}: {message}")]
    PersonalOverrideParse { path: PathBuf, message: String },
    #[error("invalid personal layout at {path}: {message}")]
    PersonalOverrideValidation { path: PathBuf, message: String },
    #[error("failed to read recent projects at {path}: {source}")]
    ReadRecentProjects {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse recent projects at {path}: {source}")]
    ParseRecentProjects {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("failed to serialize recent projects at {path}: {source}")]
    SerializeRecentProjects {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("failed to write recent projects at {path}: {source}")]
    WriteRecentProjects {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("project config is read-only for {project_path}")]
    ProjectConfigReadOnly { project_path: PathBuf },
}

pub fn parse_personal_layout(
    path: &Path,
    source: &str,
) -> Result<PersonalLayout, ProjectOpenError> {
    personal_layout::parse(source).map_err(|error| match error {
        PersonalLayoutFileError::Parse(message) => ProjectOpenError::PersonalOverrideParse {
            path: path.to_path_buf(),
            message,
        },
        PersonalLayoutFileError::Validation(message) => {
            ProjectOpenError::PersonalOverrideValidation {
                path: path.to_path_buf(),
                message,
            }
        }
    })
}

pub fn serialize_personal_patch(layout: &LayoutOverride) -> Result<String, toml::ser::Error> {
    personal_layout::serialize_patch(layout)
}

pub fn serialize_personal_replace(layout: &ProjectLayout) -> Result<String, toml::ser::Error> {
    personal_layout::serialize_replace(layout)
}

pub fn open_project_config(
    paths: &AppConfigPaths,
    project_path: &Path,
    default_state: &mut DefaultLayoutState,
) -> Result<ProjectOpenConfig, ProjectOpenError> {
    if paths.environment() == super::profile::EnvironmentKind::Test {
        super::storage::allow_test_root(project_path);
    }
    let project_path = canonicalize_path(project_path).map_err(|source| {
        ProjectOpenError::OpenProjectDirectory {
            path: project_path.to_path_buf(),
            source,
        }
    })?;
    if !crate::config::storage::is_dir(&project_path) {
        return Err(ProjectOpenError::NotDirectory(project_path));
    }

    let mut loaded = load_project_layout(paths, &project_path, default_state)?;
    resolve_layout_variables(&mut loaded.layout, &project_path);
    let location = ProjectLocation::local(project_path);
    let (project_id, recent_projects) =
        record_recent_project(paths, &location, &loaded.layout.project.name)?;

    Ok(ProjectOpenConfig {
        descriptor: ProjectDescriptor::new(project_id, location),
        layout: loaded.layout,
        layout_source: loaded.source,
        warnings: loaded.warnings,
        recent_projects,
    })
}
pub fn open_ssh_project_config(
    paths: &AppConfigPaths,
    connection_id: ConnectionId,
    root: RemotePathBuf,
    title: &str,
    default_state: &mut DefaultLayoutState,
) -> Result<ProjectOpenConfig, ProjectOpenError> {
    let _ = default_state.reload();
    let mut layout = default_state.template().materialize(title);
    resolve_layout_variables(&mut layout, Path::new(root.as_str()));
    let location = ProjectLocation::Ssh {
        connection_id,
        root,
    };
    let (project_id, recent_projects) =
        record_recent_project(paths, &location, &layout.project.name)?;
    Ok(ProjectOpenConfig {
        descriptor: ProjectDescriptor::new(project_id, location),
        layout,
        layout_source: LayoutSource::GlobalDefault(paths.default_layout_file()),
        warnings: default_state.warnings().to_vec(),
        recent_projects,
    })
}

pub fn load_recent_projects(
    paths: &AppConfigPaths,
) -> Result<RecentProjectsConfig, ProjectOpenError> {
    let path = paths.recent_projects_file();
    let source = match crate::config::storage::read_to_string(&path) {
        Ok(source) => source,
        Err(source) if source.kind() == io::ErrorKind::NotFound => {
            return Ok(RecentProjectsConfig::default());
        }
        Err(source) => {
            return Err(ProjectOpenError::ReadRecentProjects { path, source });
        }
    };
    RecentProjectsConfig::parse(&source)
        .map_err(|source| ProjectOpenError::ParseRecentProjects { path, source })
}

pub fn save_local_layout(
    paths: &AppConfigPaths,
    project_path: &Path,
    layout: &ProjectLayout,
) -> Result<PathBuf, ProjectOpenError> {
    let path = paths.local_layout_file(project_path);
    if let Some(parent) = path.parent() {
        crate::config::storage::create_dir_all(parent).map_err(|source| {
            ProjectOpenError::CreateConfigDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    let source = serialize_personal_replace(layout).map_err(|source| {
        ProjectOpenError::SerializePersonalLayout {
            path: path.clone(),
            source,
        }
    })?;
    crate::config::storage::write(&path, source).map_err(|source| {
        ProjectOpenError::WritePersonalLayout {
            path: path.clone(),
            source,
        }
    })?;
    Ok(path)
}

pub fn reset_local_override(
    paths: &AppConfigPaths,
    project_path: &Path,
) -> Result<(), ProjectOpenError> {
    reset_local_override_with_file_system(paths, project_path, &StdLocalLayoutFileSystem)
}

fn reset_local_override_with_file_system(
    paths: &AppConfigPaths,
    project_path: &Path,
    file_system: &dyn LocalLayoutFileSystem,
) -> Result<(), ProjectOpenError> {
    let path = paths.local_layout_file(project_path);
    match file_system.remove_file(&path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(ProjectOpenError::RemovePersonalLayout { path, source }),
    }
}

trait LocalLayoutFileSystem {
    fn remove_file(&self, path: &Path) -> io::Result<()>;
}

struct StdLocalLayoutFileSystem;

impl LocalLayoutFileSystem for StdLocalLayoutFileSystem {
    fn remove_file(&self, path: &Path) -> io::Result<()> {
        crate::config::storage::remove_file(path)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProjectLayoutScaffold {
    pub layout_file: PathBuf,
    pub guide_file: PathBuf,
    pub created_layout_file: bool,
    pub created_guide_file: bool,
}

pub fn create_project_layout_scaffold(
    paths: &AppConfigPaths,
    project_path: &Path,
    layout: &ProjectLayout,
) -> Result<ProjectLayoutScaffold, ProjectOpenError> {
    let layout_file = paths
        .project_layout_write_file(project_path)
        .ok_or_else(|| ProjectOpenError::ProjectConfigReadOnly {
            project_path: project_path.to_path_buf(),
        })?;
    let project_config_dir = layout_file
        .parent()
        .expect("project layout path must have a parent");
    crate::config::storage::create_dir_all(project_config_dir).map_err(|source| {
        ProjectOpenError::CreateConfigDirectory {
            path: project_config_dir.to_path_buf(),
            source,
        }
    })?;

    let layout_source = toml::to_string_pretty(layout).map_err(|source| {
        ProjectOpenError::SerializeProjectLayout {
            path: layout_file.clone(),
            source,
        }
    })?;
    let created_layout_file =
        write_new_file(&layout_file, layout_source.as_bytes()).map_err(|source| {
            ProjectOpenError::WriteProjectLayout {
                path: layout_file.clone(),
                source,
            }
        })?;

    let guide_file = project_config_dir.join(PROJECT_LAYOUT_GUIDE_FILE_NAME);
    let created_guide_file =
        write_new_file(&guide_file, PROJECT_LAYOUT_GUIDE.as_bytes()).map_err(|source| {
            ProjectOpenError::WriteProjectLayoutGuide {
                path: guide_file.clone(),
                source,
            }
        })?;

    Ok(ProjectLayoutScaffold {
        layout_file,
        guide_file,
        created_layout_file,
        created_guide_file,
    })
}

fn write_new_file(path: &Path, contents: &[u8]) -> io::Result<bool> {
    if let Some(storage) = crate::config::storage::storage_for(path)? {
        if storage.exists(path) {
            return Ok(false);
        }
        storage.write(path, contents)?;
        return Ok(true);
    }
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return if crate::config::storage::is_file(path) {
                Ok(false)
            } else {
                Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    "path exists and is not a regular file",
                ))
            };
        }
        Err(error) => return Err(error),
    };

    if let Err(error) = file.write_all(contents) {
        drop(file);
        let _ = crate::config::storage::remove_file(path);
        return Err(error);
    }
    Ok(true)
}

pub fn export_project_layout(
    paths: &AppConfigPaths,
    project_path: &Path,
    layout: &ProjectLayout,
) -> Result<PathBuf, ProjectOpenError> {
    let path = paths
        .project_layout_write_file(project_path)
        .ok_or_else(|| ProjectOpenError::ProjectConfigReadOnly {
            project_path: project_path.to_path_buf(),
        })?;
    if let Some(parent) = path.parent() {
        crate::config::storage::create_dir_all(parent).map_err(|source| {
            ProjectOpenError::CreateConfigDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }
    write_project_layout(&path, layout)?;
    Ok(path)
}

fn load_project_layout(
    paths: &AppConfigPaths,
    project_path: &Path,
    default_state: &mut DefaultLayoutState,
) -> Result<LoadedProjectLayout, ProjectOpenError> {
    let project_layout_file = paths.project_layout_file(project_path);
    let (base, base_source, mut warnings) =
        match read_project_layout(&project_layout_file, project_path) {
            Ok(layout) => (
                layout,
                LayoutSource::ProjectConfig(project_layout_file),
                Vec::new(),
            ),
            Err(ProjectOpenError::ReadProjectLayout { source, .. })
                if source.kind() == io::ErrorKind::NotFound =>
            {
                let _ = default_state.reload();
                (
                    default_state
                        .template()
                        .materialize(project_name(project_path)),
                    LayoutSource::GlobalDefault(paths.default_layout_file()),
                    default_state.warnings().to_vec(),
                )
            }
            Err(error) => return Err(error),
        };

    let local_layout_file = paths.local_layout_file(project_path);
    let read = crate::config::storage::read_to_string(&local_layout_file);
    let read_error;
    let personal_source = match &read {
        Ok(source) => PersonalSource::Text(source),
        Err(error) if error.kind() == io::ErrorKind::NotFound => PersonalSource::Missing,
        Err(error) => {
            read_error = error.to_string();
            PersonalSource::Unreadable(&read_error)
        }
    };
    let applied = apply_personal(base, personal_source);
    warnings.extend(applied.warnings.into_iter().map(|warning| {
        let path = local_layout_file.clone();
        match warning {
            PersonalWarning::Read(message) => {
                LayoutLoadWarning::PersonalOverrideRead { path, message }
            }
            PersonalWarning::Parse(message) => {
                LayoutLoadWarning::PersonalOverrideParse { path, message }
            }
            PersonalWarning::Validation(message) => {
                LayoutLoadWarning::PersonalOverrideValidation { path, message }
            }
            PersonalWarning::StaleTab(tab_id) => {
                LayoutLoadWarning::StaleOverrideTab { path, tab_id }
            }
            PersonalWarning::StalePane(pane_id) => {
                LayoutLoadWarning::StaleOverridePane { path, pane_id }
            }
        }
    }));
    let source = match (applied.personal, base_source) {
        (Personal::None, base_source) => base_source,
        (Personal::Replace, _) => LayoutSource::PersonalReplace(local_layout_file),
        (Personal::Patch, LayoutSource::GlobalDefault(global)) => {
            LayoutSource::GlobalDefaultWithPersonalPatch {
                global,
                local: local_layout_file,
            }
        }
        (Personal::Patch, LayoutSource::ProjectConfig(project)) => {
            LayoutSource::ProjectConfigWithPersonalPatch {
                project,
                local: local_layout_file,
            }
        }
        (Personal::Patch, _) => unreachable!("personal patch base must be project or global"),
    };
    Ok(LoadedProjectLayout {
        layout: applied.layout,
        source,
        warnings,
    })
}

fn read_project_layout(
    path: &Path,
    project_path: &Path,
) -> Result<ProjectLayout, ProjectOpenError> {
    let source = crate::config::storage::read_project_config(
        project_path,
        yttt_protocol::workspace::WorkspaceProjectConfigFile::Layout,
        path,
    )
    .map_err(|source| ProjectOpenError::ReadProjectLayout {
        path: path.to_path_buf(),
        source,
    })?;
    parse_project_layout(path, project_path, &source)
}

fn parse_project_layout(
    path: &Path,
    project_path: &Path,
    source: &str,
) -> Result<ProjectLayout, ProjectOpenError> {
    yttt_layout::project::parse_project_layout(source, project_path).map_err(|error| match error {
        ProjectLayoutError::Parse(source) => ProjectOpenError::ParseProjectLayout {
            path: path.to_path_buf(),
            source,
        },
        ProjectLayoutError::Invalid(source) => ProjectOpenError::InvalidProjectLayout {
            path: path.to_path_buf(),
            source,
        },
    })
}

fn write_project_layout(path: &Path, layout: &ProjectLayout) -> Result<(), ProjectOpenError> {
    if let Some(parent) = path.parent() {
        crate::config::storage::create_dir_all(parent).map_err(|source| {
            ProjectOpenError::CreateConfigDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    let source = toml::to_string_pretty(layout).map_err(|source| {
        ProjectOpenError::SerializeProjectLayout {
            path: path.to_path_buf(),
            source,
        }
    })?;
    crate::config::storage::write(path, source).map_err(|source| {
        ProjectOpenError::WriteProjectLayout {
            path: path.to_path_buf(),
            source,
        }
    })
}

fn record_recent_project(
    paths: &AppConfigPaths,
    location: &ProjectLocation,
    title: &str,
) -> Result<(ProjectId, RecentProjectsConfig), ProjectOpenError> {
    let mut recent_projects = load_recent_projects(paths)?;
    let project_id = recent_projects.record(location, title);
    write_recent_projects(paths, &recent_projects)?;
    Ok((project_id, recent_projects))
}

pub fn save_last_opened_projects(
    paths: &AppConfigPaths,
    recent_projects: &mut RecentProjectsConfig,
    projects: Vec<ProjectReferenceConfig>,
) -> Result<(), ProjectOpenError> {
    if !projects.is_empty() {
        recent_projects.last_restorable_projects = projects.clone();
    }
    recent_projects.last_opened_projects = projects;
    write_recent_projects(paths, recent_projects)
}

pub fn remove_recent_projects_for_ssh_connection(
    paths: &AppConfigPaths,
    recent_projects: &mut RecentProjectsConfig,
    connection_id: &ConnectionId,
) -> Result<(), ProjectOpenError> {
    let belongs_to_connection = |location: &ProjectLocation| {
        matches!(
            location,
            ProjectLocation::Ssh {
                connection_id: project_connection_id,
                ..
            } if project_connection_id == connection_id
        )
    };
    let mut updated = recent_projects.clone();
    updated
        .projects
        .retain(|project| !belongs_to_connection(&project.location));
    updated
        .last_opened_projects
        .retain(|project| !belongs_to_connection(&project.location));
    updated
        .last_restorable_projects
        .retain(|project| !belongs_to_connection(&project.location));
    write_recent_projects(paths, &updated)?;
    *recent_projects = updated;
    Ok(())
}

fn write_recent_projects(
    paths: &AppConfigPaths,
    recent_projects: &RecentProjectsConfig,
) -> Result<(), ProjectOpenError> {
    let path = paths.recent_projects_file();
    if let Some(parent) = path.parent() {
        crate::config::storage::create_dir_all(parent).map_err(|source| {
            ProjectOpenError::CreateConfigDirectory {
                path: parent.to_path_buf(),
                source,
            }
        })?;
    }

    let source = toml::to_string_pretty(recent_projects).map_err(|source| {
        ProjectOpenError::SerializeRecentProjects {
            path: path.clone(),
            source,
        }
    })?;
    crate::config::atomic_write(&path, source.as_bytes())
        .map_err(|source| ProjectOpenError::WriteRecentProjects { path, source })
}

#[cfg(test)]
mod tests {
    use std::{cell::RefCell, collections::HashSet, io};

    use super::*;

    #[derive(Default)]
    struct FakeLocalLayoutFileSystem {
        files: RefCell<HashSet<PathBuf>>,
        remove_error: RefCell<Option<String>>,
    }

    impl LocalLayoutFileSystem for FakeLocalLayoutFileSystem {
        fn remove_file(&self, path: &Path) -> io::Result<()> {
            if let Some(message) = self.remove_error.borrow_mut().take() {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, message));
            }
            self.files.borrow_mut().remove(path);
            Ok(())
        }
    }

    #[test]
    fn missing_recent_projects_uses_defaults_without_creating_config_directory() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));

        assert_eq!(
            load_recent_projects(&paths).unwrap(),
            RecentProjectsConfig::default()
        );
        assert!(!paths.config_dir().exists());
    }

    #[test]
    fn removing_ssh_connection_prunes_all_recent_project_references() {
        let temp = tempfile::tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path().join("config"));
        let removed_connection = ConnectionId::new("removed");
        let retained_connection = ConnectionId::new("retained");
        let removed_location = ProjectLocation::Ssh {
            connection_id: removed_connection.clone(),
            root: RemotePathBuf::new("/srv/removed").unwrap(),
        };
        let retained_location = ProjectLocation::Ssh {
            connection_id: retained_connection,
            root: RemotePathBuf::new("/srv/retained").unwrap(),
        };
        let local_location = ProjectLocation::local("/tmp/local");
        let mut recent = RecentProjectsConfig {
            projects: vec![
                RecentProjectConfig {
                    id: ProjectId::random(),
                    title: "removed".to_string(),
                    location: removed_location.clone(),
                },
                RecentProjectConfig {
                    id: ProjectId::random(),
                    title: "retained".to_string(),
                    location: retained_location.clone(),
                },
                RecentProjectConfig {
                    id: ProjectId::random(),
                    title: "local".to_string(),
                    location: local_location.clone(),
                },
            ],
            last_opened_projects: vec![
                ProjectReferenceConfig::new(ProjectId::random(), removed_location.clone()),
                ProjectReferenceConfig::new(ProjectId::random(), retained_location.clone()),
            ],
            last_restorable_projects: vec![
                ProjectReferenceConfig::new(ProjectId::random(), removed_location),
                ProjectReferenceConfig::new(ProjectId::random(), local_location.clone()),
            ],
            ..RecentProjectsConfig::default()
        };

        remove_recent_projects_for_ssh_connection(&paths, &mut recent, &removed_connection)
            .unwrap();

        let persisted = load_recent_projects(&paths).unwrap();
        assert_eq!(recent, persisted);
        assert_eq!(
            recent
                .projects
                .iter()
                .map(|project| project.location.clone())
                .collect::<Vec<_>>(),
            vec![retained_location.clone(), local_location.clone()]
        );
        assert_eq!(
            recent.last_opened_projects,
            vec![ProjectReferenceConfig::new(
                recent.last_opened_projects[0].id.clone(),
                retained_location,
            )]
        );
        assert_eq!(
            recent.last_restorable_projects,
            vec![ProjectReferenceConfig::new(
                recent.last_restorable_projects[0].id.clone(),
                local_location,
            )]
        );
    }

    #[test]
    fn reset_local_override_remove_failure_keeps_personal_file() {
        let paths = AppConfigPaths::from_config_dir("/config");
        let project = Path::new("/project");
        let path = paths.local_layout_file(project);
        let file_system = FakeLocalLayoutFileSystem::default();
        file_system.files.borrow_mut().insert(path.clone());
        *file_system.remove_error.borrow_mut() = Some("remove denied".to_string());

        let error =
            reset_local_override_with_file_system(&paths, project, &file_system).unwrap_err();

        assert!(file_system.files.borrow().contains(&path));
        assert!(matches!(
            error,
            ProjectOpenError::RemovePersonalLayout {
                path: error_path,
                source,
            } if error_path == path && source.to_string() == "remove denied"
        ));
    }
}
