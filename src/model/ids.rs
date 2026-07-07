#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct ProjectId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct TabId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct PaneId(String);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TabId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl PaneId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::{PaneId, ProjectId, TabId};

    #[test]
    fn ids_preserve_string_values() {
        let project = ProjectId::new("/tmp/yttt");
        let tab = TabId::new("dev");
        let pane = PaneId::new("server");

        assert_eq!(project.as_str(), "/tmp/yttt");
        assert_eq!(tab.as_str(), "dev");
        assert_eq!(pane.as_str(), "server");
    }
}
