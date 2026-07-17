use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct ProjectId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct ProjectInstanceId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct ConnectionId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct CredentialId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct TabId(String);

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Deserialize, serde::Serialize)]
pub struct PaneId(String);
macro_rules! impl_id_display {
    ($($id:ty),+ $(,)?) => {
        $(
            impl std::fmt::Display for $id {
                fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                    formatter.write_str(&self.0)
                }
            }
        )+
    };
}

impl_id_display!(
    ProjectId,
    ProjectInstanceId,
    ConnectionId,
    CredentialId,
    TabId,
    PaneId,
);

impl ProjectId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn random() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn from_legacy_location(location: &str) -> Self {
        const LEGACY_PROJECT_NAMESPACE: Uuid =
            Uuid::from_u128(0xd9e3_88e6_0a25_49e2_b0ae_9f65_67e4_2131);
        Self(Uuid::new_v5(&LEGACY_PROJECT_NAMESPACE, location.as_bytes()).to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ProjectInstanceId {
    pub fn random() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl ConnectionId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn random() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl CredentialId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }

    pub fn random() -> Self {
        Self(Uuid::new_v4().to_string())
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
