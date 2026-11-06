use std::process::ExitCode;

use yttt_control::{Control, ControlConfig};

const USAGE: &str = "usage: yttt-control --config <path.toml>\n       yttt-control public-key <ticket_secret hex>";

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
        // Prints the key relay nodes list in `ticket_keys`.
        (Some("public-key"), Some(secret)) => {
            let Some(secret) = yttt_relay_ticket::parse_key(&secret) else {
                eprintln!("the ticket secret is 32 bytes of hex");
                return ExitCode::from(2);
            };
            println!(
                "{}",
                yttt_relay_ticket::hex(&yttt_relay_ticket::public_key(&secret))
            );
            return ExitCode::SUCCESS;
        }
        _ => {
            eprintln!("{USAGE}");
            return ExitCode::from(2);
        }
    };
    let config = match std::fs::read_to_string(&path)
        .map_err(|error| error.to_string())
        .and_then(|text| ControlConfig::from_toml(&text).map_err(|error| error.to_string()))
    {
        Ok(config) => config,
        Err(error) => {
            eprintln!("{path}: {error}");
            return ExitCode::FAILURE;
        }
    };
    let control = match Control::spawn(config).await {
        Ok(control) => control,
        Err(error) => {
            eprintln!("yttt-control: {error}");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        addr = %control.addr(),
        ticket_key = %yttt_relay_ticket::hex(&control.ticket_key()),
        "control plane serving"
    );
    let _ = tokio::signal::ctrl_c().await;
    control.shutdown().await;
    ExitCode::SUCCESS
}
