use std::io::{self, Read, Write};

use serde::{Serialize, de::DeserializeOwned};
use thiserror::Error;

use crate::{FRAME_FORMAT_VERSION, HEADER_LEN, MAX_FRAME_BYTES, PROTOCOL_MAGIC};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u16)]
pub enum FrameKind {
    Handshake = 1,
    Control = 2,
    Lifecycle = 3,
    DesktopShell = 4,
    TerminalInteractive = 5,
    StateEvent = 6,
}

impl TryFrom<u16> for FrameKind {
    type Error = ProtocolCodecError;

    fn try_from(value: u16) -> Result<Self, Self::Error> {
        match value {
            1 => Ok(Self::Handshake),
            2 => Ok(Self::Control),
            3 => Ok(Self::Lifecycle),
            4 => Ok(Self::DesktopShell),
            5 => Ok(Self::TerminalInteractive),
            6 => Ok(Self::StateEvent),
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
    Encode(String),
    #[error("message deserialization failed: {0}")]
    Decode(String),
    #[error("frame I/O failed: {0}")]
    Io(#[from] io::Error),
}

pub fn encode_message<T: Serialize>(
    kind: FrameKind,
    message: &T,
) -> Result<Vec<u8>, ProtocolCodecError> {
    let mut payload = Vec::new();
    ciborium::into_writer(message, &mut payload)
        .map_err(|error| ProtocolCodecError::Encode(error.to_string()))?;
    encode_frame(kind, &payload)
}

pub fn decode_message<T: DeserializeOwned>(frame: &DecodedFrame) -> Result<T, ProtocolCodecError> {
    ciborium::from_reader(frame.payload.as_slice())
        .map_err(|error| ProtocolCodecError::Decode(error.to_string()))
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
    frame.extend_from_slice(&FRAME_FORMAT_VERSION.to_be_bytes());
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
    if version != FRAME_FORMAT_VERSION {
        return Err(ProtocolCodecError::VersionMismatch {
            received: version,
            expected: FRAME_FORMAT_VERSION,
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

#[cfg(test)]
mod tests {
    use super::*;
    use yttt_core::model::ids::{ClientInstanceId, TerminalSessionId};

    use crate::{
        ControlMessage, FailureCode, HostResponse, ProtocolFailure, Response, TerminalLease,
        terminal::{
            CursorShape, SemanticCursor, SemanticViewport, TerminalCheckpoint, TerminalGeometry,
            TerminalInput, TerminalLeaseMode, TerminalModes, TerminalMutationContext,
            TerminalPalette, TerminalProcessState,
        },
    };

    fn sample_viewport() -> SemanticViewport {
        SemanticViewport {
            session_id: TerminalSessionId::new("session"),
            session_epoch: 1,
            sequence: 4,
            geometry: TerminalGeometry {
                cols: 80,
                rows: 24,
                cell_width: 8,
                cell_height: 16,
            },
            geometry_epoch: 1,
            scrollback_epoch: 1,
            history_size: 0,
            display_offset: 0,
            rows: Vec::new(),
            cursor: SemanticCursor {
                row: 0,
                column: 0,
                shape: CursorShape::Block,
                visible: true,
                blinking: false,
            },
            modes: TerminalModes {
                bits: 0,
                title: None,
                cwd: None,
            },
            palette: TerminalPalette {
                colors: Vec::new(),
                revision: 1,
            },
            process_state: TerminalProcessState::Running,
        }
    }

    fn attached_response(checkpoint: TerminalCheckpoint) -> ControlMessage {
        ControlMessage::Response(HostResponse {
            request_id: 1,
            result: Ok(Response::TerminalAttached {
                lease: TerminalLease {
                    session_id: TerminalSessionId::new("session"),
                    owner: ClientInstanceId::new("client"),
                    mode: TerminalLeaseMode::Interactive,
                    lease_epoch: 1,
                },
                checkpoint,
            }),
        })
    }

    #[test]
    fn one_way_terminal_input_round_trips_as_a_control_message() {
        let message = ControlMessage::TerminalInput(TerminalInput {
            session_id: TerminalSessionId::new("session"),
            context: TerminalMutationContext {
                host_epoch: 1,
                session_epoch: 2,
                lease_epoch: 3,
                client_sequence: 4,
                geometry_epoch: 5,
            },
            bytes: b"typed".to_vec(),
        });

        let encoded = encode_message(FrameKind::Control, &message).unwrap();
        let decoded: ControlMessage = decode_message(&decode_frame(&encoded).unwrap()).unwrap();
        assert_eq!(decoded, message);
    }

    #[test]
    fn control_checkpoint_with_full_raw_replay_tail_exceeds_frame_limit() {
        let encoded = encode_message(
            FrameKind::Control,
            &attached_response(TerminalCheckpoint {
                viewport: sample_viewport(),
                raw_replay_tail: vec![b'x'; MAX_FRAME_BYTES],
                raw_tail_start_sequence: 0,
            }),
        );
        assert!(matches!(
            encoded,
            Err(ProtocolCodecError::FrameTooLarge { actual, maximum })
                if actual > MAX_FRAME_BYTES && maximum == MAX_FRAME_BYTES
        ));
    }

    #[test]
    fn control_attach_and_checkpoint_responses_fit_when_raw_tail_is_empty() {
        let checkpoint = TerminalCheckpoint {
            viewport: sample_viewport(),
            raw_replay_tail: Vec::new(),
            raw_tail_start_sequence: MAX_FRAME_BYTES as u64,
        };
        let attached = encode_message(FrameKind::Control, &attached_response(checkpoint.clone()))
            .expect("attach response must encode");
        assert!(attached.len() <= MAX_FRAME_BYTES);

        let requested = encode_message(
            FrameKind::Control,
            &ControlMessage::Response(HostResponse {
                request_id: 2,
                result: Ok(Response::TerminalCheckpoint(checkpoint)),
            }),
        )
        .expect("checkpoint response must encode");
        assert!(requested.len() <= MAX_FRAME_BYTES);
    }

    #[test]
    fn resync_required_failure_encodes_as_a_typed_control_error() {
        let encoded = encode_message(
            FrameKind::Control,
            &ControlMessage::Response(HostResponse {
                request_id: 3,
                result: Err(ProtocolFailure::new(
                    FailureCode::ResyncRequired,
                    "raw replay starts at 8388608",
                    true,
                )),
            }),
        )
        .expect("resync failure must encode");
        assert!(encoded.len() <= MAX_FRAME_BYTES);
        let decoded: ControlMessage = decode_message(&decode_frame(&encoded).unwrap()).unwrap();
        let ControlMessage::Response(HostResponse {
            result: Err(failure),
            ..
        }) = decoded
        else {
            panic!("expected failure response");
        };
        assert_eq!(failure.code, FailureCode::ResyncRequired);
        assert!(failure.retryable);
    }

    #[test]
    fn project_file_response_at_editor_limit_fits_the_control_frame() {
        let text = "a".repeat(6 * 1024 * 1024);
        let encoded = encode_message(
            FrameKind::Control,
            &ControlMessage::Response(HostResponse {
                request_id: 4,
                result: Ok(Response::Project(crate::project::ProjectResponse::File(
                    crate::project::ProjectFileContent {
                        relative_path: crate::ProjectRelativePath::from_utf8("a.txt").unwrap(),
                        text,
                        fingerprint: crate::project::ProjectFileFingerprint {
                            exists: true,
                            byte_len: 6 * 1024 * 1024,
                            modified_nanos: Some(1),
                            content_hash: 1,
                            revision: crate::project::ContentRevision::default(),
                        },
                    },
                ))),
            }),
        )
        .expect("6 MiB file response must encode");
        assert!(encoded.len() <= MAX_FRAME_BYTES);
    }

    #[test]
    fn project_file_response_above_frame_limit_is_rejected() {
        let text = "a".repeat(MAX_FRAME_BYTES);
        let encoded = encode_message(
            FrameKind::Control,
            &ControlMessage::Response(HostResponse {
                request_id: 5,
                result: Ok(Response::Project(crate::project::ProjectResponse::File(
                    crate::project::ProjectFileContent {
                        relative_path: crate::ProjectRelativePath::from_utf8("a.txt").unwrap(),
                        text,
                        fingerprint: crate::project::ProjectFileFingerprint {
                            exists: true,
                            byte_len: MAX_FRAME_BYTES as u64,
                            modified_nanos: Some(1),
                            content_hash: 1,
                            revision: crate::project::ContentRevision::default(),
                        },
                    },
                ))),
            }),
        );
        assert!(matches!(
            encoded,
            Err(ProtocolCodecError::FrameTooLarge { .. })
        ));
    }

    #[test]
    fn remote_file_response_at_editor_limit_fits_the_control_frame() {
        let encoded = encode_message(
            FrameKind::Control,
            &ControlMessage::Response(HostResponse {
                request_id: 6,
                result: Ok(Response::RemoteFile(crate::ssh::RemoteFileResponse::File(
                    crate::ssh::RemoteFileContent {
                        relative_path: "a.txt".to_string(),
                        bytes: vec![b'b'; 6 * 1024 * 1024],
                        fingerprint: crate::ssh::RemoteFileFingerprint {
                            byte_len: 6 * 1024 * 1024,
                            modified_seconds: Some(1),
                            content_hash: 1,
                            revision: crate::project::ContentRevision::default(),
                        },
                    },
                ))),
            }),
        )
        .expect("6 MiB remote file response must encode");
        assert!(encoded.len() <= MAX_FRAME_BYTES);
    }
}
