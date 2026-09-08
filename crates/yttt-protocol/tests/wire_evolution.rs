use serde::{Deserialize, Serialize};
use yttt_core::model::ids::{ClientInstanceId, ProfileId};
use yttt_protocol::{
    BuildIdentity, ClientHello, ControlMessage, FrameKind, Nonce, ProtocolRange,
    RESOURCE_PROTOCOL_VERSION, Request, decode_frame, decode_message, encode_message,
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct TaggedV2 {
    request_id: u64,
    sent_millis: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct TaggedV3 {
    request_id: u64,
    sent_millis: u64,
    #[serde(default)]
    actor_device_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
enum TaggedCommand {
    Ping { sent_millis: u64 },
    ListResources,
}

#[derive(Serialize)]
struct SlimClientHello {
    supported: ProtocolRange,
    build: BuildIdentity,
    profile_id: ProfileId,
    client_instance_id: ClientInstanceId,
    host_epoch_hint: Option<u64>,
    nonce: Nonce,
}

fn encode_cbor<T: Serialize>(value: &T) -> Vec<u8> {
    let mut payload = Vec::new();
    ciborium::into_writer(value, &mut payload).unwrap();
    payload
}

fn decode_cbor<T: for<'de> Deserialize<'de>>(bytes: &[u8]) -> T {
    ciborium::from_reader(bytes).unwrap()
}

#[test]
fn tagged_encoding_ignores_new_fields_and_defaults_missing_ones() {
    let v2 = TaggedV2 {
        request_id: 9,
        sent_millis: 11,
    };
    let upgraded: TaggedV3 = decode_cbor(&encode_cbor(&v2));
    assert_eq!(
        upgraded,
        TaggedV3 {
            request_id: 9,
            sent_millis: 11,
            actor_device_id: None,
        }
    );

    let v3 = TaggedV3 {
        request_id: 9,
        sent_millis: 11,
        actor_device_id: Some("phone".to_string()),
    };
    let downgraded: TaggedV2 = decode_cbor(&encode_cbor(&v3));
    assert_eq!(downgraded, v2);
}

#[test]
fn unknown_tagged_enum_variants_are_rejected_instead_of_shifted() {
    #[derive(Serialize)]
    #[allow(dead_code)]
    enum FutureCommand {
        Ping { sent_millis: u64 },
        ListResources,
        RequestControl,
    }

    let encoded = encode_cbor(&FutureCommand::RequestControl);
    assert!(ciborium::from_reader::<TaggedCommand, _>(encoded.as_slice()).is_err());
}

#[test]
fn client_hello_without_session_binding_is_rejected() {
    let encoded = encode_cbor(&SlimClientHello {
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: BuildIdentity {
            product_version: "0.2.0".to_string(),
            build_fingerprint: "desktop".to_string(),
            resource_compatibility: "resource-v1".to_string(),
        },
        profile_id: ProfileId::new("profile"),
        client_instance_id: ClientInstanceId::new("client"),
        host_epoch_hint: None,
        nonce: Nonce([3; 32]),
    });
    assert!(ciborium::from_reader::<ClientHello, _>(encoded.as_slice()).is_err());
}

#[test]
fn adjacent_control_schema_versions_round_trip_through_the_frame_codec() {
    let current = ControlMessage::Request(yttt_protocol::ClientRequest::new(
        4,
        Request::Ping { sent_millis: 21 },
    ));
    let encoded = encode_message(FrameKind::Control, &current).unwrap();
    let decoded: ControlMessage = decode_message(&decode_frame(&encoded).unwrap()).unwrap();
    assert_eq!(decoded, current);
}
