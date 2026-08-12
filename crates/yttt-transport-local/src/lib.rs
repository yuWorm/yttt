#![deny(unsafe_code)]

mod auth;
mod endpoint;
mod platform;
mod wire;

pub use auth::{
    AuthToken, AuthenticatedClient, AuthenticatedHost, ClientIdentity, HandshakeError,
    HostIdentity, client_handshake, server_handshake,
};
pub use endpoint::{EndpointAddress, LocalEndpoint};
pub use platform::{AsyncLocalStream, LocalListener, LocalStream, TransportError, connect};
pub use wire::{WireError, receive_control, receive_handshake, send_control, send_handshake};
