use std::{fmt, path::PathBuf};

use super::ids::{ConnectionId, ProjectId, ProjectInstanceId};

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct RemotePathBuf(String);

impl RemotePathBuf {
    pub fn new(path: impl Into<String>) -> Result<Self, RemotePathError> {
        let path = path.into();
        if path.as_bytes().contains(&0) {
            return Err(RemotePathError::ContainsNul);
        }
        if !path.starts_with('/') {
            return Err(RemotePathError::NotAbsolute(path));
        }

        let mut components = Vec::new();
        for component in path.split('/') {
            match component {
                "" | "." => {}
                ".." => {
                    components.pop();
                }
                component => components.push(component),
            }
        }
        let normalized = if components.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", components.join("/"))
        };
        Ok(Self(normalized))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').find(|component| !component.is_empty())
    }

    pub fn join_relative(&self, relative: &RemoteRelativePathBuf) -> Self {
        if relative.as_str().is_empty() {
            return self.clone();
        }
        let joined = if self.0 == "/" {
            format!("/{}", relative.as_str())
        } else {
            format!("{}/{}", self.0, relative.as_str())
        };
        Self(joined)
    }
}

impl fmt::Display for RemotePathBuf {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
#[serde(transparent)]
pub struct RemoteRelativePathBuf(String);

impl RemoteRelativePathBuf {
    pub fn new(path: impl Into<String>) -> Result<Self, RemotePathError> {
        let path = path.into();
        if path.as_bytes().contains(&0) {
            return Err(RemotePathError::ContainsNul);
        }
        if path.starts_with('/') {
            return Err(RemotePathError::NotRelative(path));
        }

        let mut components = Vec::new();
        for component in path.split('/') {
            match component {
                "" | "." => {}
                ".." => return Err(RemotePathError::ParentTraversal),
                component => components.push(component),
            }
        }
        Ok(Self(components.join("/")))
    }

    pub fn root() -> Self {
        Self::default()
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    pub fn file_name(&self) -> Option<&str> {
        self.0.rsplit('/').find(|component| !component.is_empty())
    }

    pub fn parent(&self) -> Self {
        match self.0.rsplit_once('/') {
            Some((parent, _)) => Self(parent.to_string()),
            None => Self::root(),
        }
    }

    pub fn join(&self, name: &str) -> Result<Self, RemotePathError> {
        if name.is_empty() || name == "." || name == ".." || name.contains('/') {
            return Err(RemotePathError::InvalidComponent(name.to_string()));
        }
        Self::new(if self.0.is_empty() {
            name.to_string()
        } else {
            format!("{}/{}", self.0, name)
        })
    }
}

impl fmt::Display for RemoteRelativePathBuf {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ProjectLocation {
    Local {
        path: PathBuf,
    },
    Ssh {
        connection_id: ConnectionId,
        root: RemotePathBuf,
    },
}

impl ProjectLocation {
    pub fn local(path: impl Into<PathBuf>) -> Self {
        Self::Local { path: path.into() }
    }

    pub fn local_path(&self) -> Option<&PathBuf> {
        match self {
            Self::Local { path } => Some(path),
            Self::Ssh { .. } => None,
        }
    }

    pub fn display_path(&self) -> String {
        match self {
            Self::Local { path } => path.display().to_string(),
            Self::Ssh {
                connection_id,
                root,
            } => format!("ssh://{}{root}", connection_id.as_str()),
        }
    }

    pub fn fallback_title(&self) -> String {
        match self {
            Self::Local { path } => path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .filter(|name| !name.is_empty())
                .unwrap_or_else(|| path.display().to_string()),
            Self::Ssh { root, .. } => root
                .file_name()
                .filter(|name| !name.is_empty())
                .unwrap_or(root.as_str())
                .to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProjectDescriptor {
    pub id: ProjectId,
    pub instance_id: ProjectInstanceId,
    pub location: ProjectLocation,
}

impl ProjectDescriptor {
    pub fn new(id: ProjectId, location: ProjectLocation) -> Self {
        Self {
            id,
            instance_id: ProjectInstanceId::random(),
            location,
        }
    }
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum RemotePathError {
    #[error("remote path contains a NUL byte")]
    ContainsNul,
    #[error("remote path is not absolute: {0}")]
    NotAbsolute(String),
    #[error("remote path is not relative: {0}")]
    NotRelative(String),
    #[error("remote relative path cannot contain parent traversal")]
    ParentTraversal,
    #[error("invalid remote path component: {0}")]
    InvalidComponent(String),
}

#[cfg(test)]
mod tests {
    use super::{RemotePathBuf, RemoteRelativePathBuf};

    #[test]
    fn remote_paths_normalize_without_host_path_rules() {
        assert_eq!(
            RemotePathBuf::new("/srv/./api/../web").unwrap().as_str(),
            "/srv/web"
        );
        assert_eq!(
            RemoteRelativePathBuf::new("src//main.rs").unwrap().as_str(),
            "src/main.rs"
        );
        assert!(RemoteRelativePathBuf::new("../secret").is_err());
    }
}
