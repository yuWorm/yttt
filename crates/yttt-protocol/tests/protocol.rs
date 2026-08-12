use yttt_core::model::ids::{ClientInstanceId, HostId, ProfileId};
use yttt_protocol::{
    AuthMac, ClientHello, ControlMessage, FrameKind, HEADER_LEN, HandshakeMessage, HostChallenge,
    MAX_FRAME_BYTES, Nonce, PROTOCOL_MAGIC, PROTOCOL_VERSION, ProtocolCodecError, ProtocolRange,
    RejectReason, decode_frame, decode_message, encode_frame, encode_message,
};

fn client_hello() -> ClientHello {
    ClientHello {
        supported: ProtocolRange::exact(PROTOCOL_VERSION),
        build_id: "test-build".to_string(),
        profile_id: ProfileId::new("test"),
        client_instance_id: ClientInstanceId::new("client"),
        host_epoch_hint: Some(3),
        nonce: Nonce([7; 32]),
    }
}

#[test]
fn protocol_header_is_fixed_width_and_round_trips_typed_messages() {
    let message = HandshakeMessage::ClientHello(client_hello());
    let encoded = encode_message(FrameKind::Handshake, &message).unwrap();

    assert!(encoded.len() >= HEADER_LEN);
    assert_eq!(&encoded[..4], &PROTOCOL_MAGIC);
    let frame = decode_frame(&encoded).unwrap();
    assert_eq!(frame.header.version, PROTOCOL_VERSION);
    assert_eq!(frame.header.kind, FrameKind::Handshake);
    assert_eq!(decode_message::<HandshakeMessage>(&frame).unwrap(), message);
}

#[test]
fn protocol_rejects_oversized_truncated_and_checksum_corrupt_frames() {
    let oversized = vec![0_u8; MAX_FRAME_BYTES + 1];
    assert!(matches!(
        encode_frame(FrameKind::Control, &oversized),
        Err(ProtocolCodecError::FrameTooLarge { .. })
    ));

    let encoded = encode_frame(FrameKind::Control, b"payload").unwrap();
    assert!(matches!(
        decode_frame(&encoded[..encoded.len() - 1]),
        Err(ProtocolCodecError::LengthMismatch { .. })
    ));

    let mut corrupted = encoded;
    *corrupted.last_mut().unwrap() ^= 0xff;
    assert!(matches!(
        decode_frame(&corrupted),
        Err(ProtocolCodecError::ChecksumMismatch)
    ));
}

#[test]
fn protocol_rejects_unknown_kind_and_version_before_allocating_payload() {
    let mut unknown_kind = [0_u8; HEADER_LEN];
    unknown_kind[..4].copy_from_slice(&PROTOCOL_MAGIC);
    unknown_kind[4..6].copy_from_slice(&PROTOCOL_VERSION.to_be_bytes());
    unknown_kind[6..8].copy_from_slice(&999_u16.to_be_bytes());
    assert!(matches!(
        yttt_protocol::decode_header(&unknown_kind),
        Err(ProtocolCodecError::UnknownFrameKind(999))
    ));

    let mut future_version = unknown_kind;
    future_version[6..8].copy_from_slice(&(FrameKind::Control as u16).to_be_bytes());
    future_version[4..6].copy_from_slice(&(PROTOCOL_VERSION + 1).to_be_bytes());
    assert!(matches!(
        yttt_protocol::decode_header(&future_version),
        Err(ProtocolCodecError::VersionMismatch { .. })
    ));

    let mut declared_oversized = unknown_kind;
    declared_oversized[6..8].copy_from_slice(&(FrameKind::Control as u16).to_be_bytes());
    declared_oversized[8..12]
        .copy_from_slice(&u32::try_from(MAX_FRAME_BYTES + 1).unwrap().to_be_bytes());
    assert!(matches!(
        yttt_protocol::decode_header(&declared_oversized),
        Err(ProtocolCodecError::FrameTooLarge { .. })
    ));
}

#[test]
fn protocol_range_selects_newest_shared_version() {
    assert_eq!(
        ProtocolRange {
            minimum: 1,
            maximum: 4,
        }
        .negotiate(ProtocolRange {
            minimum: 2,
            maximum: 3,
        }),
        Some(3)
    );
    assert_eq!(
        ProtocolRange {
            minimum: 1,
            maximum: 2,
        }
        .negotiate(ProtocolRange {
            minimum: 3,
            maximum: 4,
        }),
        None
    );
}

#[test]
fn auth_proofs_are_redacted_from_debug_output() {
    let challenge = HostChallenge {
        selected_version: PROTOCOL_VERSION,
        build_id: "test-build".to_string(),
        profile_id: ProfileId::new("test"),
        host_id: HostId::new("host"),
        host_epoch: 9,
        client_nonce: Nonce([1; 32]),
        host_nonce: Nonce([2; 32]),
        proof: AuthMac([3; 32]),
    };

    let debug = format!("{challenge:?}");
    assert!(debug.contains("[redacted]"));
    assert!(!debug.contains("3, 3, 3"));
}

#[test]
fn every_top_level_variant_has_a_typed_binary_payload() {
    let messages = [
        HandshakeMessage::ClientHello(client_hello()),
        HandshakeMessage::Rejected(RejectReason::AuthenticationFailed),
    ];
    for message in messages {
        let frame = decode_frame(&encode_message(FrameKind::Handshake, &message).unwrap()).unwrap();
        let decoded: HandshakeMessage = decode_message(&frame).unwrap();
        assert_eq!(decoded, message);
    }

    let control = ControlMessage::Request(yttt_protocol::ClientRequest {
        request_id: 42,
        body: yttt_protocol::Request::Ping { sent_millis: 7 },
    });
    let frame = decode_frame(&encode_message(FrameKind::Control, &control).unwrap()).unwrap();
    assert_eq!(decode_message::<ControlMessage>(&frame).unwrap(), control);

    let control = ControlMessage::Event(yttt_protocol::HostEvent {
        host_sequence: 9,
        body: yttt_protocol::ServerEvent::AgentHook(yttt_protocol::agent::AgentHookEvent {
            scope: yttt_protocol::agent::AgentHookScope {
                project_id: "project".to_string(),
                tab_id: "tab".to_string(),
                pane_id: "pane".to_string(),
                generation: 3,
            },
            source: "codex".to_string(),
            event: "turn-start".to_string(),
            payload_json: br#"{"status":"working"}"#.to_vec(),
        }),
    });
    let frame = decode_frame(&encode_message(FrameKind::Control, &control).unwrap()).unwrap();
    assert_eq!(decode_message::<ControlMessage>(&frame).unwrap(), control);
}
