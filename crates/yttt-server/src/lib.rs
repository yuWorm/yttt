#![forbid(unsafe_code)]

use std::{
    ffi::{OsStr, OsString},
    fs::{self, File, OpenOptions},
    io::{self, Read as _, Write as _},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::Duration,
};

use fs2::FileExt as _;
use rand::{RngCore as _, rngs::OsRng};
use serde::Serialize;
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, HostBlocker, HostLifecycleStatus, LIFECYCLE_PROTOCOL_VERSION,
    LifecycleMessage, LifecycleRequest, LifecycleRequestEnvelope, LifecycleResponse,
    LifecycleResponseEnvelope, ProtocolRange, RESOURCE_PROTOCOL_VERSION,
};
use yttt_transport_local::{
    AuthToken, ClientIdentity, LocalEndpoint, LocalListener, LocalStream, client_handshake,
    connect, receive_lifecycle, send_lifecycle,
};
use zeroize::{Zeroize as _, Zeroizing};

const DEFAULT_PROFILE: &str = "default";
const RESOURCE_COMPATIBILITY: &str = "yttt-resource-v1";
const READY_TIMEOUT: Duration = Duration::from_secs(8);
const READY_RETRY_INTERVAL: Duration = Duration::from_millis(20);

#[derive(thiserror::Error, Debug)]
pub enum ServerError {
    #[error(
        "usage: yttt-server <serve|ensure|status|stop-if-idle> [--profile <id>] [--state-root <absolute path>]"
    )]
    Usage,
    #[error("{0}")]
    InvalidArgument(String),
    #[error("the state root must be an absolute path")]
    RelativeStateRoot,
    #[error("the server state directory is not user-only: {}", .0.display())]
    InsecurePath(PathBuf),
    #[error("the server authentication token is invalid")]
    InvalidToken,
    #[error("the server is not running")]
    NotRunning,
    #[error("a live Host is busy or incompatible: {0}")]
    Busy(String),
    #[error("a live Host did not become ready before the startup timeout")]
    StartupTimeout,
    #[error("the Host lifecycle protocol returned an unexpected response")]
    UnexpectedLifecycleResponse,
    #[error("server I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("server JSON encoding failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("server transport failed: {0}")]
    Transport(#[from] yttt_transport_local::TransportError),
    #[error("server handshake failed: {0}")]
    Handshake(#[from] yttt_transport_local::HandshakeError),
    #[error("server wire protocol failed: {0}")]
    Wire(#[from] yttt_transport_local::WireError),
    #[error("Host failed: {0}")]
    Host(#[from] yttt_host::HostError),
}

#[derive(Serialize)]
pub struct ServerDescriptor {
    pub profile_id: String,
    pub runtime_root: String,
    pub endpoint: String,
    pub auth_token_hex: String,
    pub resource_protocol: u16,
    pub lifecycle_protocol: u16,
    pub state_root: String,
}

#[derive(Serialize)]
#[serde(tag = "result", rename_all = "snake_case")]
enum StopIfIdleOutput {
    Stopping,
    Busy { blockers: Vec<HostBlocker> },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommandKind {
    Serve,
    Ensure,
    Status,
    StopIfIdle,
}

struct Cli {
    command: CommandKind,
    profile: String,
    state_root: PathBuf,
}

struct ServerPaths {
    profile: ProfileId,
    state_root: PathBuf,
    runtime_root: PathBuf,
    token_file: PathBuf,
    log_file: PathBuf,
}

struct Token {
    bytes: Zeroizing<[u8; 32]>,
    auth: AuthToken,
}

impl Token {
    fn from_bytes(mut bytes: [u8; 32]) -> Self {
        let auth = AuthToken::from_bytes(bytes);
        let stored = Zeroizing::new(bytes);
        bytes.zeroize();
        Self {
            bytes: stored,
            auth,
        }
    }
}

struct EnsureLock(File);

impl EnsureLock {
    fn acquire(path: &Path) -> Result<Self, ServerError> {
        reject_symlink(path)?;
        let mut options = OpenOptions::new();
        options.create(true).read(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let file = options.open(path)?;
        secure_file(path)?;
        file.lock_exclusive()?;
        Ok(Self(file))
    }
}

impl Drop for EnsureLock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}

struct LifecycleClient {
    stream: LocalStream,
    next_request_id: u64,
}

impl LifecycleClient {
    async fn request(&mut self, body: LifecycleRequest) -> Result<LifecycleResponse, ServerError> {
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
            _ => Err(ServerError::UnexpectedLifecycleResponse),
        }
    }

    async fn probe_and_status(&mut self) -> Result<HostLifecycleStatus, ServerError> {
        match self.request(LifecycleRequest::Probe).await? {
            LifecycleResponse::Pong => {}
            _ => return Err(ServerError::UnexpectedLifecycleResponse),
        }
        match self.request(LifecycleRequest::Status).await? {
            LifecycleResponse::Status(status) => Ok(status),
            _ => Err(ServerError::UnexpectedLifecycleResponse),
        }
    }
}

enum AttachAttempt {
    Offline,
    ActiveButUnavailable(ServerError),
    Ready(HostLifecycleStatus),
}

pub async fn run_from_os_args(args: impl IntoIterator<Item = OsString>) -> Result<(), ServerError> {
    let cli = parse_cli(args)?;
    let paths = ServerPaths::new(cli.profile, cli.state_root)?;
    match cli.command {
        CommandKind::Serve => serve(paths).await,
        CommandKind::Ensure => ensure(paths).await,
        CommandKind::Status => status(paths).await,
        CommandKind::StopIfIdle => stop_if_idle(paths).await,
    }
}

async fn serve(paths: ServerPaths) -> Result<(), ServerError> {
    paths.prepare_for_host()?;
    let _token = ensure_auth_token(&paths.token_file)?;
    let work_token = ensure_auth_token(&paths.runtime_root.join("work-auth-token"))?;
    let bootstrap = paths.bootstrap();
    let endpoint =
        LocalEndpoint::for_profile(bootstrap.profile_id.clone(), bootstrap.runtime_root.clone());
    let work_endpoint = paths.work_endpoint();
    yttt_host::run_with_remote_work(bootstrap, || async move {
        let admin = LocalListener::bind(endpoint).await?;
        let work = LocalListener::bind(work_endpoint).await?;
        Ok::<_, yttt_transport_local::TransportError>((
            admin,
            Some(yttt_host::RemoteWorkListener::new(work, work_token.auth)),
        ))
    })
    .await?;
    Ok(())
}

async fn ensure(paths: ServerPaths) -> Result<(), ServerError> {
    paths.prepare_for_host()?;
    let _ensure_lock = EnsureLock::acquire(&paths.state_root.join("ensure.lock"))?;
    let token = ensure_auth_token(&paths.token_file)?;

    match attach_attempt(&paths, &token).await {
        AttachAttempt::Ready(status) => {
            if ensure_protocol_compatible(&status).is_ok() {
                write_json(&paths.descriptor(&status)?)?;
                return Ok(());
            }
            let mut lifecycle = connect_lifecycle(&paths, &token).await?;
            match lifecycle.request(LifecycleRequest::StopIfIdle).await? {
                LifecycleResponse::Stopping => {}
                LifecycleResponse::Busy { blockers } => {
                    return Err(ServerError::Busy(format!(
                        "incompatible Host is still in use: {blockers:?}"
                    )));
                }
                _ => return Err(ServerError::UnexpectedLifecycleResponse),
            }
            drop(lifecycle);
            let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
            while yttt_host::profile_lock_is_held(&paths.runtime_root)? {
                if tokio::time::Instant::now() >= deadline {
                    return Err(ServerError::StartupTimeout);
                }
                tokio::time::sleep(READY_RETRY_INTERVAL).await;
            }
        }
        AttachAttempt::ActiveButUnavailable(error) => return Err(active_host_error(error)),
        AttachAttempt::Offline => {}
    }

    if yttt_host::profile_lock_is_held(&paths.runtime_root)? {
        return wait_for_ready(&paths, &token).await;
    }

    spawn_server(&paths)?;
    wait_for_spawned_server(&paths, &token).await
}

async fn status(paths: ServerPaths) -> Result<(), ServerError> {
    paths.prepare_for_client()?;
    let token = read_token(&paths.token_file)?;
    match attach_attempt(&paths, &token).await {
        AttachAttempt::Ready(status) => write_json(&status),
        AttachAttempt::Offline => Err(ServerError::NotRunning),
        AttachAttempt::ActiveButUnavailable(error) => Err(active_host_error(error)),
    }
}

async fn stop_if_idle(paths: ServerPaths) -> Result<(), ServerError> {
    paths.prepare_for_client()?;
    let token = read_token(&paths.token_file)?;
    let mut lifecycle = connect_lifecycle(&paths, &token).await?;
    let result = match lifecycle.request(LifecycleRequest::StopIfIdle).await? {
        LifecycleResponse::Stopping => StopIfIdleOutput::Stopping,
        LifecycleResponse::Busy { blockers } => StopIfIdleOutput::Busy { blockers },
        _ => return Err(ServerError::UnexpectedLifecycleResponse),
    };
    write_json(&result)
}

async fn wait_for_ready(paths: &ServerPaths, token: &Token) -> Result<(), ServerError> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        match attach_attempt(paths, token).await {
            AttachAttempt::Ready(status) => {
                ensure_protocol_compatible(&status)?;
                return write_json(&paths.descriptor(&status)?);
            }
            AttachAttempt::ActiveButUnavailable(error) => return Err(active_host_error(error)),
            AttachAttempt::Offline => {}
        }
        if !yttt_host::profile_lock_is_held(&paths.runtime_root)? {
            return Err(ServerError::NotRunning);
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(ServerError::StartupTimeout);
        }
        tokio::time::sleep(READY_RETRY_INTERVAL).await;
    }
}

async fn wait_for_spawned_server(paths: &ServerPaths, token: &Token) -> Result<(), ServerError> {
    let deadline = tokio::time::Instant::now() + READY_TIMEOUT;
    loop {
        match attach_attempt(paths, token).await {
            AttachAttempt::Ready(status) => {
                ensure_protocol_compatible(&status)?;
                return write_json(&paths.descriptor(&status)?);
            }
            AttachAttempt::ActiveButUnavailable(error) => return Err(active_host_error(error)),
            AttachAttempt::Offline => {}
        }
        if tokio::time::Instant::now() >= deadline {
            return Err(ServerError::StartupTimeout);
        }
        tokio::time::sleep(READY_RETRY_INTERVAL).await;
    }
}

fn ensure_protocol_compatible(status: &HostLifecycleStatus) -> Result<(), ServerError> {
    if status.resource_protocol != RESOURCE_PROTOCOL_VERSION
        || status.lifecycle_protocol != LIFECYCLE_PROTOCOL_VERSION
    {
        return Err(ServerError::Busy(
            "the existing Host uses an incompatible resource or lifecycle protocol".to_string(),
        ));
    }
    Ok(())
}

fn active_host_error(error: ServerError) -> ServerError {
    match error {
        ServerError::Handshake(yttt_transport_local::HandshakeError::Rejected(
            yttt_protocol::RejectReason::VersionMismatch { .. }
            | yttt_protocol::RejectReason::BuildMismatch,
        )) => ServerError::Busy("the existing Host uses an incompatible protocol".to_string()),
        other => ServerError::Busy(other.to_string()),
    }
}

async fn attach_attempt(paths: &ServerPaths, token: &Token) -> AttachAttempt {
    let mut lifecycle = match connect_lifecycle(paths, token).await {
        Ok(lifecycle) => lifecycle,
        Err(ServerError::Transport(yttt_transport_local::TransportError::Io(error)))
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
            ) =>
        {
            return AttachAttempt::Offline;
        }
        Err(error) => return AttachAttempt::ActiveButUnavailable(error),
    };
    match lifecycle.probe_and_status().await {
        Ok(status) => AttachAttempt::Ready(status),
        Err(error) => AttachAttempt::ActiveButUnavailable(error),
    }
}

async fn connect_lifecycle(
    paths: &ServerPaths,
    token: &Token,
) -> Result<LifecycleClient, ServerError> {
    let mut stream = connect(&paths.endpoint()).await?;
    client_handshake(
        &mut stream,
        &ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(LIFECYCLE_PROTOCOL_VERSION),
            build: build_identity(),
            profile_id: paths.profile.clone(),
            client_instance_id: ClientInstanceId::new(format!(
                "yttt-server-{}",
                std::process::id()
            )),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: ConnectionChannel::Lifecycle,
            terminal_session_id: None,
        },
        &token.auth,
    )
    .await?;
    Ok(LifecycleClient {
        stream,
        next_request_id: 1,
    })
}

fn spawn_server(paths: &ServerPaths) -> Result<(), ServerError> {
    let executable = std::env::current_exe()?;
    let log = open_private_log(&paths.log_file)?;
    let error_log = log.try_clone()?;
    let mut command = detached_server_command(executable);
    command
        .arg("serve")
        .arg("--profile")
        .arg(paths.profile.as_str())
        .arg("--state-root")
        .arg(&paths.state_root)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(error_log));
    command.spawn()?;
    Ok(())
}

#[cfg(unix)]
fn detached_server_command(executable: PathBuf) -> Command {
    for setsid in ["/usr/bin/setsid", "/bin/setsid"] {
        if Path::new(setsid).is_file() {
            let mut command = Command::new(setsid);
            command.arg(executable);
            return command;
        }
    }

    use std::os::unix::process::CommandExt as _;
    let mut command = Command::new(executable);
    command.process_group(0);
    command
}

#[cfg(windows)]
fn detached_server_command(executable: PathBuf) -> Command {
    use std::os::windows::process::CommandExt as _;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    let mut command = Command::new(executable);
    command.creation_flags(CREATE_NEW_PROCESS_GROUP);
    command
}

impl ServerPaths {
    fn new(profile: String, state_root: PathBuf) -> Result<Self, ServerError> {
        validate_profile_id(&profile)?;
        if !state_root.is_absolute() {
            return Err(ServerError::RelativeStateRoot);
        }
        let profile = ProfileId::new(profile);
        let runtime_root = state_root.join("runtime");
        let token_file = runtime_root.join("host-auth-token");
        let logs_root = state_root.join("logs");
        let log_file = logs_root.join("server.log");
        Ok(Self {
            profile,
            state_root,
            runtime_root,
            token_file,
            log_file,
        })
    }

    fn prepare_for_host(&self) -> Result<(), ServerError> {
        ensure_directory(&self.state_root)?;
        ensure_child_directory(&self.state_root, "runtime")?;
        ensure_child_directory(&self.state_root, "logs")?;
        Ok(())
    }

    fn prepare_for_client(&self) -> Result<(), ServerError> {
        secure_existing_directory(&self.state_root)?;
        secure_existing_directory(&self.runtime_root)?;
        Ok(())
    }

    fn endpoint(&self) -> LocalEndpoint {
        LocalEndpoint::for_profile(self.profile.clone(), self.runtime_root.clone())
    }

    fn work_endpoint(&self) -> LocalEndpoint {
        LocalEndpoint::for_remote_work(self.profile.clone(), self.runtime_root.clone())
    }

    fn bootstrap(&self) -> yttt_host::HostBootstrap {
        yttt_host::HostBootstrap {
            profile_id: self.profile.clone(),
            runtime_root: self.runtime_root.clone(),
            state_root: self.state_root.clone(),
            config_root: self.state_root.join("config"),
            auth_token_file: self.token_file.clone(),
            ssh_host_keys_file: self.state_root.join("ssh-host-keys.toml"),
            credential_namespace: format!("dev.yttt.remote.{}", self.profile.as_str()),
            build: build_identity(),
            lifetime: yttt_host::HostLifetime::Independent,
        }
    }

    fn descriptor(&self, status: &HostLifecycleStatus) -> Result<ServerDescriptor, ServerError> {
        let token = read_token(&self.runtime_root.join("work-auth-token"))?;
        Ok(ServerDescriptor {
            profile_id: self.profile.as_str().to_string(),
            runtime_root: self.runtime_root.to_string_lossy().into_owned(),
            endpoint: endpoint_string(&self.work_endpoint()),
            auth_token_hex: encode_hex(token.bytes.as_ref()),
            resource_protocol: status.resource_protocol,
            lifecycle_protocol: status.lifecycle_protocol,
            state_root: self.state_root.to_string_lossy().into_owned(),
        })
    }
}

fn parse_cli(args: impl IntoIterator<Item = OsString>) -> Result<Cli, ServerError> {
    let mut args = args.into_iter();
    let _program = args.next();
    let command = match args.next().as_deref() {
        Some(value) if value == OsStr::new("serve") => CommandKind::Serve,
        Some(value) if value == OsStr::new("ensure") => CommandKind::Ensure,
        Some(value) if value == OsStr::new("status") => CommandKind::Status,
        Some(value) if value == OsStr::new("stop-if-idle") => CommandKind::StopIfIdle,
        _ => return Err(ServerError::Usage),
    };
    let mut profile = None;
    let mut state_root = None;
    while let Some(argument) = args.next() {
        if argument == OsStr::new("--profile") {
            if profile.is_some() {
                return Err(ServerError::InvalidArgument(
                    "--profile may only be specified once".to_string(),
                ));
            }
            profile = Some(os_string_value(args.next(), "--profile")?);
        } else if argument == OsStr::new("--state-root") {
            if state_root.is_some() {
                return Err(ServerError::InvalidArgument(
                    "--state-root may only be specified once".to_string(),
                ));
            }
            state_root = Some(PathBuf::from(os_string_value(args.next(), "--state-root")?));
        } else {
            return Err(ServerError::InvalidArgument(format!(
                "unknown argument {}",
                argument.to_string_lossy()
            )));
        }
    }
    let profile = profile.unwrap_or_else(|| DEFAULT_PROFILE.to_string());
    validate_profile_id(&profile)?;
    let state_root = match state_root {
        Some(path) => path,
        None => default_state_root(&profile)?,
    };
    if !state_root.is_absolute() {
        return Err(ServerError::RelativeStateRoot);
    }
    Ok(Cli {
        command,
        profile,
        state_root,
    })
}

fn os_string_value(value: Option<OsString>, option: &str) -> Result<String, ServerError> {
    value
        .ok_or_else(|| ServerError::InvalidArgument(format!("missing value for {option}")))?
        .into_string()
        .map_err(|_| ServerError::InvalidArgument(format!("{option} must be valid UTF-8")))
}

fn default_state_root(profile: &str) -> Result<PathBuf, ServerError> {
    let base = match std::env::var_os("XDG_STATE_HOME") {
        Some(path) => PathBuf::from(path),
        None => {
            let home = std::env::var_os("HOME").ok_or_else(|| {
                ServerError::InvalidArgument(
                    "HOME is required when XDG_STATE_HOME is unset".to_string(),
                )
            })?;
            PathBuf::from(home).join(".local/state")
        }
    };
    if !base.is_absolute() {
        return Err(ServerError::RelativeStateRoot);
    }
    Ok(base.join("yttt").join("remote").join(profile))
}

fn validate_profile_id(profile: &str) -> Result<(), ServerError> {
    if profile.is_empty()
        || profile.len() > 64
        || !profile
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        return Err(ServerError::InvalidArgument(
            "profile must contain 1-64 ASCII letters, digits, '-' or '_'".to_string(),
        ));
    }
    Ok(())
}

fn ensure_auth_token(path: &Path) -> Result<Token, ServerError> {
    match create_token_file(path) {
        Ok(bytes) => Ok(Token::from_bytes(bytes)),
        Err(ServerError::Io(error)) if error.kind() == io::ErrorKind::AlreadyExists => {
            read_token(path)
        }
        Err(error) => Err(error),
    }
}

fn create_token_file(path: &Path) -> Result<[u8; 32], ServerError> {
    reject_symlink(path)?;
    if path.exists() {
        return Err(ServerError::Io(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "server authentication token already exists",
        )));
    }

    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    let (temporary, mut file) = create_private_temporary(path)?;
    let result = (|| -> Result<(), io::Error> {
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        fs::hard_link(&temporary, path)
    })();
    let _ = fs::remove_file(&temporary);
    result?;
    secure_file(path)?;
    Ok(bytes)
}

fn create_private_temporary(path: &Path) -> Result<(PathBuf, File), ServerError> {
    for _ in 0..16 {
        let mut nonce = [0_u8; 16];
        OsRng.fill_bytes(&mut nonce);
        let temporary = path.with_file_name(format!(".host-auth-token-{}.tmp", encode_hex(&nonce)));
        let mut options = OpenOptions::new();
        options.create_new(true).write(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&temporary) {
            Ok(file) => return Ok((temporary, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error.into()),
        }
    }
    Err(ServerError::Io(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not create a unique server authentication token temporary file",
    )))
}

fn read_token(path: &Path) -> Result<Token, ServerError> {
    secure_file(path)?;
    let mut file = File::open(path)?;
    let mut bytes = [0_u8; 32];
    file.read_exact(&mut bytes)
        .map_err(|_| ServerError::InvalidToken)?;
    let mut trailing = [0_u8; 1];
    if file
        .read(&mut trailing)
        .map_err(|_| ServerError::InvalidToken)?
        != 0
    {
        return Err(ServerError::InvalidToken);
    }
    Ok(Token::from_bytes(bytes))
}

fn ensure_directory(path: &Path) -> Result<(), ServerError> {
    match fs::symlink_metadata(path) {
        Ok(_) => secure_existing_directory(path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            fs::create_dir_all(path)?;
            secure_existing_directory(path)
        }
        Err(error) => Err(error.into()),
    }
}

fn ensure_child_directory(parent: &Path, name: &str) -> Result<(), ServerError> {
    let path = parent.join(name);
    match fs::symlink_metadata(&path) {
        Ok(_) => secure_existing_directory(&path),
        Err(error) if error.kind() == io::ErrorKind::NotFound => match fs::create_dir(&path) {
            Ok(()) => secure_existing_directory(&path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                secure_existing_directory(&path)
            }
            Err(error) => Err(error.into()),
        },
        Err(error) => Err(error.into()),
    }
}

fn secure_existing_directory(path: &Path) -> Result<(), ServerError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() {
        return Err(ServerError::InsecurePath(path.to_path_buf()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.uid() != rustix::process::geteuid().as_raw() {
            return Err(ServerError::InsecurePath(path.to_path_buf()));
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

fn secure_file(path: &Path) -> Result<(), ServerError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() {
        return Err(ServerError::InsecurePath(path.to_path_buf()));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::{MetadataExt as _, PermissionsExt as _};
        if metadata.uid() != rustix::process::geteuid().as_raw()
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(ServerError::InsecurePath(path.to_path_buf()));
        }
    }
    Ok(())
}

fn reject_symlink(path: &Path) -> Result<(), ServerError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            Err(ServerError::InsecurePath(path.to_path_buf()))
        }
        Ok(_) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn open_private_log(path: &Path) -> Result<File, ServerError> {
    reject_symlink(path)?;
    let mut options = OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let log = options.open(path)?;
    secure_file(path)?;
    Ok(log)
}

fn build_identity() -> BuildIdentity {
    BuildIdentity {
        product_version: env!("CARGO_PKG_VERSION").to_string(),
        build_fingerprint: option_env!("YTTT_BUILD_FINGERPRINT")
            .filter(|value| !value.is_empty())
            .unwrap_or("yttt-server")
            .to_string(),
        resource_compatibility: RESOURCE_COMPATIBILITY.to_string(),
    }
}

fn endpoint_string(endpoint: &LocalEndpoint) -> String {
    #[cfg(unix)]
    {
        endpoint.unix_path().to_string_lossy().into_owned()
    }
    #[cfg(windows)]
    {
        endpoint.pipe_name().to_string()
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut value = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        value.push(HEX[(byte >> 4) as usize] as char);
        value.push(HEX[(byte & 0x0f) as usize] as char);
    }
    value
}

fn write_json(value: &impl Serialize) -> Result<(), ServerError> {
    let stdout = io::stdout();
    let mut stdout = stdout.lock();
    serde_json::to_writer(&mut stdout, value)?;
    stdout.write_all(b"\n")?;
    stdout.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{
        ffi::OsString,
        sync::{Arc, mpsc},
        thread,
        time::Duration,
    };

    use super::*;

    #[test]
    fn profile_identifiers_cannot_escape_the_profile_namespace() {
        for invalid in [
            "",
            ".",
            "..",
            "remote/profile",
            "remote\\profile",
            "space value",
        ] {
            assert!(
                validate_profile_id(invalid).is_err(),
                "{invalid:?} must be rejected"
            );
        }
        for valid in ["remote", "remote_2", "production-eu"] {
            assert!(
                validate_profile_id(valid).is_ok(),
                "{valid:?} must be accepted"
            );
        }
    }

    #[test]
    fn state_root_must_be_absolute() {
        let error = match parse_cli([
            OsString::from("yttt-server"),
            OsString::from("ensure"),
            OsString::from("--state-root"),
            OsString::from("relative/state"),
        ]) {
            Err(error) => error,
            Ok(_) => panic!("relative state roots must be rejected"),
        };
        assert!(matches!(error, ServerError::RelativeStateRoot));
    }

    #[test]
    fn concurrent_token_creation_converges_on_one_private_token() {
        let temporary = tempfile::tempdir().unwrap();
        let runtime = temporary.path().join("runtime");
        fs::create_dir(&runtime).unwrap();
        secure_existing_directory(&runtime).unwrap();
        let path = Arc::new(runtime.join("host-auth-token"));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let path = path.clone();
            workers.push(thread::spawn(move || {
                let token = ensure_auth_token(&path).unwrap();
                *token.bytes
            }));
        }
        let tokens: Vec<[u8; 32]> = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect();
        assert!(tokens.iter().all(|token| token == &tokens[0]));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(path.as_ref()).unwrap().permissions().mode() & 0o077,
                0
            );
        }
    }

    #[test]
    fn ensure_lock_serializes_concurrent_ensure_callers() {
        let temporary = tempfile::tempdir().unwrap();
        let lock_path = temporary.path().join("ensure.lock");
        let first = EnsureLock::acquire(&lock_path).unwrap();
        let (ready_tx, ready_rx) = mpsc::channel();
        let worker_path = lock_path.clone();
        let worker = thread::spawn(move || {
            let _second = EnsureLock::acquire(&worker_path).unwrap();
            ready_tx.send(()).unwrap();
        });

        assert!(ready_rx.recv_timeout(Duration::from_millis(50)).is_err());
        drop(first);
        ready_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("the second ensure caller must proceed after the first releases its lock");
        worker.join().unwrap();
    }
}
