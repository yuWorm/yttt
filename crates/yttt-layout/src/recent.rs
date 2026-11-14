//! The projects a profile opened, most recent first (`recent-projects.toml`).

use std::path::PathBuf;

use yttt_core::model::{ids::ProjectId, project::ProjectLocation};

/// How many projects the history keeps.
pub const RECENT_PROJECTS_LIMIT: usize = 20;

#[derive(Clone, Debug, serde::Deserialize, serde::Serialize, PartialEq, Eq)]
pub struct RecentProjectsConfig {
    #[serde(default = "recent_projects_version")]
    pub version: u32,
    #[serde(default)]
    pub projects: Vec<RecentProjectConfig>,
    #[serde(default)]
    pub last_opened_projects: Vec<ProjectReferenceConfig>,
    #[serde(default)]
    pub last_restorable_projects: Vec<ProjectReferenceConfig>,
}

impl Default for RecentProjectsConfig {
    fn default() -> Self {
        Self {
            version: recent_projects_version(),
            projects: Vec::new(),
            last_opened_projects: Vec::new(),
            last_restorable_projects: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct ProjectReferenceConfig {
    pub id: ProjectId,
    #[serde(flatten)]
    pub location: ProjectLocation,
}

impl ProjectReferenceConfig {
    pub fn new(id: ProjectId, location: ProjectLocation) -> Self {
        Self { id, location }
    }
}

impl<'de> serde::Deserialize<'de> for ProjectReferenceConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Representation {
            Current {
                id: ProjectId,
                #[serde(flatten)]
                location: ProjectLocation,
            },
            Legacy(PathBuf),
        }

        match Representation::deserialize(deserializer)? {
            Representation::Current { id, location } => Ok(Self { id, location }),
            Representation::Legacy(path) => {
                let location = ProjectLocation::local(path);
                let id = ProjectId::from_legacy_location(&location.display_path());
                Ok(Self { id, location })
            }
        }
    }
}

#[derive(Clone, Debug, serde::Serialize, PartialEq, Eq)]
pub struct RecentProjectConfig {
    pub id: ProjectId,
    pub title: String,
    #[serde(flatten)]
    pub location: ProjectLocation,
}

impl<'de> serde::Deserialize<'de> for RecentProjectConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        #[serde(untagged)]
        enum Representation {
            Current {
                id: ProjectId,
                title: String,
                #[serde(flatten)]
                location: ProjectLocation,
            },
            Legacy {
                title: String,
                path: PathBuf,
            },
        }

        match Representation::deserialize(deserializer)? {
            Representation::Current {
                id,
                title,
                location,
            } => Ok(Self {
                id,
                title,
                location,
            }),
            Representation::Legacy { title, path } => {
                let location = ProjectLocation::local(path);
                let id = ProjectId::from_legacy_location(&location.display_path());
                Ok(Self {
                    id,
                    title,
                    location,
                })
            }
        }
    }
}

fn recent_projects_version() -> u32 {
    1
}

impl RecentProjectsConfig {
    /// Parses `recent-projects.toml`.
    pub fn parse(source: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(source)
    }

    /// Moves `location` to the front of the history under `title` and gives
    /// its project ID: the one it had, or a new one.
    pub fn record(&mut self, location: &ProjectLocation, title: &str) -> ProjectId {
        let project_id = self
            .projects
            .iter()
            .find(|project| &project.location == location)
            .map(|project| project.id.clone())
            .unwrap_or_else(ProjectId::random);
        self.projects
            .retain(|project| &project.location != location);
        self.projects.insert(
            0,
            RecentProjectConfig {
                id: project_id.clone(),
                title: title.to_string(),
                location: location.clone(),
            },
        );
        self.projects.truncate(RECENT_PROJECTS_LIMIT);
        project_id
    }
}
