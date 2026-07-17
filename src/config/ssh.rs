use std::{fs, io, path::PathBuf};

use yttt_core::model::{
    ids::{ConnectionId, CredentialId},
    project::RemotePathBuf,
};

use super::paths::AppConfigPaths;

pub const SSH_CONNECTIONS_CONFIG_VERSION: u32 = 1;

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SshConnectionsConfig {
    #[serde(default = "default_version")]
    pub version: u32,
    #[serde(default)]
    pub connections: Vec<SshConnectionConfig>,
}

impl Default for SshConnectionsConfig {
    fn default() -> Self {
        Self {
            version: SSH_CONNECTIONS_CONFIG_VERSION,
            connections: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct SshConnectionConfig {
    pub id: ConnectionId,
    pub name: String,
    pub host: String,
    #[serde(default = "default_port")]
    pub port: u16,
    pub user: String,
    #[serde(default)]
    pub identity_file: Option<PathBuf>,
    #[serde(default)]
    pub auth: SshAuthPreference,
    #[serde(default)]
    pub credential: Option<CredentialRef>,
    #[serde(default)]
    pub default_remote_root: Option<RemotePathBuf>,
}

impl SshConnectionConfig {
    pub fn new(
        name: impl Into<String>,
        host: impl Into<String>,
        port: u16,
        user: impl Into<String>,
    ) -> Self {
        Self {
            id: ConnectionId::random(),
            name: name.into(),
            host: host.into(),
            port,
            user: user.into(),
            identity_file: None,
            auth: SshAuthPreference::default(),
            credential: None,
            default_remote_root: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SshAuthPreference {
    #[default]
    Auto,
    Agent,
    PublicKey,
    Password,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct CredentialRef {
    pub id: CredentialId,
    pub kind: CredentialKind,
    pub binding: CredentialBinding,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialKind {
    LoginPassword,
    KeyPassphrase,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct CredentialBinding {
    pub connection_id: ConnectionId,
    pub effective_user: String,
    pub resolved_host: String,
    pub port: u16,
    pub host_key_sha256: String,
    #[serde(default)]
    pub private_key_identity: Option<String>,
}

impl CredentialBinding {
    pub fn matches_endpoint(
        &self,
        connection_id: &ConnectionId,
        effective_user: &str,
        resolved_host: &str,
        port: u16,
        host_key_sha256: &str,
        private_key_identity: Option<&str>,
    ) -> bool {
        &self.connection_id == connection_id
            && self.effective_user == effective_user
            && self.resolved_host == resolved_host
            && self.port == port
            && self.host_key_sha256 == host_key_sha256
            && self.private_key_identity.as_deref() == private_key_identity
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SshConnectionsConfigError {
    #[error("failed to read SSH connections from {path}: {source}")]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("failed to parse SSH connections from {path}: {source}")]
    Parse {
        path: PathBuf,
        #[source]
        source: toml::de::Error,
    },
    #[error("unsupported SSH connections config version {version} in {path}")]
    UnsupportedVersion { path: PathBuf, version: u32 },
    #[error("failed to serialize SSH connections: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("failed to write SSH connections to {path}: {source}")]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

pub fn load_ssh_connections(
    paths: &AppConfigPaths,
) -> Result<SshConnectionsConfig, SshConnectionsConfigError> {
    let path = paths.ssh_connections_file();
    if !path.exists() {
        return Ok(SshConnectionsConfig::default());
    }
    let source = fs::read_to_string(&path).map_err(|source| SshConnectionsConfigError::Read {
        path: path.clone(),
        source,
    })?;
    let config: SshConnectionsConfig =
        toml::from_str(&source).map_err(|source| SshConnectionsConfigError::Parse {
            path: path.clone(),
            source,
        })?;
    if config.version != SSH_CONNECTIONS_CONFIG_VERSION {
        return Err(SshConnectionsConfigError::UnsupportedVersion {
            path,
            version: config.version,
        });
    }
    Ok(config)
}

pub fn save_ssh_connections(
    paths: &AppConfigPaths,
    config: &SshConnectionsConfig,
) -> Result<(), SshConnectionsConfigError> {
    let path = paths.ssh_connections_file();
    let source = toml::to_string_pretty(config)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| SshConnectionsConfigError::Write {
            path: parent.to_path_buf(),
            source,
        })?;
    }
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("ssh-connections.toml");
    let temp_path = path.with_file_name(format!(".{file_name}.tmp"));
    fs::write(&temp_path, source).map_err(|source| SshConnectionsConfigError::Write {
        path: temp_path.clone(),
        source,
    })?;
    fs::OpenOptions::new()
        .write(true)
        .open(&temp_path)
        .and_then(|file| file.sync_all())
        .map_err(|source| SshConnectionsConfigError::Write {
            path: temp_path.clone(),
            source,
        })?;
    fs::rename(&temp_path, &path)
        .map_err(|source| SshConnectionsConfigError::Write { path, source })
}

fn default_version() -> u32 {
    SSH_CONNECTIONS_CONFIG_VERSION
}

fn default_port() -> u16 {
    22
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{
        CredentialBinding, CredentialKind, CredentialRef, SshAuthPreference, SshConnectionConfig,
        SshConnectionsConfig, load_ssh_connections, save_ssh_connections,
    };
    use crate::{
        config::paths::AppConfigPaths,
        model::ids::{ConnectionId, CredentialId},
    };

    #[test]
    fn ssh_connection_config_never_serializes_a_password() {
        let temp = tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let config = SshConnectionsConfig {
            connections: vec![SshConnectionConfig::new("dev", "localhost", 22, "alice")],
            ..SshConnectionsConfig::default()
        };

        save_ssh_connections(&paths, &config).unwrap();
        let source = std::fs::read_to_string(paths.ssh_connections_file()).unwrap();
        assert!(!source.contains("password ="));
        assert_eq!(load_ssh_connections(&paths).unwrap(), config);
    }

    #[test]
    fn stored_credential_binding_requires_the_verified_endpoint() {
        let connection_id = ConnectionId::new("dev");
        let binding = CredentialBinding {
            connection_id: connection_id.clone(),
            effective_user: "alice".to_string(),
            resolved_host: "dev.example.com".to_string(),
            port: 2222,
            host_key_sha256: "SHA256:trusted".to_string(),
            private_key_identity: None,
        };
        assert!(binding.matches_endpoint(
            &connection_id,
            "alice",
            "dev.example.com",
            2222,
            "SHA256:trusted",
            None,
        ));
        assert!(!binding.matches_endpoint(
            &connection_id,
            "alice",
            "dev.example.com",
            2222,
            "SHA256:changed",
            None,
        ));
    }

    #[test]
    fn credential_metadata_round_trips_without_secret_material() {
        let temp = tempdir().unwrap();
        let paths = AppConfigPaths::from_config_dir(temp.path());
        let mut connection = SshConnectionConfig::new("dev", "dev.example.com", 2222, "alice");
        connection.auth = SshAuthPreference::Password;
        connection.credential = Some(CredentialRef {
            id: CredentialId::new("credential-id"),
            kind: CredentialKind::LoginPassword,
            binding: CredentialBinding {
                connection_id: connection.id.clone(),
                effective_user: "alice".to_string(),
                resolved_host: "dev.example.com".to_string(),
                port: 2222,
                host_key_sha256: "SHA256:trusted".to_string(),
                private_key_identity: None,
            },
        });
        let config = SshConnectionsConfig {
            connections: vec![connection],
            ..SshConnectionsConfig::default()
        };

        save_ssh_connections(&paths, &config).unwrap();
        let source = std::fs::read_to_string(paths.ssh_connections_file()).unwrap();
        assert!(!source.contains("password ="));
        assert!(!source.contains("secret"));
        assert_eq!(load_ssh_connections(&paths).unwrap(), config);
    }
}
