use std::{
    io::{self, Read as _, Write as _},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

use crate::{
    config::{
        profile::AppProfile,
        settings::WindowSettings,
        ssh::SshConnectionConfig,
        theme::{parse_theme_snapshot, serialize_theme_file},
    },
    ui::{
        i18n::Locale,
        theme::{AppearanceState, ThemeRuntime, UiTypography},
    },
};

const MAX_REMOTE_LAUNCH_BYTES: usize = 128 * 1024;

/// A clipboard code contains credentials. Base64 is transport encoding, not encryption.
pub(crate) const MAX_CONNECTION_CODE_BYTES: usize =
    yttt_protocol::remote_access::MAX_CONNECTION_INFO_BYTES.div_ceil(3) * 4;

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ConnectionCode {
    version: u8,
    pub address: String,
    pub connection_info: yttt_protocol::remote_access::RemoteConnectionInfo,
}

impl ConnectionCode {
    pub(crate) fn encode(
        address: String,
        connection_info: yttt_protocol::remote_access::RemoteConnectionInfo,
    ) -> Result<String, &'static str> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        validate_connection_address(&address)?;
        let code = Self {
            version: 1,
            address,
            connection_info,
        };
        let json = Zeroizing::new(
            serde_json::to_vec(&code).map_err(|_| "Cannot encode connection information")?,
        );
        if json.len() > yttt_protocol::remote_access::MAX_CONNECTION_INFO_BYTES {
            return Err("Connection information exceeds 8 KiB");
        }
        Ok(STANDARD.encode(&*json))
    }

    pub(crate) fn decode(value: &str) -> Result<Self, &'static str> {
        use base64::{Engine as _, engine::general_purpose::STANDARD};
        let value = value.trim();
        if value.len() > MAX_CONNECTION_CODE_BYTES {
            return Err("Connection code is too large");
        }
        let bytes = Zeroizing::new(
            STANDARD
                .decode(value)
                .map_err(|_| "Invalid Base64 connection code")?,
        );
        if bytes.len() > yttt_protocol::remote_access::MAX_CONNECTION_INFO_BYTES {
            return Err("Connection information exceeds 8 KiB");
        }
        let code: Self = serde_json::from_slice(&bytes).map_err(|_| "Invalid connection code")?;
        if code.version != 1 {
            return Err("Unsupported connection code version");
        }
        validate_connection_address(&code.address)?;
        Ok(code)
    }
}

pub(crate) fn validate_connection_address(address: &str) -> Result<(), &'static str> {
    let valid = address.rsplit_once(':').is_some_and(|(host, port)| {
        !host.is_empty()
            && port.parse::<u16>().is_ok_and(|port| port != 0)
            && if host.starts_with('[') {
                host.strip_prefix('[')
                    .and_then(|host| host.strip_suffix(']'))
                    .is_some_and(|host| host.parse::<std::net::Ipv6Addr>().is_ok())
            } else {
                !host.contains([':', '[', ']'])
            }
    });
    if address.len() > 1024
        || address
            .chars()
            .any(|ch| ch.is_whitespace() || ch.is_control() || matches!(ch, '/' | '\\' | '@'))
        || !valid
    {
        return Err("Enter host:port or [IPv6]:port, not a URL");
    }
    Ok(())
}

#[cfg(test)]
mod connection_code_tests {
    use super::*;
    use base64::{Engine as _, engine::general_purpose::STANDARD};
    use yttt_protocol::remote_access::RemoteConnectionInfo;

    fn info() -> RemoteConnectionInfo {
        RemoteConnectionInfo {
            environment_id: "remote-machine".into(),
            profile_id: yttt_core::model::ids::ProfileId::new("remote-profile"),
            server_name: "yttt-host.local".into(),
            certificate_der: vec![1, 2, 3, 255],
            certificate_sha256: "ab".repeat(32),
            credential_generation: 7,
            work_secret: [42; 32],
        }
    }

    #[test]
    fn connection_code_round_trip_preserves_endpoint_and_authentication() {
        let expected = info();
        let encoded =
            ConnectionCode::encode("[2001:db8::1]:43123".into(), expected.clone()).unwrap();
        let decoded = ConnectionCode::decode(&format!("\n{encoded}\n")).unwrap();
        assert_eq!(decoded.address, "[2001:db8::1]:43123");
        assert_eq!(decoded.connection_info, expected);
    }

    #[test]
    fn connection_code_rejects_malformed_oversized_and_future_payloads() {
        assert!(ConnectionCode::decode("not-base64!").is_err());
        assert!(ConnectionCode::decode(&"A".repeat(MAX_CONNECTION_CODE_BYTES + 1)).is_err());
        let future = ConnectionCode {
            version: 2,
            address: "example.test:43123".into(),
            connection_info: info(),
        };
        assert!(
            ConnectionCode::decode(&STANDARD.encode(serde_json::to_vec(&future).unwrap())).is_err()
        );
        for address in [
            "https://example.test:443",
            "host:0",
            "host:65536",
            "::1:43123",
            "host:\n22",
        ] {
            assert!(ConnectionCode::encode(address.into(), info()).is_err());
        }
    }
}

/// The rendered appearance inherited by a remote Client before it connects to the Host.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteAppearance {
    theme_file: String,
    style_id: yttt_ui::style::UiStyleId,
    typography: UiTypography,
    pub window: WindowSettings,
    pub locale: Locale,
}

impl RemoteAppearance {
    pub fn capture(cx: &gpui::App, text: crate::ui::i18n::UiText) -> Self {
        let runtime = cx.global::<AppearanceState>().runtime();
        Self {
            theme_file: serialize_theme_file(&runtime.snapshot_theme())
                .expect("appearance theme must be serializable"),
            style_id: runtime.style_id,
            typography: runtime.typography.clone(),
            window: runtime.window,
            locale: text.locale(),
        }
    }

    pub(crate) fn theme_runtime(&self) -> Result<ThemeRuntime, String> {
        let theme = parse_theme_snapshot(&self.theme_file)
            .map_err(|error| format!("invalid embedded theme snapshot: {error}"))?;
        Ok(ThemeRuntime::from_snapshot(
            theme,
            self.style_id,
            self.typography.clone(),
            self.window,
        ))
    }
}

/// The authenticated connection information passed privately from the launcher to a remote Client.
///
/// This type deliberately does not implement `Debug`: password material must never reach logs.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteLaunch {
    pub local_profile: AppProfile,
    pub appearance: RemoteAppearance,
    pub target: RemoteTarget,
}

#[derive(Clone, Deserialize, Serialize)]
#[serde(tag = "kind", deny_unknown_fields)]
pub enum RemoteTarget {
    ExistingHost {
        address: String,
        connection_info: yttt_protocol::remote_access::RemoteConnectionInfo,
    },
    SshServer {
        connection: SshConnectionConfig,
        password: Option<String>,
        passphrase: Option<String>,
        save_password_as: Option<yttt_core::model::ids::CredentialId>,
    },
}

impl RemoteTarget {
    pub fn label(&self) -> String {
        match self {
            Self::ExistingHost { address, .. } => address.clone(),
            Self::SshServer { connection, .. } => {
                if connection.name.trim().is_empty() || connection.name == connection.host {
                    format!(
                        "{}@{}:{}",
                        connection.user, connection.host, connection.port
                    )
                } else {
                    format!(
                        "{} ({}@{}:{})",
                        connection.name, connection.user, connection.host, connection.port
                    )
                }
            }
        }
    }
    fn zeroize_secrets(&mut self) {
        match self {
            Self::ExistingHost {
                connection_info, ..
            } => connection_info.work_secret.zeroize(),
            Self::SshServer {
                password,
                passphrase,
                ..
            } => {
                password.zeroize();
                passphrase.zeroize();
            }
        }
    }
}

impl RemoteLaunch {
    fn zeroize_secrets(&mut self) {
        self.target.zeroize_secrets();
    }
}

impl Drop for RemoteLaunch {
    fn drop(&mut self) {
        self.zeroize_secrets();
    }
}

/// Starts an isolated remote Client and transfers its launch configuration through stdin.
pub fn spawn_remote_client(mut launch: RemoteLaunch) -> io::Result<()> {
    let payload = serde_json::to_vec(&launch)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    launch.zeroize_secrets();

    let payload = Zeroizing::new(payload);
    if payload.len() > MAX_REMOTE_LAUNCH_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "remote Client launch exceeds the 128 KiB limit",
        ));
    }

    let executable = std::env::current_exe()?;
    let mut command = Command::new(executable);
    command
        .arg("--remote-client")
        .stdin(Stdio::piped())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    detach_child(&mut command);

    let mut child = command.spawn()?;
    let Some(mut stdin) = child.stdin.take() else {
        return Err(io::Error::other("remote Client stdin is unavailable"));
    };
    stdin.write_all(&payload)?;
    stdin.flush()?;
    Ok(())
}

/// Reads the private launch configuration supplied by `spawn_remote_client`.
pub fn read_remote_launch() -> io::Result<RemoteLaunch> {
    let mut payload = Zeroizing::new(Vec::with_capacity(8 * 1024));
    io::stdin()
        .lock()
        .take((MAX_REMOTE_LAUNCH_BYTES + 1) as u64)
        .read_to_end(&mut payload)?;
    if payload.len() > MAX_REMOTE_LAUNCH_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "remote Client launch exceeds the 128 KiB limit",
        ));
    }

    serde_json::from_slice(&payload)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(unix)]
fn detach_child(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;

    command.process_group(0);
}

#[cfg(windows)]
fn detach_child(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;

    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
}
