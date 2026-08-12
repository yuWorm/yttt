use std::{
    fmt,
    path::{Path, PathBuf},
};

use crate::model::ids::ProfileId;

use super::paths::{AppConfigPaths, native_config_dir};

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum EnvironmentKind {
    Production,
    Development,
    Test,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProfilePersistence {
    Persistent,
    Ephemeral,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProjectConfigPolicy {
    Normal,
    Overlay,
    ReadOnly,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostConnectPolicy {
    ProfileDiscovery,
    ExplicitEndpoint(PathBuf),
}

impl HostConnectPolicy {
    pub fn allows_discovery(&self) -> bool {
        matches!(self, Self::ProfileDiscovery)
    }

    pub fn allows_auto_spawn(&self) -> bool {
        matches!(self, Self::ProfileDiscovery)
    }

    pub fn explicit_endpoint(&self) -> Option<&Path> {
        match self {
            Self::ProfileDiscovery => None,
            Self::ExplicitEndpoint(endpoint) => Some(endpoint),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct AppProfilePaths {
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
    pub cache: PathBuf,
    pub runtime: PathBuf,
    pub logs: PathBuf,
}

impl AppProfilePaths {
    pub fn scoped(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            config: root.join("config"),
            data: root.join("data"),
            state: root.join("state"),
            cache: root.join("cache"),
            runtime: root.join("runtime"),
            logs: root.join("logs"),
        }
    }

    pub fn all_roots(&self) -> [&Path; 6] {
        [
            &self.config,
            &self.data,
            &self.state,
            &self.cache,
            &self.runtime,
            &self.logs,
        ]
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct AgentSessionRoots {
    pub codex: PathBuf,
    pub claude: PathBuf,
    pub pi: PathBuf,
    pub omp: PathBuf,
}

impl AgentSessionRoots {
    pub fn scoped(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        Self {
            codex: root.join("codex"),
            claude: root.join("claude"),
            pi: root.join("pi/sessions"),
            omp: root.join("omp/sessions"),
        }
    }

    fn native() -> Option<Self> {
        let home = non_empty_env_path("HOME").or_else(|| non_empty_env_path("USERPROFILE"))?;
        let pi_override = non_empty_env_path("PI_CODING_AGENT_SESSION_DIR");
        let pi_root =
            non_empty_env_path("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".pi/agent"));
        let omp_root =
            non_empty_env_path("PI_CODING_AGENT_DIR").unwrap_or_else(|| home.join(".omp/agent"));

        Some(Self {
            codex: non_empty_env_path("CODEX_HOME").unwrap_or_else(|| home.join(".codex")),
            claude: non_empty_env_path("CLAUDE_CONFIG_DIR").unwrap_or_else(|| home.join(".claude")),
            pi: pi_override
                .clone()
                .unwrap_or_else(|| pi_root.join("sessions")),
            omp: pi_override.unwrap_or_else(|| omp_root.join("sessions")),
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentSessionReadPolicy {
    Native,
    ExplicitOnly,
    Disabled,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct AgentSessionAccess {
    policy: AgentSessionReadPolicy,
    roots: Option<AgentSessionRoots>,
}

impl AgentSessionAccess {
    pub fn native() -> Self {
        match AgentSessionRoots::native() {
            Some(roots) => Self {
                policy: AgentSessionReadPolicy::Native,
                roots: Some(roots),
            },
            None => Self::disabled(),
        }
    }

    pub fn explicit(roots: AgentSessionRoots) -> Self {
        Self {
            policy: AgentSessionReadPolicy::ExplicitOnly,
            roots: Some(roots),
        }
    }

    pub fn disabled() -> Self {
        Self {
            policy: AgentSessionReadPolicy::Disabled,
            roots: None,
        }
    }

    pub fn policy(&self) -> AgentSessionReadPolicy {
        self.policy
    }

    pub fn roots(&self) -> Option<&AgentSessionRoots> {
        self.roots.as_ref()
    }

    pub fn allows_native_commands(&self) -> bool {
        self.policy == AgentSessionReadPolicy::Native
    }
}

#[derive(Clone, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct AppProfile {
    id: ProfileId,
    environment: EnvironmentKind,
    persistence: ProfilePersistence,
    paths: AppProfilePaths,
    credential_namespace: String,
    project_config_policy: ProjectConfigPolicy,
    connect_policy: HostConnectPolicy,
    agent_sessions: AgentSessionAccess,
}

impl AppProfile {
    pub fn production() -> Self {
        let config = native_config_dir();
        let paths = AppProfilePaths {
            data: config.join("data"),
            state: config.join("state"),
            cache: config.join("cache"),
            runtime: config.join("runtime"),
            logs: config.join("logs"),
            config,
        };
        Self::from_parts(
            ProfileId::new("default"),
            EnvironmentKind::Production,
            ProfilePersistence::Persistent,
            paths,
            ProjectConfigPolicy::Normal,
            HostConnectPolicy::ProfileDiscovery,
            AgentSessionAccess::native(),
        )
    }

    pub fn scoped(
        id: ProfileId,
        environment: EnvironmentKind,
        persistence: ProfilePersistence,
        root: impl Into<PathBuf>,
        project_config_policy: ProjectConfigPolicy,
        connect_policy: HostConnectPolicy,
    ) -> Self {
        let root = root.into();
        let paths = AppProfilePaths::scoped(&root);
        let agent_sessions =
            AgentSessionAccess::explicit(AgentSessionRoots::scoped(root.join("agent-sessions")));
        Self::from_parts(
            id,
            environment,
            persistence,
            paths,
            project_config_policy,
            connect_policy,
            agent_sessions,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_parts(
        id: ProfileId,
        environment: EnvironmentKind,
        persistence: ProfilePersistence,
        paths: AppProfilePaths,
        project_config_policy: ProjectConfigPolicy,
        connect_policy: HostConnectPolicy,
        agent_sessions: AgentSessionAccess,
    ) -> Self {
        let credential_namespace = format!("dev.yttt.ssh.{}", id.as_str());
        Self {
            id,
            environment,
            persistence,
            paths,
            credential_namespace,
            project_config_policy,
            connect_policy,
            agent_sessions,
        }
    }

    pub fn id(&self) -> &ProfileId {
        &self.id
    }

    pub fn environment(&self) -> EnvironmentKind {
        self.environment
    }

    pub fn persistence(&self) -> ProfilePersistence {
        self.persistence
    }

    pub fn paths(&self) -> &AppProfilePaths {
        &self.paths
    }

    pub fn credential_namespace(&self) -> &str {
        &self.credential_namespace
    }

    pub fn project_config_policy(&self) -> ProjectConfigPolicy {
        self.project_config_policy
    }

    pub fn connect_policy(&self) -> &HostConnectPolicy {
        &self.connect_policy
    }

    pub fn agent_sessions(&self) -> &AgentSessionAccess {
        &self.agent_sessions
    }

    pub fn config_paths(&self) -> AppConfigPaths {
        AppConfigPaths::from_profile(self)
    }
}

impl fmt::Debug for AppProfile {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AppProfile")
            .field("id", &self.id)
            .field("environment", &self.environment)
            .field("persistence", &self.persistence)
            .field("project_config_policy", &self.project_config_policy)
            .field(
                "connect_policy",
                &match self.connect_policy {
                    HostConnectPolicy::ProfileDiscovery => "profile_discovery",
                    HostConnectPolicy::ExplicitEndpoint(_) => "explicit_endpoint",
                },
            )
            .finish_non_exhaustive()
    }
}

fn non_empty_env_path(name: &str) -> Option<PathBuf> {
    std::env::var_os(name)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}
