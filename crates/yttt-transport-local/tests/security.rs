use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};
use yttt_protocol::{PROTOCOL_VERSION, ProtocolRange};
use yttt_transport_local::{
    AuthToken, ClientIdentity, HandshakeError, HostIdentity, LocalEndpoint, LocalListener,
    TransportError, client_handshake, connect, server_handshake,
};

fn identities() -> (ClientIdentity, HostIdentity) {
    let profile_id = ProfileId::new("security-test");
    (
        ClientIdentity {
            supported: ProtocolRange::exact(PROTOCOL_VERSION),
            build_id: "test-build".to_string(),
            profile_id: profile_id.clone(),
            client_instance_id: ClientInstanceId::new("client-1"),
            host_epoch_hint: None,
        },
        HostIdentity {
            supported: ProtocolRange::exact(PROTOCOL_VERSION),
            build_id: "test-build".to_string(),
            profile_id,
            host_id: HostId::new("host-1"),
            host_epoch: 7,
            connection_sequence: 11,
        },
    )
}

#[cfg(unix)]
#[tokio::test]
async fn local_endpoint_is_user_only_and_single_instance() {
    use std::os::unix::fs::PermissionsExt as _;

    let temp = tempfile::tempdir().unwrap();
    let endpoint = LocalEndpoint::for_profile(ProfileId::new("test"), temp.path().join("runtime"));
    let listener = LocalListener::bind(endpoint.clone()).await.unwrap();

    let root_mode = std::fs::metadata(endpoint.runtime_root())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    let socket_mode = std::fs::metadata(endpoint.unix_path())
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(root_mode, 0o700);
    assert_eq!(socket_mode, 0o600);
    assert!(matches!(
        LocalListener::bind(endpoint.clone()).await,
        Err(TransportError::EndpointInUse)
    ));

    let (client, server) = tokio::join!(connect(&endpoint), listener.accept());
    drop(client.unwrap());
    drop(server.unwrap());
    drop(listener);
    assert!(!endpoint.unix_path().exists());
}

#[cfg(unix)]
#[tokio::test]
async fn matching_token_completes_mutually_authenticated_handshake() {
    let (client_identity, host_identity) = identities();
    let token = AuthToken::from_bytes([23; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut client_stream, &client_identity, &token),
        server_handshake(&mut server_stream, &host_identity, &token),
    );

    let host = client_result.unwrap();
    let client = server_result.unwrap();
    assert_eq!(host.host_id, host_identity.host_id);
    assert_eq!(host.host_epoch, 7);
    assert_eq!(host.connection_sequence, 11);
    assert_eq!(
        client.client_instance_id,
        client_identity.client_instance_id
    );
}

#[cfg(unix)]
#[tokio::test]
async fn invalid_client_proof_is_rejected_without_exposing_secret() {
    let (client_identity, host_identity) = identities();
    let host_token = AuthToken::from_bytes([2; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();
    let attacker = async {
        let hello = yttt_protocol::ClientHello {
            supported: client_identity.supported,
            build_id: client_identity.build_id.clone(),
            profile_id: client_identity.profile_id.clone(),
            client_instance_id: client_identity.client_instance_id.clone(),
            host_epoch_hint: None,
            nonce: yttt_protocol::Nonce([8; 32]),
        };
        yttt_transport_local::send_handshake(
            &mut client_stream,
            &yttt_protocol::HandshakeMessage::ClientHello(hello.clone()),
        )
        .await
        .unwrap();
        let challenge = match yttt_transport_local::receive_handshake(&mut client_stream)
            .await
            .unwrap()
        {
            yttt_protocol::HandshakeMessage::HostChallenge(challenge) => challenge,
            message => panic!("unexpected handshake message: {message:?}"),
        };
        yttt_transport_local::send_handshake(
            &mut client_stream,
            &yttt_protocol::HandshakeMessage::ClientAuthenticate(
                yttt_protocol::ClientAuthenticate {
                    client_instance_id: hello.client_instance_id,
                    host_epoch: challenge.host_epoch,
                    client_nonce: hello.nonce,
                    host_nonce: challenge.host_nonce,
                    proof: yttt_protocol::AuthMac([0; 32]),
                },
            ),
        )
        .await
        .unwrap();
        assert!(matches!(
            yttt_transport_local::receive_handshake(&mut client_stream)
                .await
                .unwrap(),
            yttt_protocol::HandshakeMessage::Rejected(
                yttt_protocol::RejectReason::AuthenticationFailed
            )
        ));
    };

    let (_, server_result) = tokio::join!(
        attacker,
        server_handshake(&mut server_stream, &host_identity, &host_token),
    );

    assert!(matches!(
        server_result,
        Err(HandshakeError::AuthenticationFailed)
    ));
    assert_eq!(format!("{host_token:?}"), "AuthToken([redacted])");
}

#[cfg(unix)]
#[tokio::test]
async fn build_and_profile_mismatches_fail_closed() {
    let (mut client_identity, host_identity) = identities();
    client_identity.build_id = "other-build".to_string();
    let token = AuthToken::from_bytes([5; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut client_stream, &client_identity, &token),
        server_handshake(&mut server_stream, &host_identity, &token),
    );

    assert!(matches!(client_result, Err(HandshakeError::Rejected(_))));
    assert!(matches!(
        server_result,
        Err(HandshakeError::IdentityMismatch)
    ));
}
