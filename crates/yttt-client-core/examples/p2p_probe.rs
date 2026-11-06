//! Pairs with a Host by code from the command line, then reports once a second
//! whether the session runs direct or through the relay and how long a request
//! takes. For testing P2P between machines without a desktop.
//!
//!     cargo run -p yttt-client-core --example p2p_probe -- '<pairing code>' [--relay-only] [--seconds 30]
//!         [--account <control url> <email>]
//!
//! With `--account` the probe signs in to a paid relay account (password from
//! `YTTT_RELAY_PASSWORD` or the terminal), registers its key and joins the
//! relay with a ticket, as the desktop app does; it removes the key from the
//! account when it ends.
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
    // RUST_LOG=iroh=debug shows how the connection finds its path.
    if std::env::var_os("RUST_LOG").is_some() {
        tracing_subscriber::fmt()
            .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
            .with_writer(std::io::stderr)
            .init();
    }
    let mut args = std::env::args().skip(1);
    let code = args
        .next()
        .ok_or("usage: p2p_probe '<pairing code>' [--relay-only] [--seconds N] [--account <control url> <email>]")?;
    let mut direct = DirectPaths::Any;
    let mut seconds = 30_u64;
    let mut account = None;
    while let Some(argument) = args.next() {
        match argument.as_str() {
            "--relay-only" => direct = DirectPaths::Off,
            "--seconds" => {
                seconds = args
                    .next()
                    .and_then(|value| value.parse().ok())
                    .ok_or("--seconds needs a number")?;
            }
            "--account" => {
                let (Some(control_url), Some(email)) = (args.next(), args.next()) else {
                    return Err("--account needs a control URL and an email".into());
                };
                account = Some((control_url, email));
            }
            other => return Err(format!("unknown argument {other}")),
        }
    }
    let code = PairingCode::decode(code.trim()).map_err(|error| error.to_string())?;

    let mut secret = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut secret);
    let session = match &account {
        Some((control_url, email)) => Some(sign_in(control_url, email).await?),
        None => None,
    };
    // The probe's key reaches only this Host, through the Host's node.
    let kind = yttt_control_client::DeviceKind::Client { host: code.host.0 };
    if let Some(session) = &session {
        session
            .register_device(&secret, "p2p probe", kind)
            .await
            .map_err(|error| error.to_string())?;
    }
    // Whatever happens next, the probe's key leaves the account at the end.
    let outcome = async {
        let relay = match &session {
            Some(session) => {
                let ticket = session
                    .ticket(&secret, kind)
                    .await
                    .map_err(|error| error.to_string())?;
                println!("ticket for {} at {}", ticket.node_id, ticket.relay_url);
                if code.relay_url.as_deref() != Some(ticket.relay_url.as_str()) {
                    println!(
                        "note: the Host invited through {:?}; this account's relay is {}",
                        code.relay_url, ticket.relay_url
                    );
                }
                Some(ticket.target())
            }
            None => code
                .relay_url
                .clone()
                .map(|url| RelayTarget { url, token: None }),
        };
        let endpoint = IrohEndpoint::bind(EndpointConfig {
            secret,
            relay: relay.into(),
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
                // The Host keeps a session per client ID for a while after it
                // ends; each run is a new session.
                client_instance_id: ClientInstanceId::new(format!(
                    "p2p-probe-{}",
                    rand::random::<u64>()
                )),
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
        Ok::<(), String>(())
    }
    .await;
    if let Some(session) = session {
        let key = yttt_relay_ticket::public_key(&secret);
        if let Err(error) = session.remove_device(&key).await {
            println!("could not remove the probe's key from the account: {error}");
        }
    }
    outcome
}

async fn sign_in(
    control_url: &str,
    email: &str,
) -> Result<yttt_control_client::ControlSession, String> {
    let password = match std::env::var("YTTT_RELAY_PASSWORD") {
        Ok(password) => password,
        Err(_) => {
            use std::io::Write as _;
            print!("password for {email}: ");
            std::io::stdout()
                .flush()
                .map_err(|error| error.to_string())?;
            let mut line = String::new();
            std::io::stdin()
                .read_line(&mut line)
                .map_err(|error| error.to_string())?;
            line.trim_end_matches(['\r', '\n']).to_owned()
        }
    };
    yttt_control_client::ControlClient::new(control_url)
        .map_err(|error| error.to_string())?
        .login(email, &password)
        .await
        .map_err(|error| error.to_string())
}
