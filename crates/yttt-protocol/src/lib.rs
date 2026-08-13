#![forbid(unsafe_code)]

pub mod agent;
pub mod codec;
pub mod control;
pub mod desktop;
pub mod handshake;
pub mod lifecycle;
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
pub use desktop::{
    DesktopShellMessage, DesktopShellRejectReason, DesktopShellRequest,
    DesktopShellRequestEnvelope, DesktopShellResponse, DesktopShellResponseEnvelope,
};
pub use handshake::{
    AuthMac, BuildIdentity, ClientAuthenticate, ClientHello, ConnectionChannel, HandshakeMessage,
    HostChallenge, HostReady, Nonce, ProtocolRange, RejectReason,
};
pub use lifecycle::{
    HostLifecycleState, HostLifecycleStatus, LifecycleMessage, LifecycleRequest,
    LifecycleRequestEnvelope, LifecycleResponse, LifecycleResponseEnvelope,
};

pub const PROTOCOL_MAGIC: [u8; 4] = *b"YTTT";
pub const FRAME_FORMAT_VERSION: u16 = 1;
pub const RESOURCE_PROTOCOL_VERSION: u16 = 1;
pub const LIFECYCLE_PROTOCOL_VERSION: u16 = 1;
pub const DESKTOP_SHELL_PROTOCOL_VERSION: u16 = 1;
pub const MAX_DESKTOP_SHELL_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_DESKTOP_OPEN_PATHS: usize = 64;
pub const HEADER_LEN: usize = 16;
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
