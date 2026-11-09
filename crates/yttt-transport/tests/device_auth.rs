use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ClientAuthenticate, ConnectionChannel, EndpointKey, HandshakeMessage,
    LIFECYCLE_PROTOCOL_VERSION, Nonce, ProtocolRange, RESOURCE_PROTOCOL_VERSION,
};
use yttt_transport::{
    AuthToken, ClientIdentity, Credential, DeviceKeyCredential, HandshakeError, HostIdentity,
    IngressKind, client_handshake_with, receive_handshake, send_handshake, server_handshake_with,
};

const HOST_SECRET: [u8; 32] = [1; 32];
const CLIENT_SECRET: [u8; 32] = [2; 32];
const OTHER_SECRET: [u8; 32] = [3; 32];

fn key(secret: [u8; 32]) -> EndpointKey {
    DeviceKeyCredential::new(secret).local()
}

fn build() -> BuildIdentity {
    BuildIdentity {
        product_version: "0.3.0".into(),
        build_fingerprint: "test".into(),
        resource_compatibility: "resource-v1".into(),
    }
}

fn client_identity() -> ClientIdentity {
    ClientIdentity {
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: build(),
        profile_id: ProfileId::new("p2p-test"),
        client_instance_id: ClientInstanceId::new("client-1"),
        host_epoch_hint: None,
        expected_environment: Some("environment".into()),
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        can_force_stop: false,
        channel: ConnectionChannel::Session,
    }
}

fn host_identity(ingress: IngressKind) -> HostIdentity {
    HostIdentity {
        resource_supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        lifecycle_supported: ProtocolRange::exact(LIFECYCLE_PROTOCOL_VERSION),
        build: build(),
        profile_id: ProfileId::new("p2p-test"),
        environment_id: "environment".into(),
        credential_generation: 0,
        ingress,
        host_id: HostId::new("host-1"),
        host_epoch: 4,
        connection_sequence: 1,
    }
}

/// The Host's credential once the transport has authenticated `peer`.
fn host_credential(peer: EndpointKey) -> Credential {
    Credential::DeviceKey(Box::new(
        DeviceKeyCredential::new(HOST_SECRET).with_peer(peer),
    ))
}

/// A Client that pinned `host` at pairing.
fn client_credential(secret: [u8; 32], host: EndpointKey) -> Credential {
    Credential::DeviceKey(Box::new(DeviceKeyCredential::new(secret).with_peer(host)))
}

type Outcome = (
    Result<yttt_transport::AuthenticatedClient, HandshakeError>,
    Result<yttt_transport::AuthenticatedHost, HandshakeError>,
);

/// Runs both sides; each drops its end as soon as it finishes, like a real peer.
async fn handshake(ingress: IngressKind, host: Credential, client: Credential) -> Outcome {
    let (mut client_stream, mut host_stream) = tokio::io::duplex(64 * 1024);
    let host_side = async move {
        server_handshake_with(&mut host_stream, &host_identity(ingress), &host).await
    };
    let client_side =
        async move { client_handshake_with(&mut client_stream, &client_identity(), &client).await };
    tokio::join!(host_side, client_side)
}

#[tokio::test]
async fn paired_device_and_host_prove_their_keys_to_each_other() {
    let (host_result, client_result) = handshake(
        IngressKind::P2pWork,
        host_credential(key(CLIENT_SECRET)),
        client_credential(CLIENT_SECRET, key(HOST_SECRET)),
    )
    .await;
    let authenticated = host_result.unwrap();
    assert_eq!(authenticated.session.ingress, IngressKind::P2pWork);
    assert_eq!(authenticated.session.peer, Some(key(CLIENT_SECRET)));
    assert_eq!(client_result.unwrap().host_epoch, 4);
}

#[tokio::test]
async fn a_client_refuses_a_host_other_than_the_pinned_one() {
    let impostor = Credential::DeviceKey(Box::new(
        DeviceKeyCredential::new(OTHER_SECRET).with_peer(key(CLIENT_SECRET)),
    ));
    let (host_result, client_result) = handshake(
        IngressKind::P2pWork,
        impostor,
        client_credential(CLIENT_SECRET, key(HOST_SECRET)),
    )
    .await;
    assert!(matches!(
        client_result,
        Err(HandshakeError::IdentityMismatch)
    ));
    // The Client dropped the stream without answering the challenge.
    assert!(host_result.is_err());
}

#[tokio::test]
async fn a_host_refuses_a_device_other_than_the_transport_peer() {
    let (host_result, client_result) = handshake(
        IngressKind::P2pWork,
        host_credential(key(OTHER_SECRET)),
        client_credential(CLIENT_SECRET, key(HOST_SECRET)),
    )
    .await;
    assert!(matches!(
        host_result,
        Err(HandshakeError::AuthenticationFailed)
    ));
    assert!(client_result.is_err());
}

#[tokio::test]
async fn a_client_signature_by_another_key_is_refused() {
    let (mut client, mut host) = tokio::io::duplex(64 * 1024);
    let identity = host_identity(IngressKind::P2pWork);
    let credential = host_credential(key(CLIENT_SECRET));
    let host_side = server_handshake_with(&mut host, &identity, &credential);
    let forger = async {
        // Claims the paired key but cannot sign with it.
        let mut hello = hello_for(key(CLIENT_SECRET));
        hello.nonce = Nonce([9; 32]);
        send_handshake(&mut client, &HandshakeMessage::ClientHello(hello.clone()))
            .await
            .unwrap();
        let HandshakeMessage::HostChallenge(challenge) =
            receive_handshake(&mut client).await.unwrap()
        else {
            panic!("expected a challenge");
        };
        let signature = sign_with(OTHER_SECRET, b"anything");
        send_handshake(
            &mut client,
            &HandshakeMessage::ClientAuthenticate(ClientAuthenticate {
                client_instance_id: hello.client_instance_id,
                host_epoch: challenge.host_epoch,
                client_nonce: hello.nonce,
                host_nonce: challenge.host_nonce,
                proof: yttt_protocol::AuthMac([0; 32]),
                signature: Some(signature),
            }),
        )
        .await
        .unwrap();
    };
    let (host_result, ()) = tokio::join!(host_side, forger);
    assert!(matches!(
        host_result,
        Err(HandshakeError::AuthenticationFailed)
    ));
}

#[tokio::test]
async fn a_tampered_challenge_fails_the_host_signature() {
    let (mut client, mut proxy_client) = tokio::io::duplex(64 * 1024);
    let (mut proxy_host, mut host) = tokio::io::duplex(64 * 1024);
    let host_side = async move {
        let identity = host_identity(IngressKind::P2pWork);
        server_handshake_with(&mut host, &identity, &host_credential(key(CLIENT_SECRET))).await
    };
    let client_side = async move {
        let credential = client_credential(CLIENT_SECRET, key(HOST_SECRET));
        client_handshake_with(&mut client, &client_identity(), &credential).await
    };
    let proxy = async move {
        let hello = receive_handshake(&mut proxy_client).await.unwrap();
        send_handshake(&mut proxy_host, &hello).await.unwrap();
        let HandshakeMessage::HostChallenge(mut challenge) =
            receive_handshake(&mut proxy_host).await.unwrap()
        else {
            panic!("expected a challenge");
        };
        challenge.host_epoch += 1;
        send_handshake(
            &mut proxy_client,
            &HandshakeMessage::HostChallenge(challenge),
        )
        .await
        .unwrap();
        drop(proxy_host);
    };
    let (_, client_result, ()) = tokio::join!(host_side, client_side, proxy);
    assert!(matches!(
        client_result,
        Err(HandshakeError::AuthenticationFailed)
    ));
}

#[tokio::test]
async fn each_ingress_takes_only_its_own_credential_kind() {
    let token = AuthToken::from_bytes([5; 32]);
    let cases = [
        // A shared secret on the P2P ingress.
        (
            IngressKind::P2pWork,
            Credential::SharedSecret(token.clone()),
            Credential::SharedSecret(token.clone()),
        ),
        // A device key on the TLS ingress.
        (
            IngressKind::TlsWork,
            host_credential(key(CLIENT_SECRET)),
            client_credential(CLIENT_SECRET, key(HOST_SECRET)),
        ),
        // A device-key Client against a TLS Host using the shared secret.
        (
            IngressKind::TlsWork,
            Credential::SharedSecret(token),
            client_credential(CLIENT_SECRET, key(HOST_SECRET)),
        ),
    ];
    for (ingress, host, client) in cases {
        let (host_result, client_result) = handshake(ingress, host, client).await;
        assert!(matches!(
            host_result,
            Err(HandshakeError::AuthenticationFailed)
        ));
        assert!(client_result.is_err());
    }
}

fn hello_for(endpoint_id: EndpointKey) -> yttt_protocol::ClientHello {
    let identity = client_identity();
    yttt_protocol::ClientHello {
        supported: identity.supported,
        build: identity.build,
        profile_id: identity.profile_id,
        client_instance_id: identity.client_instance_id,
        host_epoch_hint: None,
        session_nonce: identity.session_nonce,
        credential_generation: 0,
        can_force_stop: false,
        nonce: Nonce([0; 32]),
        channel: ConnectionChannel::Session,
        endpoint_id: Some(endpoint_id),
        accepts_compression: false,
        accepts_agent_board_patches: false,
    }
}

fn sign_with(secret: [u8; 32], payload: &[u8]) -> yttt_protocol::Signature {
    use ed25519_dalek::Signer as _;
    yttt_protocol::Signature(
        ed25519_dalek::SigningKey::from_bytes(&secret)
            .sign(payload)
            .to_bytes(),
    )
}
