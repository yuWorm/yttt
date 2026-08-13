#![forbid(unsafe_code)]

pub mod agent;
pub mod codec;
pub mod control;
pub mod handshake;
pub mod project;
pub mod ssh;
pub mod terminal;

pub use codec::{
    DecodedFrame, FrameHeader, FrameKind, ProtocolCodecError, decode_frame, decode_header,
    decode_message, encode_frame, encode_message, read_frame, write_frame,
};
pub use control::{
    ClientRequest, ControlMessage, FailureCode, HostBlocker, HostEvent, HostResponse,
    ProtocolFailure, Request, ResourceCatalog, Response, ServerEvent, TerminalLease,
    TerminalPlacement, TerminalTerminationResult,
};
pub use handshake::{
    AuthMac, ClientAuthenticate, ClientHello, ConnectionChannel, HandshakeMessage, HostChallenge,
    HostReady, Nonce, ProtocolRange, RejectReason,
};

pub const PROTOCOL_MAGIC: [u8; 4] = *b"YTTT";
pub const PROTOCOL_VERSION: u16 = 3;
pub const HEADER_LEN: usize = 16;
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
pub const DEFAULT_COMPATIBILITY_WINDOW: u16 = 1;
