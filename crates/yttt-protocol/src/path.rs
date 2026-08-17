use std::ffi::OsString;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "PathSegmentWire")]
pub enum PathSegment {
    Utf8(String),
    Bytes(#[serde(with = "serde_bytes")] Vec<u8>),
}

/// Mirrors [`PathSegment`] on the wire so decoding runs the same validation as the
/// constructors; a peer must not be able to smuggle `..` past `utf8`/`bytes`.
#[derive(Deserialize)]
enum PathSegmentWire {
    Utf8(String),
    Bytes(#[serde(with = "serde_bytes")] Vec<u8>),
}

impl TryFrom<PathSegmentWire> for PathSegment {
    type Error = ProjectPathError;

    fn try_from(wire: PathSegmentWire) -> Result<Self, Self::Error> {
        match wire {
            PathSegmentWire::Utf8(text) => Self::utf8(text),
            PathSegmentWire::Bytes(bytes) => Self::bytes(bytes),
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProjectRelativePath {
    pub segments: Vec<PathSegment>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HostPath {
    pub volume: Option<String>,
    pub segments: Vec<PathSegment>,
}

#[derive(Clone, Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProjectPathError {
    #[error("path contains a NUL byte")]
    ContainsNul,
    #[error("path is not relative")]
    NotRelative,
    #[error("path is not absolute")]
    NotAbsolute,
    #[error("path cannot contain parent traversal")]
    ParentTraversal,
    #[error("path contains an invalid segment")]
    InvalidSegment,
    #[error("path is not valid on this Host")]
    UnsupportedPlatform,
}

impl PathSegment {
    pub fn from_os_str(name: impl AsRef<std::ffi::OsStr>) -> Result<Self, ProjectPathError> {
        let name = name.as_ref();
        if let Some(text) = name.to_str() {
            Self::utf8(text)
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStrExt as _;
                Self::bytes(name.as_bytes().to_vec())
            }
            #[cfg(not(unix))]
            {
                let _ = name;
                Err(ProjectPathError::InvalidSegment)
            }
        }
    }

    pub fn utf8(text: impl Into<String>) -> Result<Self, ProjectPathError> {
        let text = text.into();
        validate_utf8_segment(&text)?;
        Ok(Self::Utf8(text))
    }

    pub fn bytes(bytes: Vec<u8>) -> Result<Self, ProjectPathError> {
        validate_bytes_segment(&bytes)?;
        Ok(Self::Bytes(bytes))
    }

    pub fn to_os_string(&self) -> OsString {
        match self {
            Self::Utf8(text) => OsString::from(text),
            Self::Bytes(bytes) => {
                #[cfg(unix)]
                {
                    use std::os::unix::ffi::OsStringExt as _;
                    OsString::from_vec(bytes.clone())
                }
                #[cfg(not(unix))]
                {
                    OsString::from(String::from_utf8_lossy(bytes).into_owned())
                }
            }
        }
    }
}

impl ProjectRelativePath {
    pub fn root() -> Self {
        Self::default()
    }

    pub fn from_utf8(path: &str) -> Result<Self, ProjectPathError> {
        if path.contains('\0') {
            return Err(ProjectPathError::ContainsNul);
        }
        if is_absolute_utf8(path) {
            return Err(ProjectPathError::NotRelative);
        }
        let mut segments = Vec::new();
        for component in path.split(['/', '\\']) {
            match component {
                "" | "." => {}
                ".." => return Err(ProjectPathError::ParentTraversal),
                component => segments.push(PathSegment::utf8(component)?),
            }
        }
        Ok(Self { segments })
    }

    pub fn from_path(path: &Path) -> Result<Self, ProjectPathError> {
        if path.is_absolute() {
            return Err(ProjectPathError::NotRelative);
        }
        let mut segments = Vec::new();
        for component in path.components() {
            match component {
                Component::Prefix(_) | Component::RootDir => {
                    return Err(ProjectPathError::NotRelative);
                }
                Component::CurDir => {}
                Component::ParentDir => return Err(ProjectPathError::ParentTraversal),
                Component::Normal(name) => segments.push(PathSegment::from_os_str(name)?),
            }
        }
        Ok(Self { segments })
    }

    pub fn join_under(&self, root: &Path) -> PathBuf {
        let mut path = root.to_path_buf();
        for segment in &self.segments {
            path.push(segment.to_os_string());
        }
        path
    }

    pub fn to_utf8(&self) -> Result<String, ProjectPathError> {
        let mut parts = Vec::with_capacity(self.segments.len());
        for segment in &self.segments {
            match segment {
                PathSegment::Utf8(text) => parts.push(text.as_str()),
                PathSegment::Bytes(_) => return Err(ProjectPathError::InvalidSegment),
            }
        }
        Ok(parts.join("/"))
    }
}

impl HostPath {
    pub fn from_path(path: &Path) -> Result<Self, ProjectPathError> {
        if !path.is_absolute() {
            return Err(ProjectPathError::NotAbsolute);
        }
        let mut volume = None;
        let mut segments = Vec::new();
        for component in path.components() {
            match component {
                Component::Prefix(prefix) => {
                    volume = Some(prefix.as_os_str().to_string_lossy().into_owned());
                }
                Component::RootDir | Component::CurDir => {}
                Component::ParentDir => return Err(ProjectPathError::ParentTraversal),
                Component::Normal(name) => segments.push(PathSegment::from_os_str(name)?),
            }
        }
        Ok(Self { volume, segments })
    }

    pub fn to_path(&self) -> Result<PathBuf, ProjectPathError> {
        #[cfg(unix)]
        {
            if self.volume.is_some() {
                return Err(ProjectPathError::UnsupportedPlatform);
            }
            let mut path = PathBuf::from("/");
            for segment in &self.segments {
                path.push(segment.to_os_string());
            }
            Ok(path)
        }
        #[cfg(windows)]
        {
            let volume = self
                .volume
                .as_deref()
                .ok_or(ProjectPathError::UnsupportedPlatform)?;
            let mut path = PathBuf::from(volume);
            path.push("\\");
            for segment in &self.segments {
                path.push(segment.to_os_string());
            }
            Ok(path)
        }
        #[cfg(not(any(unix, windows)))]
        {
            Err(ProjectPathError::UnsupportedPlatform)
        }
    }
}

fn is_absolute_utf8(path: &str) -> bool {
    path.starts_with('/')
        || path.starts_with('\\')
        || path
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() && path[1..].starts_with(':'))
}

fn validate_utf8_segment(text: &str) -> Result<(), ProjectPathError> {
    if text.is_empty()
        || text == "."
        || text == ".."
        || text.contains('\0')
        || text.contains('/')
        || text.contains('\\')
    {
        return Err(if text == ".." {
            ProjectPathError::ParentTraversal
        } else if text.contains('\0') {
            ProjectPathError::ContainsNul
        } else {
            ProjectPathError::InvalidSegment
        });
    }
    Ok(())
}

fn validate_bytes_segment(bytes: &[u8]) -> Result<(), ProjectPathError> {
    if bytes.is_empty()
        || bytes == b"."
        || bytes == b".."
        || bytes.contains(&0)
        || bytes.contains(&b'/')
        || bytes.contains(&b'\\')
    {
        return Err(if bytes == b".." {
            ProjectPathError::ParentTraversal
        } else if bytes.contains(&0) {
            ProjectPathError::ContainsNul
        } else {
            ProjectPathError::InvalidSegment
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{HostPath, PathSegment, ProjectPathError, ProjectRelativePath};

    fn decode_relative(segments: Vec<PathSegment>) -> Result<ProjectRelativePath, String> {
        let mut encoded = Vec::new();
        ciborium::into_writer(&ProjectRelativePath { segments }, &mut encoded).unwrap();
        ciborium::from_reader::<ProjectRelativePath, _>(encoded.as_slice())
            .map_err(|error| error.to_string())
    }

    #[test]
    fn relative_path_rejects_absolute_parent_and_nul() {
        assert_eq!(
            ProjectRelativePath::from_utf8("/etc/passwd"),
            Err(ProjectPathError::NotRelative)
        );
        assert_eq!(
            ProjectRelativePath::from_utf8("C:\\secret"),
            Err(ProjectPathError::NotRelative)
        );
        assert_eq!(
            ProjectRelativePath::from_utf8("../secret"),
            Err(ProjectPathError::ParentTraversal)
        );
        assert_eq!(
            ProjectRelativePath::from_utf8("src/\0main.rs"),
            Err(ProjectPathError::ContainsNul)
        );
        assert_eq!(
            ProjectRelativePath::from_utf8("src/./lib.rs")
                .unwrap()
                .to_utf8()
                .unwrap(),
            "src/lib.rs"
        );
    }

    #[test]
    fn decoding_rejects_segments_the_constructors_would_reject() {
        for smuggled in [
            PathSegment::Utf8("..".to_string()),
            PathSegment::Utf8("../etc".to_string()),
            PathSegment::Utf8(String::new()),
            PathSegment::Utf8("a\0b".to_string()),
            PathSegment::Bytes(b"..".to_vec()),
            PathSegment::Bytes(b"..\\windows".to_vec()),
        ] {
            assert!(
                decode_relative(vec![smuggled.clone()]).is_err(),
                "{smuggled:?} must not survive decoding"
            );
        }
        assert_eq!(
            decode_relative(vec![
                PathSegment::Utf8("src".to_string()),
                PathSegment::Utf8("lib.rs".to_string()),
            ])
            .unwrap(),
            ProjectRelativePath::from_utf8("src/lib.rs").unwrap()
        );
    }

    #[test]
    fn host_path_round_trips_absolute_paths() {
        let path = std::env::temp_dir();
        let encoded = HostPath::from_path(&path).unwrap();
        assert_eq!(encoded.to_path().unwrap(), path);
    }
}
