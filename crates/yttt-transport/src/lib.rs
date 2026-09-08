#![forbid(unsafe_code)]

mod auth;
mod error;
mod memory;
mod stream;
mod wire;

pub use auth::{
    AuthToken, AuthenticatedClient, AuthenticatedHost, AuthenticatedSession, ClientIdentity,
    HandshakeError, HostIdentity, IngressKind, client_handshake, new_session_nonce,
    server_handshake,
};
pub use error::TransportError;
pub use memory::{MemoryConnector, MemoryListener, memory_pair};
pub use stream::{
    BoxFuture, SharedConnector, TransportConnector, TransportIo, TransportListener, TransportStream,
};
pub use wire::{
    WireError, WireReceiveDiagnostics, receive_control, receive_control_observed,
    receive_desktop_shell, receive_handshake, receive_lifecycle, receive_state_event,
    receive_terminal_interactive, send_control, send_control_bounded, send_desktop_shell,
    send_handshake, send_lifecycle, send_state_event, send_terminal_interactive,
};
