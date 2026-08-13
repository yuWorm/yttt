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
pub use wire::{
    WireError, WireReceiveDiagnostics, receive_control, receive_control_observed,
    receive_desktop_shell, receive_handshake, receive_lifecycle, send_control,
    send_control_bounded, send_desktop_shell, send_handshake, send_lifecycle,
};
