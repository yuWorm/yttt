use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::Duration,
};

use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    ClientRequest, ControlMessage, HostResponse, PROTOCOL_VERSION, ProtocolRange, Request, Response,
};
use yttt_transport_local::{
    AuthToken, ClientIdentity, LocalEndpoint, LocalStream, client_handshake, connect,
    receive_control, send_control,
};

use crate::config::profile::AppProfile;

const HOST_READY_TIMEOUT: Duration = Duration::from_secs(8);
const HOST_STOP_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessRole {
    Desktop,
    Host,
}

pub fn process_role(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> ProcessRole {
    args.into_iter()
        .any(|argument| argument.as_ref() == OsStr::new("--process-role=host"))
        .then_some(ProcessRole::Host)
        .unwrap_or(ProcessRole::Desktop)
}

#[derive(Debug, thiserror::Error)]
pub enum HostLaunchError {
    #[error("missing Host argument {0}")]
    MissingArgument(&'static str),
    #[error("Host argument {0} is not valid UTF-8")]
    InvalidArgument(&'static str),
    #[error("Host process exited before readiness: {0}")]
    EarlyExit(ExitStatus),
    #[error("Host did not become ready before timeout")]
    ReadyTimeout,
    #[error("Host returned an unexpected control message")]
    UnexpectedMessage,
    #[error("Host request failed: {0}")]
    RequestFailed(String),
    #[error("Host I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("Host transport failed: {0}")]
    Transport(#[from] yttt_transport_local::TransportError),
    #[error("Host handshake failed: {0}")]
    Handshake(#[from] yttt_transport_local::HandshakeError),
    #[error("Host wire protocol failed: {0}")]
    Wire(#[from] yttt_transport_local::WireError),
    #[error("Host runtime failed: {0}")]
    Host(#[from] yttt_host::HostError),
}

#[derive(Clone, Debug)]
pub struct HostLauncher {
    profile: AppProfile,
    executable: PathBuf,
    build_id: String,
}

impl HostLauncher {
    pub fn new(profile: AppProfile, executable: impl Into<PathBuf>) -> Self {
        Self {
            profile,
            executable: executable.into(),
            build_id: env!("CARGO_PKG_VERSION").to_string(),
        }
    }

    pub fn for_current_executable(profile: AppProfile) -> Result<Self, HostLaunchError> {
        Ok(Self::new(profile, std::env::current_exe()?))
    }

    pub fn endpoint(&self) -> LocalEndpoint {
        LocalEndpoint::for_profile(
            self.profile.id().clone(),
            self.profile.paths().runtime.clone(),
        )
    }

    pub async fn launch_or_attach(&self) -> Result<ManagedHostProcess, HostLaunchError> {
        let token_file = self.ensure_auth_token_file()?;
        let token = read_token(&token_file)?;
        if self.connect_with_token(&token).await.is_ok() {
            return Ok(ManagedHostProcess {
                launcher: self.clone(),
                token_file,
                child: None,
            });
        }

        fs::create_dir_all(&self.profile.paths().logs)?;
        let log_file = open_host_log(&self.profile.paths().logs.join("host.log"))?;
        let error_log = log_file.try_clone()?;
        let mut command = Command::new(&self.executable);
        command
            .arg("--process-role=host")
            .arg("--profile-id")
            .arg(self.profile.id().as_str())
            .arg("--runtime-root")
            .arg(&self.profile.paths().runtime)
            .arg("--auth-token-file")
            .arg(&token_file)
            .arg("--ssh-host-keys-file")
            .arg(self.profile.paths().config.join("ssh-host-keys.toml"))
            .arg("--credential-namespace")
            .arg(self.profile.credential_namespace())
            .arg("--build-id")
            .arg(&self.build_id)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log_file))
            .stderr(Stdio::from(error_log));
        detach_child(&mut command);
        let child = command.spawn()?;
        let mut process = ManagedHostProcess {
            launcher: self.clone(),
            token_file,
            child: Some(child),
        };
        process.wait_until_ready(&token).await?;
        Ok(process)
    }

    pub async fn connect(&self) -> Result<HostControlClient, HostLaunchError> {
        let token = read_token(&self.auth_token_file())?;
        self.connect_with_token(&token).await
    }

    pub fn client_core_config(
        &self,
        client_instance_id: ClientInstanceId,
    ) -> Result<(LocalEndpoint, ClientIdentity, AuthToken), HostLaunchError> {
        let token_file = self.ensure_auth_token_file()?;
        let token = read_token(&token_file)?;
        Ok((
            self.endpoint(),
            ClientIdentity {
                supported: ProtocolRange::exact(PROTOCOL_VERSION),
                build_id: self.build_id.clone(),
                profile_id: self.profile.id().clone(),
                client_instance_id,
                host_epoch_hint: None,
            },
            token,
        ))
    }

    fn auth_token_file(&self) -> PathBuf {
        self.profile.paths().runtime.join("host-auth-token")
    }

    fn ensure_auth_token_file(&self) -> Result<PathBuf, HostLaunchError> {
        fs::create_dir_all(&self.profile.paths().runtime)?;
        set_user_only_directory(&self.profile.paths().runtime)?;
        let path = self.auth_token_file();
        match create_token_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                let _ = read_token(&path)?;
            }
            Err(error) => return Err(error.into()),
        }
        Ok(path)
    }

    async fn connect_with_token(
        &self,
        token: &AuthToken,
    ) -> Result<HostControlClient, HostLaunchError> {
        let mut stream = connect(&self.endpoint()).await?;
        let authenticated = client_handshake(
            &mut stream,
            &ClientIdentity {
                supported: ProtocolRange::exact(PROTOCOL_VERSION),
                build_id: self.build_id.clone(),
                profile_id: self.profile.id().clone(),
                client_instance_id: ClientInstanceId::new(format!(
                    "desktop-{}",
                    uuid::Uuid::new_v4()
                )),
                host_epoch_hint: None,
            },
            token,
        )
        .await?;
        Ok(HostControlClient {
            stream,
            host_epoch: authenticated.host_epoch,
            next_request_id: 1,
        })
    }
}

pub struct ManagedHostProcess {
    launcher: HostLauncher,
    token_file: PathBuf,
    child: Option<Child>,
}

impl ManagedHostProcess {
    pub fn spawned(&self) -> bool {
        self.child.is_some()
    }

    pub fn child_id(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    pub async fn connect(&self) -> Result<HostControlClient, HostLaunchError> {
        let token = read_token(&self.token_file)?;
        self.launcher.connect_with_token(&token).await
    }

    pub async fn drain_and_stop(mut self) -> Result<(), HostLaunchError> {
        let mut client = self.connect().await?;
        match client.request(Request::DrainAndStop).await? {
            Response::Draining => {}
            _ => return Err(HostLaunchError::UnexpectedMessage),
        }
        let deadline = tokio::time::Instant::now() + HOST_STOP_TIMEOUT;
        if let Some(child) = self.child.as_mut() {
            loop {
                if let Some(status) = child.try_wait()? {
                    if status.success() {
                        return Ok(());
                    }
                    return Err(HostLaunchError::EarlyExit(status));
                }
                if tokio::time::Instant::now() >= deadline {
                    child.kill()?;
                    let _ = child.wait();
                    return Err(HostLaunchError::ReadyTimeout);
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
        while tokio::time::Instant::now() < deadline {
            let disconnected = self.launcher.connect().await.is_err();
            let runtime = &self.launcher.profile.paths().runtime;
            if disconnected
                && !runtime.join("host-ready.json").exists()
                && !runtime.join("host.pid").exists()
            {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Err(HostLaunchError::ReadyTimeout)
    }

    pub fn force_stop(&mut self) -> Result<(), HostLaunchError> {
        if let Some(child) = self.child.as_mut() {
            child.kill()?;
            let _ = child.wait();
        }
        Ok(())
    }

    async fn wait_until_ready(&mut self, token: &AuthToken) -> Result<(), HostLaunchError> {
        let deadline = tokio::time::Instant::now() + HOST_READY_TIMEOUT;
        let mut early_exit = None;
        loop {
            if let Some(child) = self.child.as_mut()
                && let Some(status) = child.try_wait()?
            {
                early_exit = Some(status);
                self.child = None;
            }
            if self.launcher.connect_with_token(token).await.is_ok() {
                return Ok(());
            }
            if tokio::time::Instant::now() >= deadline {
                return Err(early_exit
                    .map(HostLaunchError::EarlyExit)
                    .unwrap_or(HostLaunchError::ReadyTimeout));
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for ManagedHostProcess {
    fn drop(&mut self) {
        if let Some(child) = self.child.as_mut() {
            let _ = child.try_wait();
        }
    }
}

pub struct HostControlClient {
    stream: LocalStream,
    host_epoch: u64,
    next_request_id: u64,
}

impl HostControlClient {
    pub fn host_epoch(&self) -> u64 {
        self.host_epoch
    }

    pub async fn request(&mut self, body: Request) -> Result<Response, HostLaunchError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        send_control(
            &mut self.stream,
            &ControlMessage::Request(ClientRequest { request_id, body }),
        )
        .await?;
        loop {
            match receive_control(&mut self.stream).await? {
                ControlMessage::Response(HostResponse {
                    request_id: response_id,
                    result,
                }) if response_id == request_id => {
                    return result
                        .map_err(|failure| HostLaunchError::RequestFailed(failure.message));
                }
                ControlMessage::Event(_) => continue,
                _ => return Err(HostLaunchError::UnexpectedMessage),
            }
        }
    }
}

pub async fn run_host_process(
    args: impl IntoIterator<Item = OsString>,
) -> Result<(), HostLaunchError> {
    let parsed = ParsedHostArgs::parse(args)?;
    yttt_host::run(yttt_host::HostBootstrap {
        profile_id: parsed.profile_id,
        runtime_root: parsed.runtime_root,
        auth_token_file: parsed.auth_token_file,
        ssh_host_keys_file: parsed.ssh_host_keys_file,
        credential_namespace: parsed.credential_namespace,
        build_id: parsed.build_id,
    })
    .await?;
    Ok(())
}

struct ParsedHostArgs {
    profile_id: ProfileId,
    runtime_root: PathBuf,
    auth_token_file: PathBuf,
    ssh_host_keys_file: PathBuf,
    credential_namespace: String,
    build_id: String,
}

impl ParsedHostArgs {
    fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, HostLaunchError> {
        let arguments: Vec<OsString> = args.into_iter().collect();
        let value = |name: &'static str| -> Result<OsString, HostLaunchError> {
            arguments
                .windows(2)
                .find(|pair| pair[0] == OsStr::new(name))
                .map(|pair| pair[1].clone())
                .ok_or(HostLaunchError::MissingArgument(name))
        };
        let profile_id = value("--profile-id")?
            .into_string()
            .map_err(|_| HostLaunchError::InvalidArgument("--profile-id"))?;
        let build_id = value("--build-id")?
            .into_string()
            .map_err(|_| HostLaunchError::InvalidArgument("--build-id"))?;
        let credential_namespace = value("--credential-namespace")?
            .into_string()
            .map_err(|_| HostLaunchError::InvalidArgument("--credential-namespace"))?;
        Ok(Self {
            profile_id: ProfileId::new(profile_id),
            runtime_root: PathBuf::from(value("--runtime-root")?),
            ssh_host_keys_file: PathBuf::from(value("--ssh-host-keys-file")?),
            auth_token_file: PathBuf::from(value("--auth-token-file")?),
            credential_namespace,
            build_id,
        })
    }
}

fn create_token_file(path: &Path) -> io::Result<()> {
    let first = uuid::Uuid::new_v4();
    let second = uuid::Uuid::new_v4();
    let mut bytes = [0_u8; 32];
    bytes[..16].copy_from_slice(first.as_bytes());
    bytes[16..].copy_from_slice(second.as_bytes());
    let temporary = path.with_file_name(format!(".host-auth-token-{}.tmp", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.create_new(true).write(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary)?;
    file.write_all(&bytes)?;
    file.sync_all()?;
    drop(file);
    let result = fs::hard_link(&temporary, path);
    let _ = fs::remove_file(&temporary);
    result
}

fn read_token(path: &Path) -> Result<AuthToken, HostLaunchError> {
    validate_token_file(path)?;
    let mut file = File::open(path)?;
    let mut bytes = [0_u8; 32];
    file.read_exact(&mut bytes)?;
    let mut trailing = [0_u8; 1];
    if file.read(&mut trailing)? != 0 {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "invalid Host token length").into());
    }
    Ok(AuthToken::from_bytes(bytes))
}

#[cfg(unix)]
fn set_user_only_directory(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
}

#[cfg(windows)]
fn set_user_only_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn validate_token_file(path: &Path) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt as _;
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Host token file must be user-only",
        ));
    }
    Ok(())
}

#[cfg(windows)]
fn validate_token_file(path: &Path) -> io::Result<()> {
    if !fs::metadata(path)?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "Host token path is not a file",
        ));
    }
    Ok(())
}

fn open_host_log(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    options.open(path)
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
