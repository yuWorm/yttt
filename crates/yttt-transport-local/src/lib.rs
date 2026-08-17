#![deny(unsafe_code)]

mod endpoint;
mod platform;

pub use endpoint::{EndpointAddress, LocalEndpoint};
pub use platform::{AsyncLocalStream, LocalListener, LocalStream, TransportError, connect};
pub use yttt_transport::{
    AuthToken, AuthenticatedClient, AuthenticatedHost, ClientIdentity, HandshakeError,
    HostIdentity, WireError, WireReceiveDiagnostics, client_handshake, receive_control,
    receive_control_observed, receive_desktop_shell, receive_handshake, receive_lifecycle,
    send_control, send_control_bounded, send_desktop_shell, send_handshake, send_lifecycle,
    server_handshake,
};

#[derive(Clone, Debug)]
pub struct LocalConnector {
    endpoint: LocalEndpoint,
}

impl LocalConnector {
    pub fn new(endpoint: LocalEndpoint) -> Self {
        Self { endpoint }
    }

    pub fn endpoint(&self) -> &LocalEndpoint {
        &self.endpoint
    }
}

impl From<LocalEndpoint> for LocalConnector {
    fn from(endpoint: LocalEndpoint) -> Self {
        Self::new(endpoint)
    }
}

impl yttt_transport::TransportListener for LocalListener {
    fn accept(
        &self,
    ) -> yttt_transport::BoxFuture<
        '_,
        Result<yttt_transport::TransportStream, yttt_transport::TransportError>,
    > {
        Box::pin(async { LocalListener::accept(self).await.map_err(Into::into) })
    }
}

impl yttt_transport::TransportConnector for LocalConnector {
    fn connect(
        &self,
    ) -> yttt_transport::BoxFuture<
        '_,
        Result<yttt_transport::TransportStream, yttt_transport::TransportError>,
    > {
        let endpoint = self.endpoint.clone();
        Box::pin(async move { connect(&endpoint).await.map_err(Into::into) })
    }
}

impl From<TransportError> for yttt_transport::TransportError {
    fn from(error: TransportError) -> Self {
        match error {
            TransportError::Io(error) => Self::Io(error),
            other => Self::Other(other.to_string()),
        }
    }
}
