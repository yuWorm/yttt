//! Pairs with a Host by code from the command line, then reports once a second
//! whether the session runs direct or through the relay and how long a request
//! takes. For testing P2P between machines without a desktop.
//!
//!     cargo run -p yttt-client-core --example p2p_probe -- '<pairing code>' [--relay-only] [--seconds 30]
//!
//! The device key lives only in this process: revoke the device on the Host
//! afterwards.

use std::{sync::Arc, time::Duration};

use rand::RngCore as _;
use yttt_client_core::{ClientCore, p2p::pair};
use yttt_core::model::ids::ClientInstanceId;
use yttt_protocol::{BuildIdentity, ConnectionChannel, ProtocolRange, Request, p2p::PairingCode};
use yttt_transport::ClientIdentity;
use yttt_transport_iroh::{
    DirectPaths, EndpointConfig, IrohEndpoint, OpenGate, PathKind, RelayTarget,
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let code = args
        .next()
        .ok_or("usage: p2p_probe '<pairing code>' [--relay-only] [--seconds N]")?;
    let mut direct = DirectPaths::Any;
    let mut seconds = 30_u64;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--relay-only" => direct = DirectPaths::Off,
            "--seconds" => {
                seconds = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .ok_or("--seconds needs a number")?;
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let code = PairingCode::decode(code.trim()).map_err(|error| error.to_string())?;

    let mut secret = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let endpoint = IrohEndpoint::bind(EndpointConfig {
        secret,
        relay: code
            .relay_url
            .clone()
            .map(|url| RelayTarget { url, token: None })
            .into(),
        alpns: Vec::new(),
        gate: Arc::new(OpenGate),
        direct,
    })
    .await
    .map_err(|error| error.to_string())?;
    println!("device key {}", endpoint.id());
    println!("reaching the Host…");
    let host = pair(&endpoint, &code, "p2p probe", |short_code| {
        let digits = format!("{short_code:06}");
        println!(
            "confirm on the Host that it shows: {} {}",
            &digits[..3],
            &digits[3..]
        );
    })
    .await
    .map_err(|error| error.to_string())?;
    println!("paired with {}", host.host);

    let connector = Arc::new(host.connector(endpoint.clone()));
    let started = std::time::Instant::now();
    let client = ClientCore::connect(
        yttt_transport::SharedConnector::new(connector.clone()),
        ClientIdentity {
            expected_environment: Some(host.environment_id.clone()),
            credential_generation: 0,
            session_nonce: yttt_transport::new_session_nonce(),
            supported: ProtocolRange::exact(yttt_protocol::RESOURCE_PROTOCOL_VERSION),
            build: BuildIdentity {
                product_version: env!("CARGO_PKG_VERSION").into(),
                build_fingerprint: "p2p-probe".into(),
                resource_compatibility: "yttt-resource-v1".into(),
            },
            profile_id: host.profile_id.clone(),
            client_instance_id: ClientInstanceId::new("p2p-probe"),
            host_epoch_hint: None,
            can_force_stop: false,
            channel: ConnectionChannel::Session,
        },
        host.credential(secret),
    )
    .await
    .map_err(|error| error.to_string())?;
    println!(
        "session open after {:.1} s",
        started.elapsed().as_secs_f64()
    );

    for second in 1..=seconds {
        let sent = std::time::Instant::now();
        let result = client.request(Request::ListResources).await;
        let path = match connector.path_kind().await {
            PathKind::Direct => "direct",
            PathKind::Relay => "relay",
            PathKind::Unknown => "unknown",
        };
        match result {
            Ok(_) => println!(
                "{second:>3}s  path {path:<7}  request {:>5.1} ms",
                sent.elapsed().as_secs_f64() * 1000.0
            ),
            Err(error) => println!("{second:>3}s  path {path:<7}  request failed: {error}"),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    endpoint.close().await;
    Ok(())
}
