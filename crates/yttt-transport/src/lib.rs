#![forbid(unsafe_code)]

mod auth;
mod error;
mod memory;
mod stream;
mod wire;

pub use auth::{
    AuthToken, AuthenticatedClient, AuthenticatedHost, ClientIdentity, HandshakeError,
    HostIdentity, client_handshake, server_handshake,
};
pub use error::TransportError;
pub use memory::{MemoryConnector, MemoryListener, memory_pair};
pub use stream::{
    BoxFuture, SharedConnector, TransportConnector, TransportIo, TransportListener, TransportStream,
};
pub use wire::{
    WireError, WireReceiveDiagnostics, receive_control, receive_control_observed,
    receive_desktop_shell, receive_handshake, receive_lifecycle, send_control,
    send_control_bounded, send_desktop_shell, send_handshake, send_lifecycle,
};
