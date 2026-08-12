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

async fn receive(
    stream: &mut (impl AsyncRead + Unpin),
    expected_kind: FrameKind,
) -> Result<DecodedFrame, WireError> {
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
    let mut bytes = Vec::with_capacity(HEADER_LEN + header.payload_len as usize);
    bytes.extend_from_slice(&header_bytes);
    bytes.resize(HEADER_LEN + header.payload_len as usize, 0);
    stream
        .read_exact(&mut bytes[HEADER_LEN..])
        .await
        .map_err(ProtocolCodecError::Io)?;
    Ok(decode_frame(&bytes)?)
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
    let frame = receive(stream, FrameKind::Handshake).await?;
    Ok(decode_message(&frame)?)
}

pub async fn send_control(
    stream: &mut (impl AsyncWrite + Unpin),
    message: &ControlMessage,
) -> Result<(), WireError> {
    send(stream, FrameKind::Control, message).await
}

pub async fn receive_control(
    stream: &mut (impl AsyncRead + Unpin),
) -> Result<ControlMessage, WireError> {
    let frame = receive(stream, FrameKind::Control).await?;
    Ok(decode_message(&frame)?)
}
