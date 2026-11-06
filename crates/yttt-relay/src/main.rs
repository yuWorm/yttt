use std::process::ExitCode;

use yttt_relay::{NodeConfig, RelayNode};

const USAGE: &str = "usage: yttt-relay --config <path.toml>";

#[tokio::main]
async fn main() -> ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let mut args = std::env::args().skip(1);
    let path = match (args.next().as_deref(), args.next()) {
        (Some("--config"), Some(path)) => path,
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let config = match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|text| NodeConfig::from_toml(&text).map_err(|error| error.to_string()))
    {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let node = match RelayNode::spawn(config).await {
        Ok(node) => node,
        Err(error) => {
            eprintln!("yttt-relay: {error}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(addr = %node.addr(), quic = ?node.quic_addr(), "relay node serving");
    let _ = tokio::signal::ctrl_c().await;
    node.shutdown().await;
    ExitCode::SUCCESS
}
