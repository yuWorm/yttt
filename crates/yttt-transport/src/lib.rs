#![forbid(unsafe_code)]

mod auth;
mod error;
mod memory;
mod session;
mod stream;
mod wire;

pub use auth::{
    AuthToken, AuthenticatedClient, AuthenticatedHost, AuthenticatedSession, ClientIdentity,
    HandshakeError, HostIdentity, IngressKind, client_handshake, new_session_nonce,
    server_handshake,
};
pub use error::TransportError;
pub use memory::{MemoryConnector, MemoryListener, memory_pair};
pub use session::{
    CHUNK_PAYLOAD_BYTES, RENDER_LANE_BYTES, RenderLane, RenderObserver, RenderRejected,
    SessionReader, SessionSendError, SessionWriter, SessionWriterTask,
};
pub use stream::{
    BoxFuture, SharedConnector, TransportConnector, TransportIo, TransportListener, TransportStream,
};
pub use wire::{
    WireError, WireReceiveDiagnostics, receive_control, receive_desktop_shell, receive_handshake,
    receive_lifecycle, send_control, send_control_bounded, send_desktop_shell, send_frame,
    send_handshake, send_lifecycle,
};
