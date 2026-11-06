//! Connects two clients to a relay and passes a datagram between them, to
//! check a relay works at an address, e.g. behind a reverse proxy.
//!
//!     cargo run -p yttt-relay --example relay_ping -- <relay url> [token]

use std::{sync::Arc, time::Duration};

use futures_util::{SinkExt as _, StreamExt as _};
use iroh_base::{RelayUrl, SecretKey};
use iroh_relay::{
    client::ClientBuilder,
    protos::relay::{ClientToRelayMsg, Datagrams, RelayToClientMsg},
};

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), String> {
    let mut args = std::env::args().skip(1);
    let url: RelayUrl = args
        .next()
        .ok_or("usage: relay_ping <relay url> [token]")?
        .parse()
        .map_err(|error| format!("{error}"))?;
    let token = args.next();
    let connect = |secret: [u8; 32]| {
        let url = url.clone();
        let token = token.clone();
        async move {
            let tls = iroh_relay::tls::CaTlsConfig::default()
                .client_config(Arc::new(rustls::crypto::ring::default_provider()))
                .map_err(|error| format!("{error}"))?;
            let mut builder = ClientBuilder::new(
                url,
                SecretKey::from_bytes(&secret),
                iroh_dns::dns::DnsResolver::new(),
            )
            .tls_client_config(tls);
            if let Some(token) = token {
                builder = builder.auth_token(token);
            }
            builder.connect().await.map_err(|error| format!("{error}"))
        }
    };
    let (alice, bob) = ([1_u8; 32], [2_u8; 32]);
    let mut sender = connect(alice).await?;
    let mut receiver = connect(bob).await?;
    sender
        .send(ClientToRelayMsg::Datagrams {
            dst_endpoint_id: SecretKey::from_bytes(&bob).public(),
            datagrams: Datagrams::from(b"ping".to_vec()),
        })
        .await
        .map_err(|error| format!("{error}"))?;
    let arrived = tokio::time::timeout(Duration::from_secs(10), async {
        while let Some(message) = receiver.next().await {
            if let Ok(RelayToClientMsg::Datagrams { datagrams, .. }) = message {
                return Some(datagrams.contents.to_vec());
            }
        }
        None
    })
    .await
    .map_err(|_| "no datagram within 10 s".to_string())?;
    match arrived {
        Some(bytes) if bytes == b"ping" => {
            println!("ok: a datagram crossed {url}");
            Ok(())
        }
        other => Err(format!("unexpected: {other:?}")),
    }
}
