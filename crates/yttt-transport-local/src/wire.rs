use std::time::{Duration, Instant};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use yttt_protocol::{
    ControlMessage, DecodedFrame, FrameKind, HEADER_LEN, HandshakeMessage, ProtocolCodecError,
    decode_frame, decode_header, decode_message, encode_message,
};

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
    stream
        .write_all(&frame)
        .await
        .map_err(ProtocolCodecError::Io)?;
    stream.flush().await.map_err(ProtocolCodecError::Io)?;
    Ok(())
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
    stream
        .write_all(&frame)
        .await
        .map_err(ProtocolCodecError::Io)?;
    stream.flush().await.map_err(ProtocolCodecError::Io)?;
    Ok(())
}

async fn receive(
    stream: &mut (impl AsyncRead + Unpin),
    expected_kind: FrameKind,
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
    let started_at = Instant::now();
    let mut bytes = Vec::with_capacity(HEADER_LEN + header.payload_len as usize);
    bytes.extend_from_slice(&header_bytes);
    bytes.resize(HEADER_LEN + header.payload_len as usize, 0);
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
    let (frame, _) = receive(stream, FrameKind::Handshake).await?;
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

pub async fn receive_control(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<ControlMessage, WireError> {
    receive_control_observed(stream)
        .await
        .map(|(message, _)| message)
}

pub async fn receive_control_observed(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<(ControlMessage, WireReceiveDiagnostics), WireError> {
    let (frame, payload_read_and_check) = receive(stream, FrameKind::Control).await?;
    let payload_bytes = frame.payload.len();
    let started_at = Instant::now();
    let message = decode_message(&frame)?;
    Ok((
        message,
        WireReceiveDiagnostics {
            payload_bytes,
            payload_read_and_check,
            message_decode: started_at.elapsed(),
        },
    ))
}
