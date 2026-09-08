use yttt_core::model::ids::ProfileId;
#[cfg(unix)]
use yttt_core::model::ids::{ClientInstanceId, HostId};
#[cfg(unix)]
use yttt_protocol::{
    BuildIdentity, ConnectionChannel, LIFECYCLE_PROTOCOL_VERSION, ProtocolRange,
    RESOURCE_PROTOCOL_VERSION,
};
#[cfg(unix)]
use yttt_transport_local::{
    AuthToken, ClientIdentity, HandshakeError, HostIdentity, client_handshake, server_handshake,
};
use yttt_transport_local::{
    LocalEndpoint, LocalListener, TransportError, WireError, connect, send_control_bounded,
};

#[cfg(unix)]
fn build_identity(fingerprint: &str, compatibility: &str) -> BuildIdentity {
    BuildIdentity {
        product_version: "0.2.0".to_string(),
        build_fingerprint: fingerprint.to_string(),
        resource_compatibility: compatibility.to_string(),
    }
}
#[cfg(unix)]
fn identities() -> (ClientIdentity, HostIdentity) {
    let profile_id = ProfileId::new("security-test");
    (
        ClientIdentity {
            expected_environment: None,
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            build: build_identity("test-build", "resource-v1"),
            profile_id: profile_id.clone(),
            client_instance_id: ClientInstanceId::new("client-1"),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: ConnectionChannel::Control,
            terminal_session_id: None,
        },
        HostIdentity {
            resource_supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
            lifecycle_supported: ProtocolRange::exact(LIFECYCLE_PROTOCOL_VERSION),
            build: build_identity("test-build", "resource-v1"),
            profile_id,
            environment_id: "test-environment".to_string(),
            credential_generation: 0,
            ingress: yttt_transport::IngressKind::LocalAdmin,
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
#[cfg(windows)]
#[tokio::test]
async fn windows_named_pipe_is_single_instance_and_connects_same_user() {
    let temp = tempfile::tempdir().unwrap();
    let endpoint = LocalEndpoint::for_profile(
        ProfileId::new(format!("security-test-{}", std::process::id())),
        temp.path().join("runtime"),
    );
    let listener = LocalListener::bind(endpoint.clone()).await.unwrap();
    assert!(matches!(
        LocalListener::bind(endpoint.clone()).await,
        Err(TransportError::EndpointInUse)
    ));

    let (client, server) = tokio::join!(connect(&endpoint), listener.accept());
    drop(client.unwrap());
    drop(server.unwrap());
}

#[tokio::test]
async fn bounded_control_send_rejects_the_frame_before_writing() {
    use tokio::io::AsyncReadExt as _;

    let (mut sender, mut receiver) = tokio::io::duplex(1024);
    let message = yttt_protocol::ControlMessage::Request(yttt_protocol::ClientRequest::new(
        1,
        yttt_protocol::Request::Ping { sent_millis: 1 },
    ));
    assert!(matches!(
        send_control_bounded(&mut sender, &message, 1).await,
        Err(WireError::FrameTooLarge {
            encoded_bytes,
            max_bytes: 1,
        }) if encoded_bytes > 1
    ));
    let mut byte = [0_u8; 1];
    assert!(
        tokio::time::timeout(
            std::time::Duration::from_millis(20),
            receiver.read_exact(&mut byte),
        )
        .await
        .is_err()
    );
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
async fn lifecycle_channel_authenticates_across_a_build_mismatch() {
    let (mut client_identity, host_identity) = identities();
    client_identity.channel = ConnectionChannel::Lifecycle;
    client_identity.supported = ProtocolRange::exact(LIFECYCLE_PROTOCOL_VERSION);
    client_identity.build = build_identity("next-build", "resource-v2");
    let token = AuthToken::from_bytes([31; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();
    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut client_stream, &client_identity, &token),
        server_handshake(&mut server_stream, &host_identity, &token),
    );

    assert!(client_result.is_ok());
    let authenticated = server_result.unwrap();
    assert_eq!(authenticated.channel, ConnectionChannel::Lifecycle);
    assert!(!authenticated.can_force_stop);
}

#[cfg(unix)]
#[tokio::test]
async fn channel_and_terminal_session_identity_must_match() {
    let (control_identity, host_identity) = identities();
    let mut control_with_session = control_identity.clone();
    control_with_session.terminal_session_id =
        Some(yttt_core::model::ids::TerminalSessionId::new("unexpected"));
    let mut data_without_session = control_identity.clone();
    data_without_session.channel = ConnectionChannel::TerminalData;
    let mut lifecycle_with_session = control_identity.clone();
    lifecycle_with_session.channel = ConnectionChannel::Lifecycle;
    lifecycle_with_session.terminal_session_id =
        Some(yttt_core::model::ids::TerminalSessionId::new("unexpected"));
    for client_identity in [
        control_with_session,
        data_without_session,
        lifecycle_with_session,
    ] {
        let token = AuthToken::from_bytes([29; 32]);
        let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();
        let (client_result, server_result) = tokio::join!(
            client_handshake(&mut client_stream, &client_identity, &token),
            server_handshake(&mut server_stream, &host_identity, &token),
        );
        assert!(matches!(
            client_result,
            Err(HandshakeError::Rejected(
                yttt_protocol::RejectReason::InvalidMessage
            ))
        ));
        assert!(matches!(
            server_result,
            Err(HandshakeError::Rejected(
                yttt_protocol::RejectReason::InvalidMessage
            ))
        ));
    }
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
            build: client_identity.build.clone(),
            profile_id: client_identity.profile_id.clone(),
            client_instance_id: client_identity.client_instance_id.clone(),
            host_epoch_hint: None,
            session_nonce: client_identity.session_nonce,
            credential_generation: 0,
            can_force_stop: false,
            nonce: yttt_protocol::Nonce([8; 32]),
            channel: ConnectionChannel::Control,
            terminal_session_id: None,
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
async fn profile_mismatches_fail_closed() {
    let (mut client_identity, host_identity) = identities();
    client_identity.profile_id = ProfileId::new("other-profile");
    let token = AuthToken::from_bytes([5; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut client_stream, &client_identity, &token),
        server_handshake(&mut server_stream, &host_identity, &token),
    );

    assert!(matches!(
        client_result,
        Err(HandshakeError::Rejected(
            yttt_protocol::RejectReason::ProfileMismatch
        ))
    ));
    assert!(matches!(
        server_result,
        Err(HandshakeError::IdentityMismatch)
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn resource_compatibility_mismatch_is_allowed_when_protocol_ranges_overlap() {
    let (mut client_identity, host_identity) = identities();
    client_identity.build.resource_compatibility = "resource-v2".to_string();
    client_identity.build.build_fingerprint = "next-build".to_string();
    let token = AuthToken::from_bytes([7; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut client_stream, &client_identity, &token),
        server_handshake(&mut server_stream, &host_identity, &token),
    );

    assert!(client_result.is_ok());
    assert!(server_result.is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn overlapping_protocol_ranges_negotiate_the_newest_shared_version() {
    let (mut client_identity, mut host_identity) = identities();
    client_identity.supported = ProtocolRange {
        minimum: RESOURCE_PROTOCOL_VERSION,
        maximum: RESOURCE_PROTOCOL_VERSION + 1,
    };
    host_identity.resource_supported = ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION);
    let token = AuthToken::from_bytes([11; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut client_stream, &client_identity, &token),
        server_handshake(&mut server_stream, &host_identity, &token),
    );

    assert_eq!(
        client_result.unwrap().selected_version,
        RESOURCE_PROTOCOL_VERSION
    );
    assert_eq!(
        server_result.unwrap().selected_version,
        RESOURCE_PROTOCOL_VERSION
    );
}

#[cfg(unix)]
#[tokio::test]
async fn disjoint_protocol_ranges_are_rejected() {
    let (mut client_identity, host_identity) = identities();
    client_identity.supported = ProtocolRange {
        minimum: RESOURCE_PROTOCOL_VERSION + 1,
        maximum: RESOURCE_PROTOCOL_VERSION + 1,
    };
    let token = AuthToken::from_bytes([13; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut client_stream, &client_identity, &token),
        server_handshake(&mut server_stream, &host_identity, &token),
    );

    assert!(matches!(
        client_result,
        Err(HandshakeError::Rejected(
            yttt_protocol::RejectReason::VersionMismatch { .. }
        ))
    ));
    assert!(matches!(
        server_result,
        Err(HandshakeError::Rejected(
            yttt_protocol::RejectReason::VersionMismatch { .. }
        ))
    ));
}

#[cfg(unix)]
#[tokio::test]
async fn compatible_resource_builds_can_have_different_fingerprints() {
    let (mut client_identity, host_identity) = identities();
    client_identity.build.build_fingerprint = "next-build".to_string();
    let token = AuthToken::from_bytes([6; 32]);
    let (mut client_stream, mut server_stream) = tokio::net::UnixStream::pair().unwrap();

    let (client_result, server_result) = tokio::join!(
        client_handshake(&mut client_stream, &client_identity, &token),
        server_handshake(&mut server_stream, &host_identity, &token),
    );

    assert!(client_result.is_ok());
    assert!(server_result.is_ok());
}

#[cfg(unix)]
#[tokio::test]
async fn work_ingress_cannot_claim_device_administration() {
    for ingress in [
        yttt_transport::IngressKind::TlsWork,
        yttt_transport::IngressKind::SshWork,
    ] {
        for (channel, force) in [
            (ConnectionChannel::DesktopOwner, false),
            (ConnectionChannel::Lifecycle, false),
            (ConnectionChannel::Control, true),
        ] {
            let (mut client, mut host) = identities();
            host.ingress = ingress;
            client.channel = channel;
            client.can_force_stop = force;
            let token = AuthToken::generate();
            let (mut left, mut right) = tokio::io::duplex(4096);
            let (client_result, host_result) = tokio::join!(
                client_handshake(&mut left, &client, &token),
                server_handshake(&mut right, &host, &token),
            );
            assert!(matches!(
                client_result,
                Err(HandshakeError::Rejected(
                    yttt_protocol::RejectReason::PermissionDenied
                ))
            ));
            assert!(matches!(
                host_result,
                Err(HandshakeError::Rejected(
                    yttt_protocol::RejectReason::PermissionDenied
                ))
            ));
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn changing_negotiated_permissions_invalidates_the_host_proof() {
    let (client, host) = identities();
    let token = AuthToken::generate();
    let (mut client_stream, mut proxy_client) = tokio::io::duplex(4096);
    let (mut proxy_host, mut host_stream) = tokio::io::duplex(4096);
    let proxy = async move {
        let yttt_protocol::HandshakeMessage::ClientHello(mut hello) =
            yttt_transport::receive_handshake(&mut proxy_client)
                .await
                .unwrap()
        else {
            panic!("hello")
        };
        hello.can_force_stop = true;
        yttt_transport::send_handshake(
            &mut proxy_host,
            &yttt_protocol::HandshakeMessage::ClientHello(hello),
        )
        .await
        .unwrap();
        let challenge = yttt_transport::receive_handshake(&mut proxy_host)
            .await
            .unwrap();
        yttt_transport::send_handshake(&mut proxy_client, &challenge)
            .await
            .unwrap();
    };
    let client_run = async {
        let result = client_handshake(&mut client_stream, &client, &token).await;
        drop(client_stream);
        result
    };
    let host_run = async {
        let result = server_handshake(&mut host_stream, &host, &token).await;
        drop(host_stream);
        result
    };
    let (result, _, ()) = tokio::join!(client_run, host_run, proxy);
    assert!(matches!(result, Err(HandshakeError::AuthenticationFailed)));
}
