#![forbid(unsafe_code)]

pub mod agent;
pub mod codec;
pub mod control;
pub mod desktop;
pub mod handshake;
pub mod lifecycle;
pub mod path;
pub mod project;
pub mod ssh;
pub mod terminal;

pub use codec::{
    DecodedFrame, FrameHeader, FrameKind, ProtocolCodecError, decode_frame, decode_header,
    decode_message, encode_frame, encode_message, read_frame, write_frame,
};
pub use control::{
    Capability, ClientRequest, ControlMessage, FailureCode, HostBlocker, HostEvent, HostResponse,
    ProtocolFailure, Request, ResourceCatalog, Response, ServerEvent, TerminalControlDeniedReason,
    TerminalLease, TerminalPlacement, TerminalTerminationResult,
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
pub use path::{HostPath, PathSegment, ProjectPathError, ProjectRelativePath};
pub use project::ContentRevision;
pub use project::{
    GitAccess, GitOperationError, ProjectGitOperation, git_argv_is_safe, is_safe_git_ref_name,
};

pub const PROTOCOL_MAGIC: [u8; 4] = *b"YTTT";
pub const FRAME_FORMAT_VERSION: u16 = 1;
pub const RESOURCE_PROTOCOL_VERSION: u16 = 3;
pub const LIFECYCLE_PROTOCOL_VERSION: u16 = 2;
pub const DESKTOP_SHELL_PROTOCOL_VERSION: u16 = 2;
pub const MAX_DESKTOP_SHELL_FRAME_BYTES: usize = 256 * 1024;
pub const MAX_DESKTOP_OPEN_PATHS: usize = 64;
pub const HEADER_LEN: usize = 16;
pub const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;
