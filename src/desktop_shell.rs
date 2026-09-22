use std::{path::PathBuf, sync::Arc, time::Duration};

use yttt_core::model::ids::ProfileId;
use yttt_protocol::{
    DESKTOP_SHELL_PROTOCOL_VERSION, DesktopShellMessage, DesktopShellRejectReason,
    DesktopShellRequest, DesktopShellRequestEnvelope, DesktopShellResponse,
    DesktopShellResponseEnvelope, MAX_DESKTOP_OPEN_PATHS,
};
use yttt_transport_local::{
    LocalEndpoint, LocalListener, TransportError, WireError, connect, receive_desktop_shell,
    send_desktop_shell,
};

use crate::{
    config::profile::AppProfile,
    runtime::project::{path_to_platform, platform_path},
};

use yttt_protocol::desktop_control::{
    DesktopControlError, DesktopControlErrorCode, DesktopControlRequest, DesktopControlResult,
};

pub struct DesktopControlCall {
    pub request: DesktopControlRequest,
    pub reply: flume::Sender<DesktopControlResult>,
    pub deadline: std::time::Instant,
}

const DESKTOP_CONTROL_TIMEOUT: Duration = Duration::from_secs(15);
const DESKTOP_COMMAND_QUEUE_CAPACITY: usize = 16;
const DESKTOP_FORWARD_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DesktopShellCommand {
    Activate,
    OpenWindow { project_paths: Vec<PathBuf> },
}

pub enum DesktopShellClaim {
    Owner(Arc<DesktopShellRuntime>),
    Forwarded,
}

pub struct DesktopShellRuntime {
    _runtime: tokio::runtime::Runtime,
    commands: flume::Receiver<DesktopShellCommand>,
    controls: flume::Receiver<DesktopControlCall>,
}

impl DesktopShellRuntime {
    pub fn claim_or_forward(
        profile: &AppProfile,
        command: DesktopShellCommand,
    ) -> Result<DesktopShellClaim, DesktopShellError> {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .thread_name("yttt-desktop-shell")
            .build()?;
        let endpoint =
            LocalEndpoint::for_desktop_shell(profile.id().clone(), profile.paths().runtime.clone());
        match runtime.block_on(LocalListener::bind(endpoint.clone())) {
            Ok(listener) => {
                let (commands_tx, commands) = flume::bounded(DESKTOP_COMMAND_QUEUE_CAPACITY);
                let (controls_tx, controls) = flume::bounded(DESKTOP_COMMAND_QUEUE_CAPACITY);
                let profile_id = profile.id().clone();
                runtime.spawn(serve_desktop_shell(
                    listener,
                    profile_id,
                    commands_tx,
                    controls_tx,
                ));
                Ok(DesktopShellClaim::Owner(Arc::new(Self {
                    _runtime: runtime,
                    commands,
                    controls,
                })))
            }
            Err(TransportError::EndpointInUse) => {
                let body = command_to_request(command)?;
                runtime.block_on(forward_desktop_request(&endpoint, profile.id(), body))?;
                Ok(DesktopShellClaim::Forwarded)
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn controls(&self) -> flume::Receiver<DesktopControlCall> {
        self.controls.clone()
    }

    pub fn commands(&self) -> flume::Receiver<DesktopShellCommand> {
        self.commands.clone()
    }
}

async fn serve_desktop_shell(
    listener: LocalListener,
    profile_id: ProfileId,
    commands: flume::Sender<DesktopShellCommand>,
    controls: flume::Sender<DesktopControlCall>,
) {
    loop {
        let Ok(mut stream) = listener.accept().await else {
            break;
        };
        let profile_id = profile_id.clone();
        let commands = commands.clone();
        let controls = controls.clone();
        tokio::spawn(async move {
            let Ok(DesktopShellMessage::Request(request)) =
                receive_desktop_shell(&mut stream).await
            else {
                return;
            };
            let request_id = request.request_id;
            let result = if request.protocol_version != DESKTOP_SHELL_PROTOCOL_VERSION {
                DesktopShellResponse::Rejected(DesktopShellRejectReason::VersionMismatch {
                    supported: DESKTOP_SHELL_PROTOCOL_VERSION,
                })
            } else if request.profile_id != profile_id {
                DesktopShellResponse::Rejected(DesktopShellRejectReason::ProfileMismatch)
            } else if let DesktopShellRequest::Control(control) = request.body {
                let (reply, response) = flume::bounded(1);
                let call = DesktopControlCall {
                    request: *control,
                    reply,
                    deadline: std::time::Instant::now() + DESKTOP_CONTROL_TIMEOUT,
                };
                let result = if controls.try_send(call).is_err() {
                    Err(DesktopControlError::new(
                        DesktopControlErrorCode::Busy,
                        "Desktop control queue is full",
                    ))
                } else {
                    match tokio::time::timeout(DESKTOP_CONTROL_TIMEOUT, response.recv_async()).await
                    {
                        Ok(Ok(result)) => result,
                        _ => Err(DesktopControlError::new(
                            DesktopControlErrorCode::OutcomeUnknown,
                            "Desktop response timed out; inspect state before retrying a mutation",
                        )),
                    }
                };
                DesktopShellResponse::Control(Box::new(result))
            } else {
                validate_desktop_request(request, &profile_id)
                    .and_then(|command| {
                        commands
                            .try_send(command)
                            .map_err(|_| DesktopShellRejectReason::Busy)
                    })
                    .map_or_else(DesktopShellResponse::Rejected, |_| {
                        DesktopShellResponse::Accepted
                    })
            };
            if let Err(WireError::FrameTooLarge { .. }) = send_desktop_shell(
                &mut stream,
                &DesktopShellMessage::Response(DesktopShellResponseEnvelope { request_id, result }),
            )
            .await
            {
                let result = DesktopShellResponse::Control(Box::new(Err(
                    DesktopControlError::new(
                        DesktopControlErrorCode::InvalidRequest,
                        "Response exceeds 256 KiB; narrow the query with --window, --project, --tab or --pane",
                    ),
                )));
                let _ = send_desktop_shell(
                    &mut stream,
                    &DesktopShellMessage::Response(DesktopShellResponseEnvelope {
                        request_id,
                        result,
                    }),
                )
                .await;
            }
        });
    }
}

fn validate_desktop_request(
    request: DesktopShellRequestEnvelope,
    profile_id: &ProfileId,
) -> Result<DesktopShellCommand, DesktopShellRejectReason> {
    if request.protocol_version != DESKTOP_SHELL_PROTOCOL_VERSION {
        return Err(DesktopShellRejectReason::VersionMismatch {
            supported: DESKTOP_SHELL_PROTOCOL_VERSION,
        });
    }
    if request.profile_id != *profile_id {
        return Err(DesktopShellRejectReason::ProfileMismatch);
    }
    match request.body {
        DesktopShellRequest::Control(_) => Err(DesktopShellRejectReason::InvalidRequest),
        DesktopShellRequest::Activate => Ok(DesktopShellCommand::Activate),
        DesktopShellRequest::OpenWindow { project_paths } => {
            if project_paths.len() > MAX_DESKTOP_OPEN_PATHS {
                return Err(DesktopShellRejectReason::InvalidRequest);
            }
            let project_paths = project_paths
                .into_iter()
                .map(platform_path)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| DesktopShellRejectReason::InvalidRequest)?;
            Ok(DesktopShellCommand::OpenWindow { project_paths })
        }
    }
}
fn command_to_request(
    command: DesktopShellCommand,
) -> Result<DesktopShellRequest, DesktopShellError> {
    match command {
        DesktopShellCommand::Activate => Ok(DesktopShellRequest::Activate),
        DesktopShellCommand::OpenWindow { project_paths } => {
            if project_paths.len() > MAX_DESKTOP_OPEN_PATHS {
                return Err(DesktopShellError::Rejected(
                    DesktopShellRejectReason::InvalidRequest,
                ));
            }
            Ok(DesktopShellRequest::OpenWindow {
                project_paths: project_paths
                    .iter()
                    .map(|path| path_to_platform(path))
                    .collect::<Result<Vec<_>, _>>()
                    .map_err(|_| {
                        DesktopShellError::Rejected(DesktopShellRejectReason::InvalidRequest)
                    })?,
            })
        }
    }
}

async fn forward_desktop_request(
    endpoint: &LocalEndpoint,
    profile_id: &ProfileId,
    body: DesktopShellRequest,
) -> Result<(), DesktopShellError> {
    tokio::time::timeout(DESKTOP_FORWARD_TIMEOUT, async {
        let mut stream = connect(endpoint).await?;
        let request_id = desktop_request_id();
        send_desktop_shell(
            &mut stream,
            &DesktopShellMessage::Request(DesktopShellRequestEnvelope {
                protocol_version: DESKTOP_SHELL_PROTOCOL_VERSION,
                profile_id: profile_id.clone(),
                request_id,
                body,
            }),
        )
        .await?;
        match receive_desktop_shell(&mut stream).await? {
            DesktopShellMessage::Response(DesktopShellResponseEnvelope {
                request_id: response_id,
                result: DesktopShellResponse::Accepted,
            }) if response_id == request_id => Ok(()),
            DesktopShellMessage::Response(DesktopShellResponseEnvelope {
                request_id: response_id,
                result: DesktopShellResponse::Rejected(reason),
            }) if response_id == request_id => Err(DesktopShellError::Rejected(reason)),
            _ => Err(DesktopShellError::UnexpectedMessage),
        }
    })
    .await
    .map_err(|_| DesktopShellError::Timeout)?
}

/// Sends one request to an already running desktop; mutations are never replayed.
pub async fn control_desktop(
    profile: &AppProfile,
    request: DesktopControlRequest,
) -> Result<DesktopControlResult, DesktopShellError> {
    let endpoint =
        LocalEndpoint::for_desktop_shell(profile.id().clone(), profile.paths().runtime.clone());
    tokio::time::timeout(DESKTOP_CONTROL_TIMEOUT + Duration::from_secs(1), async {
        let mut stream = connect(&endpoint).await?;
        let request_id = desktop_request_id();
        send_desktop_shell(
            &mut stream,
            &DesktopShellMessage::Request(DesktopShellRequestEnvelope {
                protocol_version: DESKTOP_SHELL_PROTOCOL_VERSION,
                profile_id: profile.id().clone(),
                request_id,
                body: DesktopShellRequest::Control(Box::new(request)),
            }),
        )
        .await?;
        match receive_desktop_shell(&mut stream).await? {
            DesktopShellMessage::Response(DesktopShellResponseEnvelope {
                request_id: id,
                result,
            }) if id == request_id => match result {
                DesktopShellResponse::Control(result) => Ok(*result),
                DesktopShellResponse::Rejected(reason) => Err(DesktopShellError::Rejected(reason)),
                _ => Err(DesktopShellError::UnexpectedMessage),
            },
            _ => Err(DesktopShellError::UnexpectedMessage),
        }
    })
    .await
    .map_err(|_| DesktopShellError::Timeout)?
}

fn desktop_request_id() -> u64 {
    let bytes = *uuid::Uuid::new_v4().as_bytes();
    let mut prefix = [0_u8; 8];
    prefix.copy_from_slice(&bytes[..8]);
    u64::from_le_bytes(prefix)
}

#[derive(Debug, thiserror::Error)]
pub enum DesktopShellError {
    #[error("desktop shell runtime failed: {0}")]
    Runtime(#[from] std::io::Error),
    #[error("desktop shell transport failed: {0}")]
    Transport(#[from] TransportError),
    #[error("desktop shell wire protocol failed: {0}")]
    Wire(#[from] WireError),
    #[error("desktop shell rejected activation: {0:?}")]
    Rejected(DesktopShellRejectReason),
    #[error("desktop shell returned an unexpected response")]
    UnexpectedMessage,
    #[error("desktop shell activation timed out")]
    Timeout,
}
