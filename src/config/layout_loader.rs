use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    config::paths::AppConfigPaths,
    model::layout::{
        LayoutError, LayoutNode, PaneConfig, PaneKind, ProjectConfig, ProjectLayout, TabConfig,
    },
};

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
    ProjectConfig(PathBuf),
    ProjectConfigWithAppOverride { project: PathBuf, local: PathBuf },
    AppLocalConfig(PathBuf),
    CreatedAppLocalDefault(PathBuf),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ProjectOpenConfig {
    pub path: PathBuf,
    pub layout: ProjectLayout,
    pub layout_source: LayoutSource,
    pub recent_projects: RecentProjectsConfig,
}

#[derive(Clone, Debug, Default, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct RecentProjectsConfig {
    #[serde(default)]
    pub projects: Vec<RecentProjectConfig>,
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

pub fn open_project_config(
    paths: &AppConfigPaths,
    project_path: &Path,
) -> Result<ProjectOpenConfig, ProjectOpenError> {
    let project_path =
        project_path
            .canonicalize()
            .map_err(|source| ProjectOpenError::OpenProjectDirectory {
                path: project_path.to_path_buf(),
                source,
            })?;
    if !project_path.is_dir() {
        return Err(ProjectOpenError::NotDirectory(project_path));
    }

    let (layout, layout_source) = load_project_layout(paths, &project_path)?;
    let recent_projects = record_recent_project(paths, &project_path, &layout.project.name)?;

    Ok(ProjectOpenConfig {
        path: project_path,
        layout,
        layout_source,
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
    write_project_layout(&path, layout)?;
    Ok(path)
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
) -> Result<(ProjectLayout, LayoutSource), ProjectOpenError> {
    let project_layout_file = paths.project_layout_file(project_path);
    if project_layout_file.exists() {
        let layout = read_project_layout(&project_layout_file)?;
        let local_layout_file = paths.local_layout_file(project_path);
        if local_layout_file.exists() {
            if let Some((layout, layout_source)) =
                read_local_project_override(&layout, &project_layout_file, &local_layout_file)?
            {
                return Ok((layout, layout_source));
            }
        }

        return Ok((layout, LayoutSource::ProjectConfig(project_layout_file)));
    }

    let local_layout_file = paths.local_layout_file(project_path);
    if local_layout_file.exists() {
        let layout = read_project_layout(&local_layout_file)?;
        return Ok((layout, LayoutSource::AppLocalConfig(local_layout_file)));
    }

    let layout = default_project_layout(project_path);
    write_default_layout(&local_layout_file, &layout)?;
    Ok((
        layout,
        LayoutSource::CreatedAppLocalDefault(local_layout_file),
    ))
}

fn read_project_layout(path: &Path) -> Result<ProjectLayout, ProjectOpenError> {
    let source =
        fs::read_to_string(path).map_err(|source| ProjectOpenError::ReadProjectLayout {
            path: path.to_path_buf(),
            source,
        })?;
    parse_project_layout(path, &source)
}

fn read_local_project_override(
    base: &ProjectLayout,
    project_layout_file: &Path,
    local_layout_file: &Path,
) -> Result<Option<(ProjectLayout, LayoutSource)>, ProjectOpenError> {
    let source = match fs::read_to_string(local_layout_file) {
        Ok(source) => source,
        Err(_error) => return Ok(None),
    };

    if let Ok(local_override) = toml::from_str::<LayoutOverride>(&source) {
        let merged = merge_layouts(base, &local_override).map_err(|source| {
            ProjectOpenError::InvalidProjectLayout {
                path: local_layout_file.to_path_buf(),
                source,
            }
        })?;
        return Ok(Some((
            merged.layout,
            LayoutSource::ProjectConfigWithAppOverride {
                project: project_layout_file.to_path_buf(),
                local: local_layout_file.to_path_buf(),
            },
        )));
    }

    if let Ok(layout) = parse_project_layout(local_layout_file, &source) {
        return Ok(Some((
            layout,
            LayoutSource::AppLocalConfig(local_layout_file.to_path_buf()),
        )));
    }

    Ok(None)
}

fn parse_project_layout(path: &Path, source: &str) -> Result<ProjectLayout, ProjectOpenError> {
    let layout: ProjectLayout =
        toml::from_str(&source).map_err(|source| ProjectOpenError::ParseProjectLayout {
            path: path.to_path_buf(),
            source,
        })?;
    layout
        .validate()
        .map_err(|source| ProjectOpenError::InvalidProjectLayout {
            path: path.to_path_buf(),
            source,
        })?;

    Ok(layout)
}

fn write_default_layout(path: &Path, layout: &ProjectLayout) -> Result<(), ProjectOpenError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| ProjectOpenError::CreateConfigDirectory {
            path: parent.to_path_buf(),
            source,
        })?;
    }

    let source = toml::to_string_pretty(layout).map_err(|source| {
        ProjectOpenError::SerializeDefaultLayout {
            path: path.to_path_buf(),
            source,
        }
    })?;
    fs::write(path, source).map_err(|source| ProjectOpenError::WriteDefaultLayout {
        path: path.to_path_buf(),
        source,
    })
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

fn default_project_layout(project_path: &Path) -> ProjectLayout {
    let name = project_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Project")
        .to_string();

    ProjectLayout {
        project: ProjectConfig {
            name,
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
