//! A project's own layout file (`.yttt/layout.toml`) and its path variables.

use std::path::{Path, PathBuf};

use yttt_core::model::layout::{LayoutError, ProjectConfig, ProjectLayout, TabConfig};

pub const PROJECT_DIR_VARIABLE: &str = "<ProjectDir>";

/// Why a project layout file cannot be used.
#[derive(Debug, thiserror::Error)]
pub enum ProjectLayoutError {
    #[error(transparent)]
    Parse(toml::de::Error),
    #[error(transparent)]
    Invalid(LayoutError),
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

/// Parses and validates a project layout file; a layout without a name
/// takes the project directory's.
pub fn parse_project_layout(
    source: &str,
    project_path: &Path,
) -> Result<ProjectLayout, ProjectLayoutError> {
    let file: ProjectLayoutFile = toml::from_str(source).map_err(ProjectLayoutError::Parse)?;
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
    layout.validate().map_err(ProjectLayoutError::Invalid)?;
    Ok(layout)
}

/// Resolves each tab's `cwd` against the project root: `<ProjectDir>`
/// and relative paths become paths under it.
pub fn resolve_layout_variables(layout: &mut ProjectLayout, project_path: &Path) {
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

/// The project's directory name, the name a layout without one gets.
pub fn project_name(project_path: &Path) -> String {
    project_path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or("Project")
        .to_string()
}
