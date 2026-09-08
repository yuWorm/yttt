use std::{
    io::{self, Read as _, Write as _},
    process::{Command, Stdio},
};

use serde::{Deserialize, Serialize};
use zeroize::{Zeroize as _, Zeroizing};

use crate::config::{profile::AppProfile, ssh::SshConnectionConfig};

const MAX_REMOTE_LAUNCH_BYTES: usize = 128 * 1024;

/// The authenticated connection information passed privately from the launcher to a remote Client.
///
/// This type deliberately does not implement `Debug`: password material must never reach logs.
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RemoteLaunch {
    pub local_profile: AppProfile,
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
                format!("{}:{}", connection.host, connection.port)
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
        .read_to_end(&mut *payload)?;
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
