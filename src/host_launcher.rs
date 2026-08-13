use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus, Stdio},
    time::Duration,
};

use sha2::{Digest as _, Sha256};
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ClientRequest, ConnectionChannel, ControlMessage, HostBlocker, HostResponse,
    LIFECYCLE_PROTOCOL_VERSION, LifecycleMessage, LifecycleRequest, LifecycleRequestEnvelope,
    LifecycleResponse, LifecycleResponseEnvelope, ProtocolRange, RESOURCE_PROTOCOL_VERSION,
    Request, Response,
};
use yttt_transport_local::{
    AuthToken, AuthenticatedHost, ClientIdentity, LocalEndpoint, LocalStream, client_handshake,
    connect, receive_control, receive_lifecycle, send_control, send_lifecycle,
};

use crate::config::profile::AppProfile;

const HOST_READY_TIMEOUT: Duration = Duration::from_secs(8);
const HOST_STOP_TIMEOUT: Duration = Duration::from_secs(5);
const RESOURCE_COMPATIBILITY: &str = "yttt-resource-v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcessRole {
    Desktop,
    Host,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExistingHostAction {
    Attach,
    Replace,
    Spawn,
}

pub fn process_role(args: impl IntoIterator<Item = impl AsRef<OsStr>>) -> ProcessRole {
    if args
        .into_iter()
        .any(|argument| argument.as_ref() == OsStr::new("--process-role=host"))
    {
        ProcessRole::Host
    } else {
        ProcessRole::Desktop
    }
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
    #[error("an incompatible Host is still busy: {0:?}")]
    HostBusy(Vec<HostBlocker>),
    #[error("a live Host owns the profile lock but is unreachable (pid: {pid:?})")]
    UnreachableLiveHost { pid: Option<u32> },
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
    build: BuildIdentity,
}

impl HostLauncher {
    pub fn new(profile: AppProfile, executable: impl Into<PathBuf>) -> Self {
        let executable = executable.into();
        Self {
            profile,
            build: build_identity(&executable),
            executable,
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

    pub fn build_identity(&self) -> &BuildIdentity {
        &self.build
    }

    pub async fn launch_or_attach(&self) -> Result<ManagedHostProcess, HostLaunchError> {
        let token_file = self.ensure_auth_token_file()?;
        let token = read_token(&token_file)?;
        match self.existing_host_action(&token).await? {
            ExistingHostAction::Attach => {
                return Ok(ManagedHostProcess {
                    launcher: self.clone(),
                    token_file,
                    child: None,
                });
            }
            ExistingHostAction::Replace => {
                let mut lifecycle = self.connect_lifecycle_with_token(&token, false).await?;
                match lifecycle.request(LifecycleRequest::StopIfIdle).await? {
                    LifecycleResponse::Stopping => self.wait_for_existing_host_exit().await?,
                    LifecycleResponse::Busy { blockers } => {
                        return Err(HostLaunchError::HostBusy(blockers));
                    }
                    _ => return Err(HostLaunchError::UnexpectedMessage),
                }
            }
            ExistingHostAction::Spawn => {}
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
            .arg("--product-version")
            .arg(&self.build.product_version)
            .arg("--build-fingerprint")
            .arg(&self.build.build_fingerprint)
            .arg("--resource-compatibility")
            .arg(&self.build.resource_compatibility)
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
                supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
                build: self.build.clone(),
                profile_id: self.profile.id().clone(),
                client_instance_id,
                host_epoch_hint: None,
                can_force_stop: true,
                channel: yttt_protocol::ConnectionChannel::Control,
                terminal_session_id: None,
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
        let (stream, authenticated) = self
            .connect_channel_with_token(
                token,
                ConnectionChannel::Control,
                ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
                true,
            )
            .await?;
        Ok(HostControlClient {
            stream,
            host_epoch: authenticated.host_epoch,
            next_request_id: 1,
        })
    }

    pub async fn connect_lifecycle(
        &self,
        can_force_stop: bool,
    ) -> Result<HostLifecycleClient, HostLaunchError> {
        let token = read_token(&self.auth_token_file())?;
        self.connect_lifecycle_with_token(&token, can_force_stop)
            .await
    }

    async fn connect_lifecycle_with_token(
        &self,
        token: &AuthToken,
        can_force_stop: bool,
    ) -> Result<HostLifecycleClient, HostLaunchError> {
        let (stream, authenticated) = self
            .connect_channel_with_token(
                token,
                ConnectionChannel::Lifecycle,
                ProtocolRange::exact(LIFECYCLE_PROTOCOL_VERSION),
                can_force_stop,
            )
            .await?;
        Ok(HostLifecycleClient {
            stream,
            host_epoch: authenticated.host_epoch,
            next_request_id: 1,
        })
    }

    async fn connect_channel_with_token(
        &self,
        token: &AuthToken,
        channel: ConnectionChannel,
        supported: ProtocolRange,
        can_force_stop: bool,
    ) -> Result<(LocalStream, AuthenticatedHost), HostLaunchError> {
        let mut stream = connect(&self.endpoint()).await?;
        let authenticated = client_handshake(
            &mut stream,
            &ClientIdentity {
                supported,
                build: self.build.clone(),
                profile_id: self.profile.id().clone(),
                client_instance_id: ClientInstanceId::new(format!(
                    "desktop-{}",
                    uuid::Uuid::new_v4()
                )),
                host_epoch_hint: None,
                can_force_stop,
                channel,
                terminal_session_id: None,
            },
            token,
        )
        .await?;
        Ok((stream, authenticated))
    }

    async fn existing_host_action(
        &self,
        token: &AuthToken,
    ) -> Result<ExistingHostAction, HostLaunchError> {
        let deadline = tokio::time::Instant::now() + HOST_READY_TIMEOUT;
        loop {
            match self.connect_with_token(token).await {
                Ok(_) => return Ok(ExistingHostAction::Attach),
                Err(error) if is_resource_incompatibility(&error) => {
                    return Ok(ExistingHostAction::Replace);
                }
                Err(_) => {}
            }

            if !yttt_host::profile_lock_is_held(&self.profile.paths().runtime)? {
                return Ok(ExistingHostAction::Spawn);
            }
            if self
                .profile
                .paths()
                .runtime
                .join("host-ready.json")
                .exists()
                || tokio::time::Instant::now() >= deadline
            {
                return Err(HostLaunchError::UnreachableLiveHost {
                    pid: self.live_host_pid(),
                });
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }

    fn live_host_pid(&self) -> Option<u32> {
        yttt_host::read_ready_metadata(&self.profile.paths().runtime.join("host-ready.json"))
            .ok()
            .map(|ready| ready.pid)
            .or_else(|| {
                fs::read_to_string(self.profile.paths().runtime.join("host.pid"))
                    .ok()?
                    .trim()
                    .parse()
                    .ok()
            })
    }

    async fn wait_for_existing_host_exit(&self) -> Result<(), HostLaunchError> {
        let deadline = tokio::time::Instant::now() + HOST_STOP_TIMEOUT;
        let runtime = &self.profile.paths().runtime;
        while tokio::time::Instant::now() < deadline {
            if !runtime.join("host-ready.json").exists() && !runtime.join("host.pid").exists() {
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        Err(HostLaunchError::ReadyTimeout)
    }
}
fn is_resource_incompatibility(error: &HostLaunchError) -> bool {
    matches!(
        error,
        HostLaunchError::Handshake(yttt_transport_local::HandshakeError::Rejected(
            yttt_protocol::RejectReason::BuildMismatch
                | yttt_protocol::RejectReason::VersionMismatch { .. }
        ))
    )
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
        let token = read_token(&self.token_file)?;
        let mut client = self
            .launcher
            .connect_lifecycle_with_token(&token, true)
            .await?;
        match client.request(LifecycleRequest::ForceStop).await? {
            LifecycleResponse::Draining => {}
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

pub struct HostLifecycleClient {
    stream: LocalStream,
    host_epoch: u64,
    next_request_id: u64,
}

impl HostLifecycleClient {
    pub fn host_epoch(&self) -> u64 {
        self.host_epoch
    }

    pub async fn request(
        &mut self,
        body: LifecycleRequest,
    ) -> Result<LifecycleResponse, HostLaunchError> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.saturating_add(1);
        send_lifecycle(
            &mut self.stream,
            &LifecycleMessage::Request(LifecycleRequestEnvelope { request_id, body }),
        )
        .await?;
        match receive_lifecycle(&mut self.stream).await? {
            LifecycleMessage::Response(LifecycleResponseEnvelope {
                request_id: response_id,
                result,
            }) if response_id == request_id => Ok(result),
            _ => Err(HostLaunchError::UnexpectedMessage),
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
        build: parsed.build,
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
    build: BuildIdentity,
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
        let string_value = |name: &'static str| -> Result<String, HostLaunchError> {
            value(name)?
                .into_string()
                .map_err(|_| HostLaunchError::InvalidArgument(name))
        };
        let profile_id = string_value("--profile-id")?;
        let credential_namespace = string_value("--credential-namespace")?;
        let build = BuildIdentity {
            product_version: string_value("--product-version")?,
            build_fingerprint: string_value("--build-fingerprint")?,
            resource_compatibility: string_value("--resource-compatibility")?,
        };
        Ok(Self {
            profile_id: ProfileId::new(profile_id),
            runtime_root: PathBuf::from(value("--runtime-root")?),
            ssh_host_keys_file: PathBuf::from(value("--ssh-host-keys-file")?),
            auth_token_file: PathBuf::from(value("--auth-token-file")?),
            credential_namespace,
            build,
        })
    }
}

fn build_identity(executable: &Path) -> BuildIdentity {
    let build_fingerprint = option_env!("YTTT_BUILD_FINGERPRINT")
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .unwrap_or_else(|| {
            let mut digest = Sha256::new();
            digest.update(executable.to_string_lossy().as_bytes());
            if let Ok(metadata) = fs::metadata(executable) {
                digest.update(metadata.len().to_le_bytes());
                if let Ok(modified) = metadata.modified()
                    && let Ok(elapsed) = modified.duration_since(std::time::UNIX_EPOCH)
                {
                    digest.update(elapsed.as_nanos().to_le_bytes());
                }
            }
            format!("{:x}", digest.finalize())
        });
    BuildIdentity {
        product_version: env!("CARGO_PKG_VERSION").to_string(),
        build_fingerprint,
        resource_compatibility: RESOURCE_COMPATIBILITY.to_string(),
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
