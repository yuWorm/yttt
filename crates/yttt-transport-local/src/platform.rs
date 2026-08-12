use tokio::io::{AsyncRead, AsyncWrite};

use crate::LocalEndpoint;

pub trait AsyncLocalStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T> AsyncLocalStream for T where T: AsyncRead + AsyncWrite + Unpin + Send {}

pub type LocalStream = Box<dyn AsyncLocalStream>;

#[derive(Debug, thiserror::Error)]
pub enum TransportError {
    #[error("host endpoint is already in use")]
    EndpointInUse,
    #[error("host endpoint is owned by another user")]
    WrongOwner,
    #[error("host endpoint permissions are not user-only")]
    InsecurePermissions,
    #[error("peer belongs to a different operating-system user")]
    WrongPeer,
    #[error("invalid endpoint for this platform")]
    InvalidEndpoint,
    #[error("local transport I/O failed: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(unix)]
mod imp {
    use std::{
        fs,
        os::unix::fs::{FileTypeExt as _, MetadataExt as _, PermissionsExt as _},
        path::Path,
    };

    use tokio::net::{UnixListener, UnixStream};

    use super::{LocalEndpoint, LocalStream, TransportError};

    pub struct LocalListener {
        listener: UnixListener,
        endpoint: LocalEndpoint,
        socket_device: u64,
        socket_inode: u64,
    }

    impl LocalListener {
        pub async fn bind(endpoint: LocalEndpoint) -> Result<Self, TransportError> {
            prepare_runtime_root(endpoint.runtime_root())?;
            let path = endpoint.unix_path();
            remove_stale_socket(path).await?;
            let listener = UnixListener::bind(path).map_err(|error| {
                if error.kind() == std::io::ErrorKind::AddrInUse {
                    TransportError::EndpointInUse
                } else {
                    TransportError::Io(error)
                }
            })?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
            let metadata = fs::symlink_metadata(path)?;
            Ok(Self {
                listener,
                endpoint,
                socket_device: metadata.dev(),
                socket_inode: metadata.ino(),
            })
        }

        pub async fn accept(&self) -> Result<LocalStream, TransportError> {
            loop {
                let (stream, _) = self.listener.accept().await?;
                match verify_peer(&stream) {
                    Ok(()) => return Ok(Box::new(stream)),
                    Err(TransportError::Io(error))
                        if error.kind() == std::io::ErrorKind::NotConnected =>
                    {
                        continue;
                    }
                    Err(error) => return Err(error),
                }
            }
        }

        pub fn endpoint(&self) -> &LocalEndpoint {
            &self.endpoint
        }
    }

    impl Drop for LocalListener {
        fn drop(&mut self) {
            let path = self.endpoint.unix_path();
            let Ok(metadata) = fs::symlink_metadata(path) else {
                return;
            };
            if metadata.file_type().is_socket()
                && metadata.dev() == self.socket_device
                && metadata.ino() == self.socket_inode
                && metadata.uid() == current_uid()
            {
                let _ = fs::remove_file(path);
            }
        }
    }

    pub async fn connect(endpoint: &LocalEndpoint) -> Result<LocalStream, TransportError> {
        validate_socket(endpoint.unix_path())?;
        let stream = UnixStream::connect(endpoint.unix_path()).await?;
        verify_peer(&stream)?;
        Ok(Box::new(stream))
    }

    fn prepare_runtime_root(path: &Path) -> Result<(), TransportError> {
        fs::create_dir_all(path)?;
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.is_dir() || metadata.uid() != current_uid() {
            return Err(TransportError::WrongOwner);
        }
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        Ok(())
    }

    async fn remove_stale_socket(path: &Path) -> Result<(), TransportError> {
        let metadata = match fs::symlink_metadata(path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error.into()),
        };
        if !metadata.file_type().is_socket() || metadata.uid() != current_uid() {
            return Err(TransportError::WrongOwner);
        }
        if UnixStream::connect(path).await.is_ok() {
            return Err(TransportError::EndpointInUse);
        }
        fs::remove_file(path)?;
        Ok(())
    }

    fn validate_socket(path: &Path) -> Result<(), TransportError> {
        let metadata = fs::symlink_metadata(path)?;
        if !metadata.file_type().is_socket() || metadata.uid() != current_uid() {
            return Err(TransportError::WrongOwner);
        }
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(TransportError::InsecurePermissions);
        }
        Ok(())
    }

    fn verify_peer(stream: &UnixStream) -> Result<(), TransportError> {
        if stream.peer_cred()?.uid() != current_uid() {
            return Err(TransportError::WrongPeer);
        }
        Ok(())
    }

    fn current_uid() -> u32 {
        rustix::process::geteuid().as_raw()
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::{ffi::c_void, io, mem, ptr, time::Duration};

    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeServer, PipeMode, ServerOptions,
    };
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            SECURITY_ATTRIBUTES,
        },
    };

    use super::{LocalEndpoint, LocalStream, TransportError};

    const SDDL_REVISION_1: u32 = 1;
    const ERROR_PIPE_BUSY: i32 = 231;

    pub struct LocalListener {
        server: tokio::sync::Mutex<NamedPipeServer>,
        endpoint: LocalEndpoint,
    }

    impl LocalListener {
        pub async fn bind(endpoint: LocalEndpoint) -> Result<Self, TransportError> {
            let server = create_secure_server(endpoint.pipe_name(), true).map_err(|error| {
                if error.kind() == io::ErrorKind::PermissionDenied {
                    TransportError::EndpointInUse
                } else {
                    TransportError::Io(error)
                }
            })?;
            Ok(Self {
                server: tokio::sync::Mutex::new(server),
                endpoint,
            })
        }

        pub async fn accept(&self) -> Result<LocalStream, TransportError> {
            let mut server = self.server.lock().await;
            server.connect().await?;
            let replacement = create_secure_server(self.endpoint.pipe_name(), false)?;
            let connected = mem::replace(&mut *server, replacement);
            Ok(Box::new(connected))
        }

        pub fn endpoint(&self) -> &LocalEndpoint {
            &self.endpoint
        }
    }

    pub async fn connect(endpoint: &LocalEndpoint) -> Result<LocalStream, TransportError> {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        loop {
            match ClientOptions::new()
                .pipe_mode(PipeMode::Byte)
                .open(endpoint.pipe_name())
            {
                Ok(client) => return Ok(Box::new(client)),
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY)
                        && tokio::time::Instant::now() < deadline =>
                {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => return Err(error.into()),
            }
        }
    }

    fn create_secure_server(name: &str, first: bool) -> io::Result<NamedPipeServer> {
        let mut descriptor: *mut c_void = ptr::null_mut();
        let sddl: Vec<u16> = "D:P(A;;GA;;;SY)(A;;GA;;;OW)\0".encode_utf16().collect();
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                sddl.as_ptr(),
                SDDL_REVISION_1,
                &mut descriptor,
                ptr::null_mut(),
            )
        };
        if converted == 0 {
            return Err(io::Error::last_os_error());
        }
        let mut attributes = SECURITY_ATTRIBUTES {
            nLength: mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor,
            bInheritHandle: 0,
        };
        let result = unsafe {
            ServerOptions::new()
                .pipe_mode(PipeMode::Byte)
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(
                    name,
                    (&mut attributes as *mut SECURITY_ATTRIBUTES).cast(),
                )
        };
        unsafe {
            LocalFree(descriptor);
        }
        result
    }
}

pub use imp::{LocalListener, connect};
