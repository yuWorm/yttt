use std::{fmt, time::Duration};

use hmac::{Hmac, Mac};
use rand::{RngCore, rngs::OsRng};
use sha2::Sha256;
use tokio::io::{AsyncRead, AsyncWrite};
use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};
use yttt_protocol::{
    AuthMac, BuildIdentity, ClientAuthenticate, ClientHello, ConnectionChannel, HandshakeMessage,
    HostChallenge, HostReady, Nonce, ProtocolRange, RejectReason,
};
use zeroize::Zeroize;

use crate::wire::{WireError, receive_handshake, send_handshake};

type HmacSha256 = Hmac<Sha256>;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(5);

pub struct AuthToken([u8; 32]);

impl AuthToken {
    pub fn generate() -> Self {
        let mut bytes = [0_u8; 32];
        OsRng.fill_bytes(&mut bytes);
        Self(bytes)
    }

    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl Clone for AuthToken {
    fn clone(&self) -> Self {
        Self(self.0)
    }
}

impl fmt::Debug for AuthToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("AuthToken([redacted])")
    }
}

impl Drop for AuthToken {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClientIdentity {
    pub supported: ProtocolRange,
    pub build: BuildIdentity,
    pub profile_id: ProfileId,
    pub client_instance_id: ClientInstanceId,
    pub host_epoch_hint: Option<u64>,
    pub can_force_stop: bool,
    pub channel: ConnectionChannel,
    pub terminal_session_id: Option<yttt_core::model::ids::TerminalSessionId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HostIdentity {
    pub resource_supported: ProtocolRange,
    pub lifecycle_supported: ProtocolRange,
    pub build: BuildIdentity,
    pub profile_id: ProfileId,
    pub host_id: HostId,
    pub host_epoch: u64,
    pub connection_sequence: u64,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedClient {
    pub client_instance_id: ClientInstanceId,
    pub selected_version: u16,
    pub can_force_stop: bool,
    pub channel: ConnectionChannel,
    pub terminal_session_id: Option<yttt_core::model::ids::TerminalSessionId>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthenticatedHost {
    pub host_id: HostId,
    pub host_epoch: u64,
    pub selected_version: u16,
    pub connection_sequence: u64,
}

#[derive(Debug, thiserror::Error)]
pub enum HandshakeError {
    #[error(transparent)]
    Wire(#[from] WireError),
    #[error("handshake timed out")]
    Timeout,
    #[error("host rejected handshake: {0:?}")]
    Rejected(RejectReason),
    #[error("unexpected handshake message")]
    UnexpectedMessage,
    #[error("handshake identity mismatch")]
    IdentityMismatch,
    #[error("handshake authentication failed")]
    AuthenticationFailed,
}

pub async fn client_handshake<S>(
    stream: &mut S,
    identity: &ClientIdentity,
    token: &AuthToken,
) -> Result<AuthenticatedHost, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        client_handshake_inner(stream, identity, token),
    )
    .await
    .map_err(|_| HandshakeError::Timeout)?
}

async fn client_handshake_inner<S>(
    stream: &mut S,
    identity: &ClientIdentity,
    token: &AuthToken,
) -> Result<AuthenticatedHost, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let client_nonce = random_nonce();
    let hello = ClientHello {
        supported: identity.supported,
        build: identity.build.clone(),
        profile_id: identity.profile_id.clone(),
        client_instance_id: identity.client_instance_id.clone(),
        host_epoch_hint: identity.host_epoch_hint,
        can_force_stop: identity.can_force_stop,
        nonce: client_nonce,
        channel: identity.channel,
        terminal_session_id: identity.terminal_session_id.clone(),
    };
    send_handshake(stream, &HandshakeMessage::ClientHello(hello.clone())).await?;
    let challenge = match receive_handshake(stream).await? {
        HandshakeMessage::HostChallenge(challenge) => challenge,
        HandshakeMessage::Rejected(reason) => return Err(HandshakeError::Rejected(reason)),
        _ => return Err(HandshakeError::UnexpectedMessage),
    };
    if challenge.profile_id != identity.profile_id
        || identity
            .supported
            .negotiate(ProtocolRange::exact(challenge.selected_version))
            .is_none()
    {
        return Err(HandshakeError::IdentityMismatch);
    }
    verify_mac(
        token,
        &host_proof_bytes(&hello, &challenge),
        &challenge.proof,
    )?;
    let authenticate = ClientAuthenticate {
        client_instance_id: identity.client_instance_id.clone(),
        host_epoch: challenge.host_epoch,
        client_nonce,
        host_nonce: challenge.host_nonce,
        proof: sign(token, &client_proof_bytes(&hello, &challenge)),
    };
    send_handshake(stream, &HandshakeMessage::ClientAuthenticate(authenticate)).await?;
    let ready = match receive_handshake(stream).await? {
        HandshakeMessage::HostReady(ready) => ready,
        HandshakeMessage::Rejected(reason) => return Err(HandshakeError::Rejected(reason)),
        _ => return Err(HandshakeError::UnexpectedMessage),
    };
    if ready.host_id != challenge.host_id
        || ready.host_epoch != challenge.host_epoch
        || ready.selected_version != challenge.selected_version
    {
        return Err(HandshakeError::IdentityMismatch);
    }
    Ok(AuthenticatedHost {
        host_id: ready.host_id,
        host_epoch: ready.host_epoch,
        selected_version: ready.selected_version,
        connection_sequence: ready.connection_sequence,
    })
}

pub async fn server_handshake<S>(
    stream: &mut S,
    identity: &HostIdentity,
    token: &AuthToken,
) -> Result<AuthenticatedClient, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        server_handshake_inner(stream, identity, token),
    )
    .await
    .map_err(|_| HandshakeError::Timeout)?
}

async fn server_handshake_inner<S>(
    stream: &mut S,
    identity: &HostIdentity,
    token: &AuthToken,
) -> Result<AuthenticatedClient, HandshakeError>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let hello = match receive_handshake(stream).await? {
        HandshakeMessage::ClientHello(hello) => hello,
        _ => {
            reject(stream, RejectReason::InvalidMessage).await;
            return Err(HandshakeError::UnexpectedMessage);
        }
    };
    if !matches!(
        (
            hello.channel,
            hello.terminal_session_id.as_ref(),
            hello.can_force_stop
        ),
        (ConnectionChannel::Control, None, _)
            | (ConnectionChannel::TerminalData, Some(_), false)
            | (
                ConnectionChannel::Lifecycle | ConnectionChannel::DesktopOwner,
                None,
                _
            )
    ) {
        reject(stream, RejectReason::InvalidMessage).await;
        return Err(HandshakeError::Rejected(RejectReason::InvalidMessage));
    }
    let host_supported = match hello.channel {
        ConnectionChannel::Lifecycle | ConnectionChannel::DesktopOwner => {
            identity.lifecycle_supported
        }
        ConnectionChannel::Control | ConnectionChannel::TerminalData => identity.resource_supported,
    };
    let Some(selected_version) = host_supported.negotiate(hello.supported) else {
        reject(
            stream,
            RejectReason::VersionMismatch {
                supported: host_supported,
            },
        )
        .await;
        return Err(HandshakeError::Rejected(RejectReason::VersionMismatch {
            supported: host_supported,
        }));
    };
    if hello.profile_id != identity.profile_id {
        reject(stream, RejectReason::ProfileMismatch).await;
        return Err(HandshakeError::IdentityMismatch);
    }
    if let Some(host_epoch_hint) = hello.host_epoch_hint
        && host_epoch_hint > identity.host_epoch
    {
        reject(stream, RejectReason::StaleHostEpoch).await;
        return Err(HandshakeError::IdentityMismatch);
    }
    let mut challenge = HostChallenge {
        selected_version,
        build: identity.build.clone(),
        profile_id: identity.profile_id.clone(),
        host_id: identity.host_id.clone(),
        host_epoch: identity.host_epoch,
        client_nonce: hello.nonce,
        host_nonce: random_nonce(),
        proof: AuthMac([0; 32]),
    };
    challenge.proof = sign(token, &host_proof_bytes(&hello, &challenge));
    send_handshake(stream, &HandshakeMessage::HostChallenge(challenge.clone())).await?;
    let authenticate = match receive_handshake(stream).await? {
        HandshakeMessage::ClientAuthenticate(authenticate) => authenticate,
        _ => {
            reject(stream, RejectReason::InvalidMessage).await;
            return Err(HandshakeError::UnexpectedMessage);
        }
    };
    let identity_matches = authenticate.client_instance_id == hello.client_instance_id
        && authenticate.host_epoch == identity.host_epoch
        && authenticate.client_nonce == hello.nonce
        && authenticate.host_nonce == challenge.host_nonce;
    if !identity_matches
        || verify_mac(
            token,
            &client_proof_bytes(&hello, &challenge),
            &authenticate.proof,
        )
        .is_err()
    {
        reject(stream, RejectReason::AuthenticationFailed).await;
        return Err(HandshakeError::AuthenticationFailed);
    }
    send_handshake(
        stream,
        &HandshakeMessage::HostReady(HostReady {
            selected_version,
            host_id: identity.host_id.clone(),
            host_epoch: identity.host_epoch,
            connection_sequence: identity.connection_sequence,
        }),
    )
    .await?;
    Ok(AuthenticatedClient {
        client_instance_id: hello.client_instance_id,
        can_force_stop: hello.can_force_stop,
        selected_version,
        channel: hello.channel,
        terminal_session_id: hello.terminal_session_id,
    })
}

async fn reject(stream: &mut (impl AsyncWrite + Unpin), reason: RejectReason) {
    let _ = send_handshake(stream, &HandshakeMessage::Rejected(reason)).await;
}

fn random_nonce() -> Nonce {
    let mut bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut bytes);
    Nonce(bytes)
}

fn sign(token: &AuthToken, payload: &[u8]) -> AuthMac {
    let mut mac = HmacSha256::new_from_slice(token.bytes()).expect("HMAC accepts 32-byte keys");
    mac.update(payload);
    AuthMac(mac.finalize().into_bytes().into())
}

fn verify_mac(token: &AuthToken, payload: &[u8], proof: &AuthMac) -> Result<(), HandshakeError> {
    let mut mac = HmacSha256::new_from_slice(token.bytes()).expect("HMAC accepts 32-byte keys");
    mac.update(payload);
    mac.verify_slice(&proof.0)
        .map_err(|_| HandshakeError::AuthenticationFailed)
}

fn host_proof_bytes(hello: &ClientHello, challenge: &HostChallenge) -> Vec<u8> {
    transcript(b"yttt-host-proof-v1", hello, challenge)
}

fn client_proof_bytes(hello: &ClientHello, challenge: &HostChallenge) -> Vec<u8> {
    transcript(b"yttt-client-proof-v1", hello, challenge)
}

fn transcript(label: &[u8], hello: &ClientHello, challenge: &HostChallenge) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(256);
    push_field(&mut bytes, label);
    push_field(&mut bytes, hello.profile_id.as_str().as_bytes());
    push_field(&mut bytes, hello.client_instance_id.as_str().as_bytes());
    push_field(&mut bytes, challenge.host_id.as_str().as_bytes());
    bytes.extend_from_slice(&challenge.host_epoch.to_be_bytes());
    bytes.extend_from_slice(&challenge.selected_version.to_be_bytes());
    push_build_identity(&mut bytes, &hello.build);
    bytes.extend_from_slice(&hello.nonce.0);
    bytes.extend_from_slice(&challenge.host_nonce.0);
    bytes
}

fn push_build_identity(bytes: &mut Vec<u8>, build: &BuildIdentity) {
    push_field(bytes, build.product_version.as_bytes());
    push_field(bytes, build.build_fingerprint.as_bytes());
    push_field(bytes, build.resource_compatibility.as_bytes());
}

fn push_field(bytes: &mut Vec<u8>, field: &[u8]) {
    bytes.extend_from_slice(&(field.len() as u32).to_be_bytes());
    bytes.extend_from_slice(field);
}
