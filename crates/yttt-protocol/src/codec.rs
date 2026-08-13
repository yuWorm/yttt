use std::io::{self, Read, Write};

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::{
    DEFAULT_COMPATIBILITY_WINDOW, HEADER_LEN, MAX_FRAME_BYTES, PROTOCOL_MAGIC, PROTOCOL_VERSION,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum FrameKind {
    Handshake = 1,
    Control = 2,
}

impl TryFrom<u16> for FrameKind {
    type Error = ProtocolCodecError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Handshake),
            2 => Ok(Self::Control),
            other => Err(ProtocolCodecError::UnknownFrameKind(other)),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FrameHeader {
    pub version: u16,
    pub kind: FrameKind,
    pub payload_len: u32,
    pub checksum: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecodedFrame {
    pub header: FrameHeader,
    pub payload: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum ProtocolCodecError {
    #[error("invalid protocol magic")]
    InvalidMagic,
    #[error("unsupported protocol version {received}; expected {expected}")]
    VersionMismatch { received: u16, expected: u16 },
    #[error("unknown frame kind {0}")]
    UnknownFrameKind(u16),
    #[error("frame payload {actual} bytes exceeds {maximum} byte limit")]
    FrameTooLarge { actual: usize, maximum: usize },
    #[error("frame length mismatch: declared {declared} bytes, received {actual}")]
    LengthMismatch { declared: usize, actual: usize },
    #[error("frame checksum mismatch")]
    ChecksumMismatch,
    #[error("message serialization failed: {0}")]
    Encode(postcard::Error),
    #[error("message deserialization failed: {0}")]
    Decode(postcard::Error),
    #[error("frame I/O failed: {0}")]
    Io(#[from] io::Error),
}

pub fn encode_message<T: Serialize>(
    kind: FrameKind,
    message: &T,
) -> Result<Vec<u8>, ProtocolCodecError> {
    let payload = postcard::to_allocvec(message).map_err(ProtocolCodecError::Encode)?;
    encode_frame(kind, &payload)
}

pub fn decode_message<T: DeserializeOwned>(frame: &DecodedFrame) -> Result<T, ProtocolCodecError> {
    postcard::from_bytes(&frame.payload).map_err(ProtocolCodecError::Decode)
}

pub fn encode_frame(kind: FrameKind, payload: &[u8]) -> Result<Vec<u8>, ProtocolCodecError> {
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtocolCodecError::FrameTooLarge {
            actual: payload.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }
    let payload_len =
        u32::try_from(payload.len()).map_err(|_| ProtocolCodecError::FrameTooLarge {
            actual: payload.len(),
            maximum: MAX_FRAME_BYTES,
        })?;
    let checksum = crc32fast::hash(payload);
    let mut frame = Vec::with_capacity(HEADER_LEN + payload.len());
    frame.extend_from_slice(&PROTOCOL_MAGIC);
    frame.extend_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    frame.extend_from_slice(&(kind as u16).to_be_bytes());
    frame.extend_from_slice(&payload_len.to_be_bytes());
    frame.extend_from_slice(&checksum.to_be_bytes());
    frame.extend_from_slice(payload);
    Ok(frame)
}

pub fn decode_header(bytes: &[u8; HEADER_LEN]) -> Result<FrameHeader, ProtocolCodecError> {
    if bytes[..4] != PROTOCOL_MAGIC {
        return Err(ProtocolCodecError::InvalidMagic);
    }
    let version = u16::from_be_bytes([bytes[4], bytes[5]]);
    let minimum = PROTOCOL_VERSION.saturating_sub(DEFAULT_COMPATIBILITY_WINDOW);
    let maximum = PROTOCOL_VERSION.saturating_add(DEFAULT_COMPATIBILITY_WINDOW);
    if !(minimum..=maximum).contains(&version) {
        return Err(ProtocolCodecError::VersionMismatch {
            received: version,
            expected: PROTOCOL_VERSION,
        });
    }
    let kind = FrameKind::try_from(u16::from_be_bytes([bytes[6], bytes[7]]))?;
    let payload_len = u32::from_be_bytes([bytes[8], bytes[9], bytes[10], bytes[11]]);
    if payload_len as usize > MAX_FRAME_BYTES {
        return Err(ProtocolCodecError::FrameTooLarge {
            actual: payload_len as usize,
            maximum: MAX_FRAME_BYTES,
        });
    }
    let checksum = u32::from_be_bytes([bytes[12], bytes[13], bytes[14], bytes[15]]);
    Ok(FrameHeader {
        version,
        kind,
        payload_len,
        checksum,
    })
}

pub fn decode_frame(bytes: &[u8]) -> Result<DecodedFrame, ProtocolCodecError> {
    if bytes.len() < HEADER_LEN {
        return Err(ProtocolCodecError::LengthMismatch {
            declared: HEADER_LEN,
            actual: bytes.len(),
        });
    }
    let mut header_bytes = [0_u8; HEADER_LEN];
    header_bytes.copy_from_slice(&bytes[..HEADER_LEN]);
    let header = decode_header(&header_bytes)?;
    let expected = HEADER_LEN + header.payload_len as usize;
    if bytes.len() != expected {
        return Err(ProtocolCodecError::LengthMismatch {
            declared: expected,
            actual: bytes.len(),
        });
    }
    let payload = &bytes[HEADER_LEN..];
    if crc32fast::hash(payload) != header.checksum {
        return Err(ProtocolCodecError::ChecksumMismatch);
    }
    Ok(DecodedFrame {
        header,
        payload: payload.to_vec(),
    })
}

pub fn read_frame(reader: &mut impl Read) -> Result<DecodedFrame, ProtocolCodecError> {
    let mut header_bytes = [0_u8; HEADER_LEN];
    reader.read_exact(&mut header_bytes)?;
    let header = decode_header(&header_bytes)?;
    let mut payload = vec![0_u8; header.payload_len as usize];
    reader.read_exact(&mut payload)?;
    if crc32fast::hash(&payload) != header.checksum {
        return Err(ProtocolCodecError::ChecksumMismatch);
    }
    Ok(DecodedFrame { header, payload })
}

pub fn write_frame(
    writer: &mut impl Write,
    kind: FrameKind,
    payload: &[u8],
) -> Result<(), ProtocolCodecError> {
    let frame = encode_frame(kind, payload)?;
    writer.write_all(&frame)?;
    writer.flush()?;
    Ok(())
}
