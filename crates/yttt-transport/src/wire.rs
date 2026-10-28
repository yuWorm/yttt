use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use yttt_protocol::{
    ControlMessage, DecodedFrame, DesktopShellMessage, FrameKind, HEADER_LEN, HandshakeMessage,
    HostEvent, HostResponse, LifecycleMessage, MAX_DESKTOP_SHELL_FRAME_BYTES, MAX_FRAME_BYTES,
    ProtocolCodecError, TerminalInteractiveMessage, Unrecognized, decode_control_message,
    decode_frame, decode_header, decode_host_event, decode_message,
    decode_terminal_interactive_message, encode_message,
};

const WRITE_TIMEOUT: Duration = Duration::from_secs(30);

/// A timed-out write may have sent a partial frame; callers must discard the stream.
pub async fn send_frame(
    stream: &mut (impl AsyncWrite + Unpin),
    frame: &[u8],
) -> Result<(), WireError> {
    let timed_out = || {
        ProtocolCodecError::Io(std::io::Error::new(
            std::io::ErrorKind::TimedOut,
            "protocol frame write made no progress for 30 seconds",
        ))
    };
    let mut remaining = frame;
    while !remaining.is_empty() {
        let written = tokio::time::timeout(WRITE_TIMEOUT, stream.write(remaining))
            .await
            .map_err(|_| timed_out())?
            .map_err(ProtocolCodecError::Io)?;
        if written == 0 {
            return Err(ProtocolCodecError::Io(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "protocol stream stopped accepting bytes",
            ))
            .into());
        }
        remaining = &remaining[written..];
    }
    tokio::time::timeout(WRITE_TIMEOUT, stream.flush())
        .await
        .map_err(|_| timed_out())?
        .map_err(ProtocolCodecError::Io)?;
    Ok(())
}

#[derive(Debug, thiserror::Error)]
pub enum WireError {
    #[error(transparent)]
    Codec(#[from] ProtocolCodecError),
    #[error("unexpected frame kind {received:?}; expected {expected:?}")]
    UnexpectedFrameKind {
        expected: FrameKind,
        received: FrameKind,
    },
    #[error("encoded frame is {encoded_bytes} bytes; limit is {max_bytes} bytes")]
    FrameTooLarge {
        encoded_bytes: usize,
        max_bytes: usize,
    },
}

impl WireError {
    /// A request the peer sent but this build cannot interpret. The Host answers it with
    /// `HostResponse::unsupported` and keeps serving the connection.
    pub fn unsupported_request_id(&self) -> Option<u64> {
        match self {
            Self::Codec(ProtocolCodecError::Unrecognized {
                message: Unrecognized::Request { request_id },
                ..
            }) => Some(*request_id),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WireReceiveDiagnostics {
    pub payload_bytes: usize,
    pub payload_read_and_check: Duration,
    pub message_decode: Duration,
}

async fn send<T: serde::Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    kind: FrameKind,
    message: &T,
) -> Result<(), WireError> {
    let frame = encode_message(kind, message)?;
    send_frame(stream, &frame).await
}

async fn send_bounded<T: serde::Serialize>(
    stream: &mut (impl AsyncWrite + Unpin),
    kind: FrameKind,
    message: &T,
    max_bytes: usize,
) -> Result<(), WireError> {
    let frame = encode_message(kind, message)?;
    if frame.len() > max_bytes {
        return Err(WireError::FrameTooLarge {
            encoded_bytes: frame.len(),
            max_bytes,
        });
    }
    send_frame(stream, &frame).await
}

async fn receive(
    stream: &mut (impl AsyncRead + Unpin),
    expected_kind: FrameKind,
    max_encoded_bytes: usize,
) -> Result<(DecodedFrame, Duration), WireError> {
    let mut header_bytes = [0_u8; HEADER_LEN];
    stream
        .read_exact(&mut header_bytes)
        .await
        .map_err(ProtocolCodecError::Io)?;
    let header = decode_header(&header_bytes)?;
    if header.kind != expected_kind {
        return Err(WireError::UnexpectedFrameKind {
            expected: expected_kind,
            received: header.kind,
        });
    }
    let encoded_bytes = HEADER_LEN + header.payload_len as usize;
    if encoded_bytes > max_encoded_bytes {
        return Err(WireError::FrameTooLarge {
            encoded_bytes,
            max_bytes: max_encoded_bytes,
        });
    }
    let started_at = Instant::now();
    let mut bytes = Vec::with_capacity(encoded_bytes);
    bytes.extend_from_slice(&header_bytes);
    bytes.resize(encoded_bytes, 0);
    stream
        .read_exact(&mut bytes[HEADER_LEN..])
        .await
        .map_err(ProtocolCodecError::Io)?;
    let frame = decode_frame(&bytes)?;
    Ok((frame, started_at.elapsed()))
}

pub async fn send_handshake(
    stream: &mut (impl AsyncWrite + Unpin),
    message: &HandshakeMessage,
) -> Result<(), WireError> {
    send(stream, FrameKind::Handshake, message).await
}

pub async fn receive_handshake(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<HandshakeMessage, WireError> {
    let (frame, _) = receive(stream, FrameKind::Handshake, MAX_FRAME_BYTES + HEADER_LEN).await?;
    Ok(decode_message(&frame)?)
}

pub async fn send_lifecycle(
    stream: &mut (impl AsyncWrite + Unpin),
    message: &LifecycleMessage,
) -> Result<(), WireError> {
    send(stream, FrameKind::Lifecycle, message).await
}

pub async fn receive_lifecycle(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<LifecycleMessage, WireError> {
    let (frame, _) = receive(stream, FrameKind::Lifecycle, MAX_FRAME_BYTES + HEADER_LEN).await?;
    Ok(decode_message(&frame)?)
}

pub async fn send_desktop_shell(
    stream: &mut (impl AsyncWrite + Unpin),
    message: &DesktopShellMessage,
) -> Result<(), WireError> {
    send_bounded(
        stream,
        FrameKind::DesktopShell,
        message,
        MAX_DESKTOP_SHELL_FRAME_BYTES,
    )
    .await
}

pub async fn receive_desktop_shell(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<DesktopShellMessage, WireError> {
    let (frame, _) = receive(
        stream,
        FrameKind::DesktopShell,
        MAX_DESKTOP_SHELL_FRAME_BYTES,
    )
    .await?;
    Ok(decode_message(&frame)?)
}

pub async fn send_terminal_interactive(
    stream: &mut (impl AsyncWrite + Unpin),
    message: &TerminalInteractiveMessage,
) -> Result<(), WireError> {
    send(stream, FrameKind::TerminalInteractive, message).await
}

/// Unrecognized responses resolve as `FailureCode::Unsupported`; unrecognized requests
/// surface as `WireError::unsupported_request_id`.
pub async fn receive_terminal_interactive(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<TerminalInteractiveMessage, WireError> {
    let (frame, _) = receive(
        stream,
        FrameKind::TerminalInteractive,
        MAX_FRAME_BYTES + HEADER_LEN,
    )
    .await?;
    match decode_terminal_interactive_message(&frame) {
        Err(ProtocolCodecError::Unrecognized {
            message: Unrecognized::Response { request_id },
            ..
        }) => Ok(TerminalInteractiveMessage::Response(
            HostResponse::unsupported(request_id),
        )),
        result => Ok(result?),
    }
}

pub async fn send_state_event(
    stream: &mut (impl AsyncWrite + Unpin),
    event: &HostEvent,
) -> Result<(), WireError> {
    send(stream, FrameKind::StateEvent, event).await
}

/// Skips events added by a newer Host.
pub async fn receive_state_event(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<HostEvent, WireError> {
    loop {
        let (frame, _) =
            receive(stream, FrameKind::StateEvent, MAX_FRAME_BYTES + HEADER_LEN).await?;
        match decode_host_event(&frame) {
            Err(ProtocolCodecError::Unrecognized {
                message: Unrecognized::Event,
                ..
            }) => continue,
            result => return Ok(result?),
        }
    }
}
pub async fn send_control(
    stream: &mut (impl AsyncWrite + Unpin),
    message: &ControlMessage,
) -> Result<(), WireError> {
    send(stream, FrameKind::Control, message).await
}

pub async fn send_control_bounded(
    stream: &mut (impl AsyncWrite + Unpin),
    message: &ControlMessage,
    max_bytes: usize,
) -> Result<(), WireError> {
    send_bounded(stream, FrameKind::Control, message, max_bytes).await
}

pub async fn receive_control(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<ControlMessage, WireError> {
    receive_control_observed(stream)
        .await
        .map(|(message, _)| message)
}

/// Skips events added by a newer peer, resolves unrecognized responses as
/// `FailureCode::Unsupported`, and surfaces unrecognized requests as
/// `WireError::unsupported_request_id`.
pub async fn receive_control_observed(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<(ControlMessage, WireReceiveDiagnostics), WireError> {
    loop {
        let (frame, payload_read_and_check) =
            receive(stream, FrameKind::Control, MAX_FRAME_BYTES + HEADER_LEN).await?;
        let payload_bytes = frame.payload.len();
        let started_at = Instant::now();
        let message = match decode_control_message(&frame) {
            Err(ProtocolCodecError::Unrecognized {
                message: Unrecognized::Event,
                ..
            }) => continue,
            Err(ProtocolCodecError::Unrecognized {
                message: Unrecognized::Response { request_id },
                ..
            }) => ControlMessage::Response(HostResponse::unsupported(request_id)),
            result => result?,
        };
        return Ok((
            message,
            WireReceiveDiagnostics {
                payload_bytes,
                payload_read_and_check,
                message_decode: started_at.elapsed(),
            },
        ));
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use yttt_protocol::{ClientRequest, FailureCode, Request, ServerEvent};

    use super::*;

    #[derive(Serialize)]
    enum FutureBody {
        Dragon { size: u32 },
    }

    #[derive(Serialize)]
    struct FutureRequest {
        request_id: u64,
        control: Option<()>,
        body: FutureBody,
    }

    #[derive(Serialize)]
    struct FutureResponse {
        request_id: u64,
        result: Result<FutureBody, ()>,
    }

    #[derive(Serialize)]
    struct FutureEvent {
        host_sequence: u64,
        body: FutureBody,
    }

    #[derive(Serialize)]
    enum FutureMessage {
        Request(FutureRequest),
        Response(FutureResponse),
        Event(FutureEvent),
    }

    async fn write<T: Serialize>(
        stream: &mut (impl AsyncWrite + Unpin),
        kind: FrameKind,
        message: &T,
    ) {
        send_frame(stream, &encode_message(kind, message).unwrap())
            .await
            .unwrap();
    }

    fn future_event() -> FutureEvent {
        FutureEvent {
            host_sequence: 3,
            body: FutureBody::Dragon { size: 1 },
        }
    }

    fn assert_unsupported(response: &HostResponse, request_id: u64) {
        assert_eq!(response.request_id, request_id);
        assert!(matches!(
            &response.result,
            Err(failure) if failure.code == FailureCode::Unsupported
        ));
    }

    #[tokio::test]
    async fn control_stream_survives_messages_from_a_newer_peer() {
        let (mut peer, mut local) = tokio::io::duplex(64 * 1024);
        let event = FutureMessage::Event(future_event());
        write(&mut peer, FrameKind::Control, &event).await;
        let response = FutureMessage::Response(FutureResponse {
            request_id: 9,
            result: Ok(FutureBody::Dragon { size: 2 }),
        });
        write(&mut peer, FrameKind::Control, &response).await;
        let request = FutureMessage::Request(FutureRequest {
            request_id: 7,
            control: None,
            body: FutureBody::Dragon { size: 3 },
        });
        write(&mut peer, FrameKind::Control, &request).await;
        let ping = ControlMessage::Request(ClientRequest::new(8, Request::Ping { sent_millis: 1 }));
        write(&mut peer, FrameKind::Control, &ping).await;

        let ControlMessage::Response(response) = receive_control(&mut local).await.unwrap() else {
            panic!("the unrecognized response must resolve its request");
        };
        assert_unsupported(&response, 9);
        let error = receive_control(&mut local).await.unwrap_err();
        assert_eq!(error.unsupported_request_id(), Some(7));
        assert_eq!(receive_control(&mut local).await.unwrap(), ping);
    }

    #[tokio::test]
    async fn state_events_from_a_newer_host_are_skipped() {
        let (mut peer, mut local) = tokio::io::duplex(64 * 1024);
        write(&mut peer, FrameKind::StateEvent, &future_event()).await;
        let known = HostEvent {
            host_sequence: 4,
            body: ServerEvent::ResourceCatalogChanged,
        };
        write(&mut peer, FrameKind::StateEvent, &known).await;
        assert_eq!(receive_state_event(&mut local).await.unwrap(), known);
    }

    #[tokio::test]
    async fn interactive_responses_from_a_newer_host_resolve_as_unsupported() {
        let (mut peer, mut local) = tokio::io::duplex(64 * 1024);
        write(
            &mut peer,
            FrameKind::TerminalInteractive,
            &FutureMessage::Response(FutureResponse {
                request_id: 12,
                result: Ok(FutureBody::Dragon { size: 4 }),
            }),
        )
        .await;
        let TerminalInteractiveMessage::Response(response) =
            receive_terminal_interactive(&mut local).await.unwrap()
        else {
            panic!("the unrecognized response must resolve its request");
        };
        assert_unsupported(&response, 12);
    }
}
