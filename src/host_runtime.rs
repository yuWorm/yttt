use gpui::Global;
use std::{
    sync::{Arc, Mutex},
    time::Duration,
};
use yttt_client_core::{ClientCore, ClientCoreError, ClientEvent, ConnectionState};
use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};
use yttt_protocol::{Request, Response, terminal::SemanticViewport};

use crate::{
    config::profile::AppProfile,
    host_launcher::{HostLaunchError, HostLauncher, ManagedHostProcess},
};

pub struct DesktopHostRuntime {
    client: Arc<ClientCore>,
    runtime: tokio::runtime::Runtime,
    _managed_process: Arc<Mutex<ManagedHostProcess>>,
}

impl DesktopHostRuntime {
    pub fn start(profile: AppProfile) -> Result<Arc<Self>, DesktopHostRuntimeError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("yttt-client-core")
            .build()?;
        let launcher = HostLauncher::for_current_executable(profile)?;
        let managed_process = Arc::new(Mutex::new(runtime.block_on(launcher.launch_or_attach())?));
        let (endpoint, identity, token) = launcher.client_core_config(ClientInstanceId::new(
            format!("desktop-{}", uuid::Uuid::new_v4()),
        ))?;
        let client = Arc::new(runtime.block_on(ClientCore::connect(endpoint, identity, token))?);
        let mut state = client.subscribe_state();
        let recovery_launcher = launcher;
        let recovered_process = managed_process.clone();
        runtime.spawn(async move {
            while state.changed().await.is_ok() {
                if matches!(&*state.borrow(), ConnectionState::Reconnecting { .. })
                    && let Ok(process) = recovery_launcher.launch_or_attach().await
                    && process.spawned()
                {
                    *recovered_process.lock().unwrap() = process;
                }
            }
        });
        Ok(Arc::new(Self {
            client,
            runtime,
            _managed_process: managed_process,
        }))
    }

    pub fn state(&self) -> ConnectionState {
        self.client.state()
    }

    pub fn request(&self, request: Request) -> flume::Receiver<Result<Response, ClientCoreError>> {
        let (sender, receiver) = flume::bounded(1);
        let client = self.client.clone();
        self.runtime.spawn(async move {
            let _ = sender.send_async(client.request(request).await).await;
        });
        receiver
    }

    pub fn request_blocking_typed(&self, request: Request) -> Result<Response, ClientCoreError> {
        let timeout = match &request {
            Request::RemoteFile(_) | Request::RemoteCommand(_) | Request::Project(_) => {
                Duration::from_secs(125)
            }
            _ => Duration::from_secs(15),
        };
        self.request(request)
            .recv_timeout(timeout)
            .map_err(|error| match error {
                flume::RecvTimeoutError::Timeout => ClientCoreError::RequestTimeout,
                flume::RecvTimeoutError::Disconnected => ClientCoreError::SupervisorStopped,
            })?
    }

    pub fn request_blocking(&self, request: Request) -> Result<Response, String> {
        self.request_blocking_typed(request)
            .map_err(|error| error.to_string())
    }

    pub fn events(&self) -> flume::Receiver<ClientEvent> {
        let (sender, receiver) = flume::bounded(256);
        let mut events = self.client.subscribe_events();
        self.runtime.spawn(async move {
            loop {
                match events.recv().await {
                    Ok(event) => {
                        if sender.send_async(event).await.is_err() {
                            break;
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        });
        receiver
    }

    pub fn terminal_snapshot(&self, session_id: &TerminalSessionId) -> Option<SemanticViewport> {
        self.client.terminal_snapshot(session_id)
    }
}

impl Drop for DesktopHostRuntime {
    fn drop(&mut self) {
        self.client.shutdown();
    }
}

impl yttt_ssh::HostTransportProxy for DesktopHostRuntime {
    fn request(&self, request: Request) -> Result<Response, String> {
        self.request_blocking(request)
    }

    fn events(&self) -> flume::Receiver<yttt_protocol::ServerEvent> {
        let source = DesktopHostRuntime::events(self);
        let (sender, receiver) = flume::bounded(256);
        std::thread::Builder::new()
            .name("yttt-host-ssh-events".to_string())
            .spawn(move || {
                while let Ok(event) = source.recv() {
                    if let ClientEvent::Server(event) = event
                        && sender.send(event.body).is_err()
                    {
                        break;
                    }
                }
            })
            .expect("failed to spawn Host SSH event bridge");
        receiver
    }
}

#[derive(Clone)]
pub struct HostRuntimeGlobal {
    runtime: Option<Arc<DesktopHostRuntime>>,
    error: Option<Arc<str>>,
}

impl HostRuntimeGlobal {
    pub fn ready(runtime: Arc<DesktopHostRuntime>) -> Self {
        Self {
            runtime: Some(runtime),
            error: None,
        }
    }

    pub fn unavailable(error: impl Into<Arc<str>>) -> Self {
        Self {
            runtime: None,
            error: Some(error.into()),
        }
    }

    pub fn runtime(&self) -> Option<&Arc<DesktopHostRuntime>> {
        self.runtime.as_ref()
    }

    pub fn error(&self) -> Option<&str> {
        self.error.as_deref()
    }
}

impl Global for HostRuntimeGlobal {}

#[derive(Debug, thiserror::Error)]
pub enum DesktopHostRuntimeError {
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Launch(#[from] HostLaunchError),
    #[error(transparent)]
    Client(#[from] ClientCoreError),
}
