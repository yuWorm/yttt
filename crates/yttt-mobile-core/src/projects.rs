//! Opening a project on the Host from the phone: choosing a folder on the
//! Host, or a project from the Host's history, and opening it with the
//! layout the desktop would give it.

use std::path::{Path, PathBuf};

use yttt_core::model::{
    host_workspace::{ProjectPanel, WorkspaceIntent},
    ids::ProjectId,
    project::{ProjectDescriptor, ProjectLocation},
};
use yttt_layout::{
    compose::{PersonalSource, apply_personal},
    project::{parse_project_layout, project_name, resolve_layout_variables},
    recent::RecentProjectsConfig,
    template::{DefaultLayoutTemplate, parse_template},
};
use yttt_protocol::{
    HostPath, PathSegment, ProjectRelativePath,
    workspace::{
        WorkspaceConfig, WorkspaceConfigRevision, WorkspaceDirectoryEntryKind,
        WorkspaceProjectConfigFile, WorkspaceRequest, WorkspaceResponse, encode_project_path,
    },
};

use crate::session::{MobileError, MobileSession};

const DEFAULT_LAYOUT_FILE: &str = "default-layout.toml";
const RECENT_PROJECTS_FILE: &str = "recent-projects.toml";
/// The desktop's default project panel.
const PROJECT_PANEL: ProjectPanel = ProjectPanel {
    visible: true,
    width: 280.0,
};

/// A folder on the Host and the folders in it.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct HostDirectory {
    pub path: String,
    /// The folder above, `None` at a root.
    pub parent: Option<String>,
    pub folders: Vec<HostFolder>,
}

#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct HostFolder {
    pub name: String,
    pub path: String,
}

/// A project the Host opened before, most recent first.
#[derive(Clone, Debug, PartialEq, uniffi::Record)]
pub struct RecentProject {
    pub title: String,
    pub path: String,
    /// The project's ID when it is open now.
    pub open_project_id: Option<String>,
}

impl MobileSession {
    /// The Host user's home folder, where choosing a folder starts.
    pub async fn host_home(&self) -> Result<String, MobileError> {
        let WorkspaceResponse::Environment(environment) = self
            .workspace_request(WorkspaceRequest::Environment)
            .await?
        else {
            return Err(unexpected("environment"));
        };
        display(&environment.home)
    }

    /// The folders in `path` on the Host, without hidden ones, by name.
    pub async fn browse_host(&self, path: &str) -> Result<HostDirectory, MobileError> {
        let WorkspaceResponse::Directory(directory) = self
            .workspace_request(WorkspaceRequest::Browse {
                path: host_path(path)?,
                include_hidden: false,
            })
            .await?
        else {
            return Err(unexpected("directory"));
        };
        let mut folders = directory
            .entries
            .into_iter()
            .filter(|entry| {
                matches!(
                    entry.kind,
                    WorkspaceDirectoryEntryKind::Directory
                        | WorkspaceDirectoryEntryKind::SymlinkDirectory
                )
            })
            .map(|entry| {
                Ok(HostFolder {
                    name: entry.name.to_os_string().to_string_lossy().into_owned(),
                    path: display(&entry.path)?,
                })
            })
            .collect::<Result<Vec<_>, MobileError>>()?;
        folders.sort_by_key(|folder| folder.name.to_lowercase());
        let parent = parent_of(&directory.path)
            .map(|parent| display(&parent))
            .transpose()?;
        Ok(HostDirectory {
            path: display(&directory.path)?,
            parent,
            folders,
        })
    }

    /// The local projects in the Host's history, most recent first.
    pub async fn recent_projects(&self) -> Result<Vec<RecentProject>, MobileError> {
        let (recent, _) = self.read_recent().await?;
        let open = self
            .open_projects()
            .map(|(_, projects)| projects)
            .unwrap_or_default();
        Ok(recent
            .projects
            .into_iter()
            .filter_map(|project| {
                let path = project
                    .location
                    .local_path()?
                    .to_string_lossy()
                    .into_owned();
                let open_project_id = open
                    .iter()
                    .find(|(id, location)| *id == project.id || *location == project.location)
                    .map(|(id, _)| id.as_str().to_string());
                Some(RecentProject {
                    title: project.title,
                    path,
                    open_project_id,
                })
            })
            .collect())
    }

    /// Opens the folder at `path` as a project in the Host's window, with
    /// the layout the desktop would give it, and records it in the history.
    /// Gives the project's ID; one already open is left as it is.
    pub async fn open_project(&self, path: &str) -> Result<String, MobileError> {
        let (workspace_id, open) = self
            .open_projects()
            .ok_or_else(|| MobileError::Host("the Host has no window open".into()))?;
        let root = PathBuf::from(path);
        let location = ProjectLocation::local(&root);
        if let Some((id, _)) = open.iter().find(|(_, open)| *open == location) {
            return Ok(id.as_str().to_string());
        }
        let host_root = host_path(path)?;

        let base = match self
            .read(WorkspaceRequest::ReadProjectConfig {
                project_root: host_root,
                file: WorkspaceProjectConfigFile::Layout,
            })
            .await?
        {
            Some(file) => parse_project_layout(&text(&file)?, &root)
                .map_err(|error| MobileError::Host(format!("the project layout: {error}")))?,
            None => self
                .default_template()
                .await
                .materialize(project_name(&root)),
        };
        let personal = self
            .read(config_request(&format!(
                "projects/{}/layout.toml",
                encode_project_path(&root)
            ))?)
            .await
            .and_then(|file| file.as_ref().map(text).transpose());
        let read_error;
        let source = match &personal {
            Ok(Some(text)) => PersonalSource::Text(text),
            Ok(None) => PersonalSource::Missing,
            Err(error) => {
                read_error = error.to_string();
                PersonalSource::Unreadable(&read_error)
            }
        };
        let mut layout = apply_personal(base, source).layout;
        resolve_layout_variables(&mut layout, &root);

        let project_id = self.record_recent(&location, &layout.project.name).await?;
        let applied = self
            .workspace_request(WorkspaceRequest::Apply {
                workspace_id,
                intent: WorkspaceIntent::OpenProject {
                    descriptor: ProjectDescriptor::new(project_id.clone(), location),
                    layout,
                    panel: PROJECT_PANEL,
                },
                expected_revision: None,
            })
            .await?;
        if !matches!(applied, WorkspaceResponse::Applied { .. }) {
            return Err(unexpected("apply"));
        }
        Ok(project_id.as_str().to_string())
    }

    /// The profile's default layout, or the built-in one when it has none
    /// or it is unusable, as the desktop does.
    async fn default_template(&self) -> DefaultLayoutTemplate {
        let Ok(request) = config_request(DEFAULT_LAYOUT_FILE) else {
            return DefaultLayoutTemplate::builtin();
        };
        match self.read(request).await {
            Ok(Some(file)) => text(&file)
                .ok()
                .and_then(|source| parse_template(&source).ok())
                .unwrap_or_else(DefaultLayoutTemplate::builtin),
            _ => DefaultLayoutTemplate::builtin(),
        }
    }

    async fn read_recent(
        &self,
    ) -> Result<(RecentProjectsConfig, Option<WorkspaceConfigRevision>), MobileError> {
        match self.read(config_request(RECENT_PROJECTS_FILE)?).await? {
            None => Ok((RecentProjectsConfig::default(), None)),
            Some(file) => {
                let recent = RecentProjectsConfig::parse(&text(&file)?).map_err(|error| {
                    MobileError::Host(format!("the Host's recent projects: {error}"))
                })?;
                Ok((recent, Some(file.revision)))
            }
        }
    }

    /// Puts `location` first in the history and gives its project ID. Tries
    /// again when another Client wrote the history meanwhile.
    async fn record_recent(
        &self,
        location: &ProjectLocation,
        title: &str,
    ) -> Result<ProjectId, MobileError> {
        let mut attempts = 0;
        loop {
            let (mut recent, revision) = self.read_recent().await?;
            let project_id = recent.record(location, title);
            let bytes = toml::to_string_pretty(&recent)
                .map_err(|error| MobileError::Host(error.to_string()))?
                .into_bytes();
            let written = self
                .workspace_request(WorkspaceRequest::WriteConfig {
                    relative_path: relative(RECENT_PROJECTS_FILE)?,
                    expected_revision: revision,
                    bytes,
                })
                .await;
            match written {
                Ok(WorkspaceResponse::ConfigWritten { .. }) => return Ok(project_id),
                Ok(_) => return Err(unexpected("config write")),
                Err(error) if attempts >= 2 => return Err(error),
                Err(_) => attempts += 1,
            }
        }
    }

    async fn read(
        &self,
        request: WorkspaceRequest,
    ) -> Result<Option<WorkspaceConfig>, MobileError> {
        match self.workspace_request(request).await? {
            WorkspaceResponse::Config(file) => Ok(file),
            _ => Err(unexpected("config")),
        }
    }
}

fn config_request(path: &str) -> Result<WorkspaceRequest, MobileError> {
    Ok(WorkspaceRequest::ReadConfig {
        relative_path: relative(path)?,
    })
}

fn relative(path: &str) -> Result<ProjectRelativePath, MobileError> {
    ProjectRelativePath::from_utf8(path).map_err(|error| MobileError::Host(error.to_string()))
}

fn host_path(path: &str) -> Result<HostPath, MobileError> {
    HostPath::from_client_path(Path::new(path))
        .map_err(|error| MobileError::Host(format!("{path}: {error}")))
}

fn display(path: &HostPath) -> Result<String, MobileError> {
    path.to_client_path()
        .map(|path| path.to_string_lossy().into_owned())
        .map_err(|error| MobileError::Host(error.to_string()))
}

fn parent_of(path: &HostPath) -> Option<HostPath> {
    let mut segments: Vec<PathSegment> = path.segments.clone();
    segments.pop()?;
    Some(HostPath {
        volume: path.volume.clone(),
        segments,
    })
}

fn text(file: &WorkspaceConfig) -> Result<String, MobileError> {
    String::from_utf8(file.bytes.clone())
        .map_err(|_| MobileError::Host(format!("{:?} is not UTF-8", file.relative_path)))
}

fn unexpected(what: &str) -> MobileError {
    MobileError::Host(format!("unexpected {what} response"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_parent_of_a_folder_drops_its_last_segment() {
        let path = host_path("/Users/me/code").unwrap();
        assert_eq!(display(&parent_of(&path).unwrap()).unwrap(), "/Users/me");
        assert!(parent_of(&host_path("/").unwrap()).is_none());
    }
}
