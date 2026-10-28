use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use yttt_protocol::{
    ControlMessage, DecodedFrame, DesktopShellMessage, FrameKind, HEADER_LEN, HandshakeMessage,
    HostResponse, LifecycleMessage, MAX_DESKTOP_SHELL_FRAME_BYTES, MAX_FRAME_BYTES,
    ProtocolCodecError, Unrecognized, decode_frame, decode_header, decode_message, encode_message,
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
    #[error("invalid chunked message: {0}")]
    InvalidChunk(&'static str),
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

pub(crate) async fn receive_frame(
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
    let (frame, _) =
        receive_frame(stream, FrameKind::Handshake, MAX_FRAME_BYTES + HEADER_LEN).await?;
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
    let (frame, _) =
        receive_frame(stream, FrameKind::Lifecycle, MAX_FRAME_BYTES + HEADER_LEN).await?;
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
    let (frame, _) = receive_frame(
        stream,
        FrameKind::DesktopShell,
        MAX_DESKTOP_SHELL_FRAME_BYTES,
    )
    .await?;
    Ok(decode_message(&frame)?)
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

/// Reads one message from a connection that never interleaves interactive
/// traffic with a chunked message, such as a request/response tool. Long-lived
/// readers keep a [`crate::SessionReader`] so chunk state survives between messages.
pub async fn receive_control(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<ControlMessage, WireError> {
    crate::SessionReader::new(stream).receive().await
}

/// Maps a decoded control message to what the receiver acts on: events added by
/// a newer peer are skipped (`None`), unrecognized responses resolve as
/// `FailureCode::Unsupported`, and unrecognized requests surface as
/// `WireError::unsupported_request_id`.
pub(crate) fn resolve_control(
    decoded: Result<ControlMessage, ProtocolCodecError>,
) -> Result<Option<ControlMessage>, WireError> {
    match decoded {
        Err(ProtocolCodecError::Unrecognized {
            message: Unrecognized::Event,
            ..
        }) => Ok(None),
        Err(ProtocolCodecError::Unrecognized {
            message: Unrecognized::Response { request_id },
            ..
        }) => Ok(Some(ControlMessage::Response(HostResponse::unsupported(
            request_id,
        )))),
        result => Ok(Some(result?)),
    }
}

#[cfg(test)]
mod tests {
    use serde::Serialize;
    use yttt_protocol::{ClientRequest, FailureCode, Request};

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
}
