use std::{
    fs::{self, OpenOptions},
    io::{self, Write as _},
    path::{Path, PathBuf},
};

use crate::{
    config::{
        default_layout::{DefaultLayoutState, LayoutLoadWarning},
        paths::{AppConfigPaths, canonicalize_path},
        personal_layout::{self, PersonalLayoutFileError},
    },
    model::layout::{
        LayoutError, LayoutNode, PaneConfig, PaneKind, ProcessExitBehavior, ProjectConfig,
        ProjectLayout, TabConfig, TabStartup, TerminalExecutionMode,
    },
};

pub use crate::config::personal_layout::PersonalLayout;

const PROJECT_DIR_VARIABLE: &str = "<ProjectDir>";
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
Use `execution_mode = "shell"` for shell syntax and interactive shells. Use
`execution_mode = "command"` with `args` when argument boundaries must be preserved.
`exit_behavior` accepts `close`, `auto_restart`, or `manual_restart`. `kind` accepts
`shell` or `agent`.

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

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct LayoutOverride {
    pub project: Option<ProjectOverride>,
    #[serde(default)]
    pub tabs: Vec<TabOverride>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct ProjectOverride {
    pub name: Option<String>,
    pub default_tab: Option<String>,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct TabOverride {
    pub id: String,
    pub title: Option<String>,
    pub cwd: Option<PathBuf>,
    pub startup: Option<TabStartup>,
    pub layout: Option<LayoutNodeOverride>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LayoutNodeOverride {
    Pane(PaneOverride),
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq)]
pub struct PaneOverride {
    pub id: String,
    pub title: Option<String>,
    pub command: Option<String>,
    pub args: Option<Vec<String>>,
    pub execution_mode: Option<TerminalExecutionMode>,
    pub exit_behavior: Option<ProcessExitBehavior>,
    pub kind: Option<PaneKind>,
    pub notify_on_exit: Option<bool>,
    pub detector: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeWarning {
    StaleTabOverride(String),
    StalePaneOverride(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct LayoutMerge {
    pub layout: ProjectLayout,
    pub warnings: Vec<MergeWarning>,
}

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
    pub path: PathBuf,
    pub layout: ProjectLayout,
    pub layout_source: LayoutSource,
    pub warnings: Vec<LayoutLoadWarning>,
    pub recent_projects: RecentProjectsConfig,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct RecentProjectsConfig {
    #[serde(default)]
    pub projects: Vec<RecentProjectConfig>,
    #[serde(default)]
    pub last_opened_projects: Vec<PathBuf>,
}

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct RecentProjectConfig {
    pub title: String,
    pub path: PathBuf,
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
    let project_path = canonicalize_path(project_path).map_err(|source| {
        ProjectOpenError::OpenProjectDirectory {
            path: project_path.to_path_buf(),
            source,
        }
    })?;
    if !project_path.is_dir() {
        return Err(ProjectOpenError::NotDirectory(project_path));
    }

    let mut loaded = load_project_layout(paths, &project_path, default_state)?;
    resolve_layout_variables(&mut loaded.layout, &project_path);
    let recent_projects = record_recent_project(paths, &project_path, &loaded.layout.project.name)?;

    Ok(ProjectOpenConfig {
        path: project_path,
        layout: loaded.layout,
        layout_source: loaded.source,
        warnings: loaded.warnings,
        recent_projects,
    })
}

pub fn load_recent_projects(
    paths: &AppConfigPaths,
) -> Result<RecentProjectsConfig, ProjectOpenError> {
    let path = paths.recent_projects_file();
    if !path.exists() {
        return Ok(RecentProjectsConfig::default());
    }

    let source =
        fs::read_to_string(&path).map_err(|source| ProjectOpenError::ReadRecentProjects {
            path: path.clone(),
            source,
        })?;
    toml::from_str(&source).map_err(|source| ProjectOpenError::ParseRecentProjects { path, source })
}

pub fn save_local_layout(
    paths: &AppConfigPaths,
    project_path: &Path,
    layout: &ProjectLayout,
) -> Result<PathBuf, ProjectOpenError> {
    let path = paths.local_layout_file(project_path);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ProjectOpenError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let source = serialize_personal_replace(layout).map_err(|source| {
        ProjectOpenError::SerializePersonalLayout {
            path: path.clone(),
            source,
        }
    })?;
    fs::write(&path, source).map_err(|source| ProjectOpenError::WritePersonalLayout {
        path: path.clone(),
        source,
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
    if !file_system.exists(&path) {
        return Ok(());
    }
    file_system
        .remove_file(&path)
        .map_err(|source| ProjectOpenError::RemovePersonalLayout { path, source })
}

trait LocalLayoutFileSystem {
    fn exists(&self, path: &Path) -> bool;
    fn remove_file(&self, path: &Path) -> io::Result<()>;
}

struct StdLocalLayoutFileSystem;

impl LocalLayoutFileSystem for StdLocalLayoutFileSystem {
    fn exists(&self, path: &Path) -> bool {
        path.exists()
    }

    fn remove_file(&self, path: &Path) -> io::Result<()> {
        fs::remove_file(path)
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
    let layout_file = paths.project_layout_file(project_path);
    let project_config_dir = layout_file
        .parent()
        .expect("project layout path must have a parent");
    fs::create_dir_all(project_config_dir).map_err(|source| {
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
    let mut file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
            return if path.is_file() {
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
        let _ = fs::remove_file(path);
        return Err(error);
    }
    Ok(true)
}

pub fn export_project_layout(
    paths: &AppConfigPaths,
    project_path: &Path,
    layout: &ProjectLayout,
) -> Result<PathBuf, ProjectOpenError> {
    let path = paths.project_layout_file(project_path);
    write_project_layout(&path, layout)?;
    Ok(path)
}

pub fn merge_layouts(
    base: &ProjectLayout,
    local_override: &LayoutOverride,
) -> Result<LayoutMerge, LayoutError> {
    let mut layout = base.clone();
    let mut warnings = Vec::new();

    if let Some(project_override) = &local_override.project {
        if let Some(name) = &project_override.name {
            layout.project.name = name.clone();
        }
        if let Some(default_tab) = &project_override.default_tab {
            layout.project.default_tab = Some(default_tab.clone());
        }
    }

    for tab_override in &local_override.tabs {
        let Some(tab) = layout.tabs.iter_mut().find(|tab| tab.id == tab_override.id) else {
            warnings.push(MergeWarning::StaleTabOverride(tab_override.id.clone()));
            continue;
        };

        if let Some(title) = &tab_override.title {
            tab.title = title.clone();
        }
        if let Some(cwd) = &tab_override.cwd {
            tab.cwd = Some(cwd.clone());
        }
        if let Some(startup) = tab_override.startup {
            tab.startup = startup;
        }

        if let Some(layout_override) = &tab_override.layout {
            match layout_override {
                LayoutNodeOverride::Pane(pane_override) => {
                    if !apply_pane_override(&mut tab.layout, pane_override) {
                        warnings.push(MergeWarning::StalePaneOverride(pane_override.id.clone()));
                    }
                }
            }
        }
    }

    layout.validate()?;

    Ok(LayoutMerge { layout, warnings })
}

fn apply_pane_override(layout: &mut LayoutNode, pane_override: &PaneOverride) -> bool {
    let Some(pane) = layout.find_pane_mut(&pane_override.id) else {
        return false;
    };

    merge_pane(pane, pane_override);
    true
}

fn merge_pane(pane: &mut PaneConfig, pane_override: &PaneOverride) {
    if let Some(title) = &pane_override.title {
        pane.title = title.clone();
    }
    if let Some(command) = &pane_override.command {
        pane.command = command.clone();
    }
    if let Some(args) = &pane_override.args {
        pane.args = args.clone();
    }
    if let Some(execution_mode) = pane_override.execution_mode {
        pane.execution_mode = execution_mode;
    }
    if let Some(exit_behavior) = pane_override.exit_behavior {
        pane.exit_behavior = exit_behavior;
    }
    if let Some(kind) = &pane_override.kind {
        pane.kind = kind.clone();
    }
    if let Some(notify_on_exit) = pane_override.notify_on_exit {
        pane.notify_on_exit = notify_on_exit;
    }
    if let Some(detector) = &pane_override.detector {
        pane.detector = Some(detector.clone());
    }
}

fn load_project_layout(
    paths: &AppConfigPaths,
    project_path: &Path,
    default_state: &mut DefaultLayoutState,
) -> Result<LoadedProjectLayout, ProjectOpenError> {
    let project_layout_file = paths.project_layout_file(project_path);
    let (base, base_source, mut warnings) = if project_layout_file.exists() {
        (
            read_project_layout(&project_layout_file, project_path)?,
            LayoutSource::ProjectConfig(project_layout_file),
            Vec::new(),
        )
    } else {
        let _ = default_state.reload();
        (
            default_state
                .template()
                .materialize(project_name(project_path)),
            LayoutSource::GlobalDefault(paths.default_layout_file()),
            default_state.warnings().to_vec(),
        )
    };

    let local_layout_file = paths.local_layout_file(project_path);
    if !local_layout_file.exists() {
        return Ok(LoadedProjectLayout {
            layout: base,
            source: base_source,
            warnings,
        });
    }

    let source = match fs::read_to_string(&local_layout_file) {
        Ok(source) => source,
        Err(error) => {
            warnings.push(LayoutLoadWarning::PersonalOverrideRead {
                path: local_layout_file,
                message: error.to_string(),
            });
            return Ok(LoadedProjectLayout {
                layout: base,
                source: base_source,
                warnings,
            });
        }
    };

    let personal = match parse_personal_layout(&local_layout_file, &source) {
        Ok(personal) => personal,
        Err(error) => {
            warnings.push(personal_error_warning(error));
            return Ok(LoadedProjectLayout {
                layout: base,
                source: base_source,
                warnings,
            });
        }
    };

    match personal {
        PersonalLayout::Replace(layout) => Ok(LoadedProjectLayout {
            layout,
            source: LayoutSource::PersonalReplace(local_layout_file),
            warnings,
        }),
        PersonalLayout::Patch(patch) => match merge_layouts(&base, &patch) {
            Ok(merged) => {
                warnings.extend(merged.warnings.into_iter().map(|warning| match warning {
                    MergeWarning::StaleTabOverride(tab_id) => LayoutLoadWarning::StaleOverrideTab {
                        path: local_layout_file.clone(),
                        tab_id,
                    },
                    MergeWarning::StalePaneOverride(pane_id) => {
                        LayoutLoadWarning::StaleOverridePane {
                            path: local_layout_file.clone(),
                            pane_id,
                        }
                    }
                }));
                let source = match base_source {
                    LayoutSource::GlobalDefault(global) => {
                        LayoutSource::GlobalDefaultWithPersonalPatch {
                            global,
                            local: local_layout_file,
                        }
                    }
                    LayoutSource::ProjectConfig(project) => {
                        LayoutSource::ProjectConfigWithPersonalPatch {
                            project,
                            local: local_layout_file,
                        }
                    }
                    _ => unreachable!("personal patch base must be project or global"),
                };
                Ok(LoadedProjectLayout {
                    layout: merged.layout,
                    source,
                    warnings,
                })
            }
            Err(error) => {
                warnings.push(LayoutLoadWarning::PersonalOverrideValidation {
                    path: local_layout_file,
                    message: error.to_string(),
                });
                Ok(LoadedProjectLayout {
                    layout: base,
                    source: base_source,
                    warnings,
                })
            }
        },
    }
}

fn resolve_layout_variables(layout: &mut ProjectLayout, project_path: &Path) {
    for tab in &mut layout.tabs {
        if let Some(cwd) = tab.cwd.take() {
            tab.cwd = Some(resolve_cwd(cwd, project_path));
        }
    }
}

fn resolve_cwd(cwd: PathBuf, project_path: &Path) -> PathBuf {
    if let Ok(suffix) = cwd.strip_prefix(PROJECT_DIR_VARIABLE) {
        project_path.join(suffix)
    } else if cwd.is_relative() {
        project_path.join(cwd)
    } else {
        cwd
    }
}

fn project_name(project_path: &Path) -> String {
    project_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Project")
        .to_string()
}

fn personal_error_warning(error: ProjectOpenError) -> LayoutLoadWarning {
    match error {
        ProjectOpenError::PersonalOverrideParse { path, message } => {
            LayoutLoadWarning::PersonalOverrideParse { path, message }
        }
        ProjectOpenError::PersonalOverrideValidation { path, message } => {
            LayoutLoadWarning::PersonalOverrideValidation { path, message }
        }
        other => unreachable!("unexpected personal layout error: {other}"),
    }
}

#[derive(serde::Deserialize)]
struct ProjectLayoutFile {
    project: ProjectLayoutFileProject,
    #[serde(default)]
    tabs: Vec<TabConfig>,
}

#[derive(serde::Deserialize)]
struct ProjectLayoutFileProject {
    name: Option<String>,
    default_tab: Option<String>,
}

fn read_project_layout(
    path: &Path,
    project_path: &Path,
) -> Result<ProjectLayout, ProjectOpenError> {
    let source =
        fs::read_to_string(path).map_err(|source| ProjectOpenError::ReadProjectLayout {
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
    let file: ProjectLayoutFile =
        toml::from_str(source).map_err(|source| ProjectOpenError::ParseProjectLayout {
            path: path.to_path_buf(),
            source,
        })?;
    let layout = ProjectLayout {
        project: ProjectConfig {
            name: file
                .project
                .name
                .unwrap_or_else(|| project_name(project_path)),
            default_tab: file.project.default_tab,
        },
        tabs: file.tabs,
    };
    layout
        .validate()
        .map_err(|source| ProjectOpenError::InvalidProjectLayout {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(layout)
}

fn write_project_layout(path: &Path, layout: &ProjectLayout) -> Result<(), ProjectOpenError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ProjectOpenError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let source = toml::to_string_pretty(layout).map_err(|source| {
        ProjectOpenError::SerializeProjectLayout {
            path: path.to_path_buf(),
            source,
        }
    })?;
    fs::write(path, source).map_err(|source| ProjectOpenError::WriteProjectLayout {
        path: path.to_path_buf(),
        source,
    })
}

fn record_recent_project(
    paths: &AppConfigPaths,
    project_path: &Path,
    title: &str,
) -> Result<RecentProjectsConfig, ProjectOpenError> {
    let mut recent_projects = load_recent_projects(paths)?;
    recent_projects
        .projects
        .retain(|project| project.path != project_path);
    recent_projects.projects.insert(
        0,
        RecentProjectConfig {
            title: title.to_string(),
            path: project_path.to_path_buf(),
        },
    );
    recent_projects.projects.truncate(20);
    write_recent_projects(paths, &recent_projects)?;
    Ok(recent_projects)
}

pub fn save_last_opened_projects(
    paths: &AppConfigPaths,
    recent_projects: &mut RecentProjectsConfig,
    project_paths: Vec<PathBuf>,
) -> Result<(), ProjectOpenError> {
    recent_projects.last_opened_projects = project_paths;
    write_recent_projects(paths, recent_projects)
}

fn write_recent_projects(
    paths: &AppConfigPaths,
    recent_projects: &RecentProjectsConfig,
) -> Result<(), ProjectOpenError> {
    let path = paths.recent_projects_file();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ProjectOpenError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let source = toml::to_string_pretty(recent_projects).map_err(|source| {
        ProjectOpenError::SerializeRecentProjects {
            path: path.clone(),
            source,
        }
    })?;
    fs::write(&path, source)
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
        fn exists(&self, path: &Path) -> bool {
            self.files.borrow().contains(path)
        }

        fn remove_file(&self, path: &Path) -> io::Result<()> {
            if let Some(message) = self.remove_error.borrow_mut().take() {
                return Err(io::Error::new(io::ErrorKind::PermissionDenied, message));
            }
            self.files.borrow_mut().remove(path);
            Ok(())
        }
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

        assert!(file_system.exists(&path));
        assert!(matches!(
            error,
            ProjectOpenError::RemovePersonalLayout {
                path: error_path,
                source,
            } if error_path == path && source.to_string() == "remove denied"
        ));
    }
}
