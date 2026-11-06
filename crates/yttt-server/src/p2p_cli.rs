//! `yttt-server p2p …`: P2P remote access for a Host with no screen.
//!
//! Every command goes to the running Host over its local administrator
//! endpoint, the same way the desktop settings page does, so only someone
//! logged in on this machine can pair or revoke devices.

use std::{ffi::OsString, time::Duration};

use serde::Serialize;
use yttt_client_core::ClientCore;
use yttt_core::model::ids::ClientInstanceId;
use yttt_protocol::{
    ConnectionChannel, EndpointKey, ProtocolRange, RESOURCE_PROTOCOL_VERSION, Request, Response,
    p2p::{DevicePreset, PairedDevice},
    remote_access::{
        P2pStatus, PairingState, RelayAccount, RemoteAccessRequest, RemoteAccessResponse,
        RemoteAccessStatus, SecretText,
    },
};
use yttt_transport_local::{ClientIdentity, LocalConnector};

use crate::{ServerError, ServerPaths, build_identity, read_token, write_json};

const POLL: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum P2pCommand {
    Status,
    Enable {
        relay_url: Option<String>,
    },
    Disable,
    Pair,
    Devices,
    Revoke {
        endpoint_id: EndpointKey,
        reset_tls: bool,
    },
    /// Joins the relay of a paid relay account: by default with a short code
    /// confirmed on the console from any browser; with `--password <email>`,
    /// the password comes from `YTTT_RELAY_PASSWORD` or the terminal.
    Login {
        control_url: String,
        password_email: Option<String>,
    },
    Logout,
}

pub(crate) const P2P_USAGE: &str = "usage: yttt-server p2p <status|enable [--relay <url>]|disable|pair|devices|revoke <device key> [--reset-tls]|login <control url> [--password <email>]|logout> [--profile <id>] [--state-root <absolute path>]";

/// Parses what follows `p2p`, leaving `--profile` and `--state-root` for the caller.
pub(crate) fn parse(args: &mut Vec<OsString>) -> Result<P2pCommand, ServerError> {
    let usage = || ServerError::InvalidArgument(P2P_USAGE.to_string());
    if args.is_empty() {
        return Err(usage());
    }
    let command = args.remove(0).into_string().map_err(|_| usage())?;
    let mut take_flag = |flag: &str| -> bool {
        match args.iter().position(|argument| argument == flag) {
            Some(index) => {
                args.remove(index);
                true
            }
            None => false,
        }
    };
    Ok(match command.as_str() {
        "status" => P2pCommand::Status,
        "disable" => P2pCommand::Disable,
        "pair" => P2pCommand::Pair,
        "devices" => P2pCommand::Devices,
        "logout" => P2pCommand::Logout,
        "login" => {
            let password_email = match args.iter().position(|argument| argument == "--password") {
                Some(index) => {
                    args.remove(index);
                    if index >= args.len() {
                        return Err(usage());
                    }
                    Some(args.remove(index).into_string().map_err(|_| usage())?)
                }
                None => None,
            };
            if args.is_empty() {
                return Err(usage());
            }
            let control_url = args.remove(0).into_string().map_err(|_| usage())?;
            if !(control_url.starts_with("http://") || control_url.starts_with("https://")) {
                return Err(ServerError::InvalidArgument(
                    "the control plane must be an http:// or https:// URL".into(),
                ));
            }
            P2pCommand::Login {
                control_url,
                password_email,
            }
        }
        "enable" => {
            let relay_url = match args.iter().position(|argument| argument == "--relay") {
                Some(index) => {
                    args.remove(index);
                    if index >= args.len() {
                        return Err(usage());
                    }
                    let url = args.remove(index).into_string().map_err(|_| usage())?;
                    if !(url.starts_with("http://") || url.starts_with("https://")) {
                        return Err(ServerError::InvalidArgument(
                            "the relay must be an http:// or https:// URL".into(),
                        ));
                    }
                    Some(url)
                }
                None => None,
            };
            P2pCommand::Enable { relay_url }
        }
        "revoke" => {
            let reset_tls = take_flag("--reset-tls");
            if args.is_empty() {
                return Err(usage());
            }
            let key = args.remove(0).into_string().map_err(|_| usage())?;
            let endpoint_id = key.parse().map_err(|_| {
                ServerError::InvalidArgument("the device key must be 64 hex digits".into())
            })?;
            P2pCommand::Revoke {
                endpoint_id,
                reset_tls,
            }
        }
        _ => return Err(usage()),
    })
}

pub(crate) async fn run(paths: ServerPaths, command: P2pCommand) -> Result<(), ServerError> {
    paths.prepare_for_client()?;
    let token = read_token(&paths.token_file)?;
    let identity = ClientIdentity {
        expected_environment: None,
        credential_generation: 0,
        session_nonce: yttt_transport::new_session_nonce(),
        supported: ProtocolRange::exact(RESOURCE_PROTOCOL_VERSION),
        build: build_identity(),
        profile_id: paths.profile.clone(),
        client_instance_id: ClientInstanceId::new(format!(
            "yttt-server-p2p-{}",
            std::process::id()
        )),
        host_epoch_hint: None,
        can_force_stop: false,
        channel: ConnectionChannel::Session,
    };
    let admin = ClientCore::connect(
        LocalConnector::new(paths.endpoint()),
        identity,
        token.auth.clone(),
    )
    .await
    .map_err(|error| match error {
        yttt_client_core::ClientCoreError::Connection(_) => ServerError::NotRunning,
        error => ServerError::InvalidArgument(error.to_string()),
    })?;
    let admin = Admin(admin);
    match command {
        P2pCommand::Status => write_json(&Summary::of(&admin.status().await?)),
        P2pCommand::Enable { relay_url } => {
            admin
                .send(RemoteAccessRequest::SetP2p {
                    enabled: true,
                    relay_url,
                })
                .await?;
            write_json(&Summary::of(&admin.status().await?))
        }
        P2pCommand::Disable => {
            let relay_url = admin.status().await?.settings.p2p.relay_url;
            admin
                .send(RemoteAccessRequest::SetP2p {
                    enabled: false,
                    relay_url,
                })
                .await?;
            write_json(&Summary::of(&admin.status().await?))
        }
        P2pCommand::Devices => write_json(&listed(admin.devices().await?)),
        P2pCommand::Revoke {
            endpoint_id,
            reset_tls,
        } => {
            admin
                .send(RemoteAccessRequest::RevokeDevice {
                    endpoint_id,
                    reset_tls,
                })
                .await?;
            write_json(&listed(admin.devices().await?))
        }
        P2pCommand::Pair => pair(&admin).await,
        P2pCommand::Login {
            control_url,
            password_email,
        } => {
            let refused = |error: yttt_control_client::ControlError| {
                ServerError::InvalidArgument(error.to_string())
            };
            let client = yttt_control_client::ControlClient::new(&control_url).map_err(refused)?;
            let session = match password_email {
                Some(email) => {
                    let password = match std::env::var("YTTT_RELAY_PASSWORD") {
                        Ok(password) => password,
                        Err(_) => prompt("password: ")
                            .await?
                            .trim_end_matches(['\r', '\n'])
                            .to_owned(),
                    };
                    client.login(&email, &password).await.map_err(refused)?
                }
                None => {
                    let login = client
                        .begin_device_login(&format!("yttt-server on {}", host_name()))
                        .await
                        .map_err(refused)?;
                    // Instructions on stderr; stdout keeps the JSON summary.
                    eprintln!("{}", qr_text(&login.verification_uri_complete));
                    eprintln!(
                        "To sign in, open {} on any device and enter:\n\n    {}\n\n\
                         or scan the QR code. Waiting for the confirmation (10 minutes)…",
                        login.verification_uri, login.user_code
                    );
                    login.finish().await.map_err(refused)?
                }
            };
            admin
                .send(RemoteAccessRequest::SetRelayAccount {
                    account: Some(RelayAccount {
                        control_url: session.control_url.clone(),
                        account_id: session.account_id,
                        session: SecretText(session.token.clone()),
                    }),
                })
                .await?;
            write_json(&Summary::of(&admin.status().await?))
        }
        P2pCommand::Logout => {
            admin
                .send(RemoteAccessRequest::SetRelayAccount { account: None })
                .await?;
            write_json(&Summary::of(&admin.status().await?))
        }
    }
}

struct Admin(ClientCore);

impl Admin {
    async fn send(
        &self,
        request: RemoteAccessRequest,
    ) -> Result<RemoteAccessResponse, ServerError> {
        match self.0.request(Request::RemoteAccess(request)).await {
            Ok(Response::RemoteAccess(response)) => Ok(response),
            Ok(_) => Err(ServerError::UnexpectedLifecycleResponse),
            Err(error) => Err(ServerError::InvalidArgument(error.to_string())),
        }
    }

    async fn status(&self) -> Result<RemoteAccessStatus, ServerError> {
        match self.send(RemoteAccessRequest::Status).await? {
            RemoteAccessResponse::Status(status) => Ok(status),
            _ => Err(ServerError::UnexpectedLifecycleResponse),
        }
    }

    async fn devices(&self) -> Result<Vec<PairedDevice>, ServerError> {
        match self.send(RemoteAccessRequest::ListDevices).await? {
            RemoteAccessResponse::Devices(devices) => Ok(devices),
            _ => Err(ServerError::UnexpectedLifecycleResponse),
        }
    }
}

/// A paired device as the CLI prints it: the key in the hex `revoke` takes.
#[derive(Serialize)]
struct ListedDevice {
    endpoint_id: String,
    label: String,
    preset: &'static str,
    paired_at_millis: u64,
    last_seen_millis: Option<u64>,
}

fn listed(devices: Vec<PairedDevice>) -> Vec<ListedDevice> {
    devices
        .into_iter()
        .map(|device| ListedDevice {
            endpoint_id: device.endpoint_id.to_string(),
            label: device.label,
            preset: match device.preset {
                DevicePreset::Full => "full",
                DevicePreset::Observe => "observe",
            },
            paired_at_millis: device.paired_at_millis,
            last_seen_millis: device.last_seen_millis,
        })
        .collect()
}

#[derive(Serialize)]
struct Summary {
    effective: String,
    endpoint_id: Option<String>,
    relay_url: Option<String>,
    relay_account: Option<String>,
    error: Option<String>,
    /// Remote clients connected now, by their device key or client ID.
    clients: Vec<String>,
}

impl Summary {
    fn of(status: &RemoteAccessStatus) -> Self {
        let P2pStatus {
            effective,
            endpoint_id,
            error,
            relay_account,
            ..
        } = &status.p2p;
        Self {
            effective: format!("{effective:?}").to_lowercase(),
            endpoint_id: endpoint_id.map(|key| key.to_string()),
            relay_url: relay_account
                .as_ref()
                .and_then(|account| account.relay_url.clone())
                .or_else(|| status.settings.p2p.relay_url.clone()),
            relay_account: relay_account
                .as_ref()
                .map(|account| format!("{} at {}", account.account_id, account.control_url)),
            error: error.clone(),
            clients: status
                .clients
                .iter()
                .map(|client| match client.device {
                    Some(device) => device.to_string(),
                    None => client.client_id.as_str().to_owned(),
                })
                .collect(),
        }
    }
}

/// Opens an invitation, shows its code, and lets the person at this terminal
/// compare the device's numbers and decide.
async fn pair(admin: &Admin) -> Result<(), ServerError> {
    let RemoteAccessResponse::PairingInvitation { code, .. } =
        admin.send(RemoteAccessRequest::StartPairing).await?
    else {
        return Err(ServerError::UnexpectedLifecycleResponse);
    };
    println!("{}", qr_text(&code.0));
    println!(
        "Pairing code (single use, valid for 5 minutes):\n\n{}\n",
        code.0
    );
    println!("Paste it on the device: Remote connections → Add → Pair with a Host (P2P).");
    println!("Waiting for a device… (Ctrl-C cancels)");
    drop(code);

    let outcome = tokio::select! {
        outcome = wait_and_decide(admin) => outcome,
        _ = tokio::signal::ctrl_c() => {
            println!("\nCancelled.");
            Err(ServerError::InvalidArgument("pairing was cancelled".into()))
        }
    };
    if outcome.is_err() {
        let _ = admin.send(RemoteAccessRequest::CancelPairing).await;
    }
    outcome
}

async fn wait_and_decide(admin: &Admin) -> Result<(), ServerError> {
    loop {
        match admin.status().await?.p2p.pairing {
            Some(PairingState::Open { .. }) => tokio::time::sleep(POLL).await,
            Some(PairingState::AwaitingConfirmation {
                endpoint_id,
                device_label,
                short_code,
                ..
            }) => {
                let digits = format!("{short_code:06}");
                println!(
                    "\n“{device_label}” wants to pair. Accept only if the device shows:\n\n    {} {}\n",
                    &digits[..3],
                    &digits[3..]
                );
                let answer = prompt(
                    "[v] allow viewing only, [f] allow full control, anything else declines: ",
                )
                .await?;
                let preset = match answer.trim() {
                    "v" | "V" => DevicePreset::Observe,
                    "f" | "F" => DevicePreset::Full,
                    _ => {
                        admin.send(RemoteAccessRequest::CancelPairing).await?;
                        println!("Declined.");
                        return Ok(());
                    }
                };
                admin
                    .send(RemoteAccessRequest::ConfirmPairing {
                        endpoint_id,
                        preset,
                    })
                    .await?;
                println!("Paired. Device key: {endpoint_id}");
                return Ok(());
            }
            Some(PairingState::Compromised) => {
                return Err(ServerError::InvalidArgument(
                    "a second device used this code, so the invitation was withdrawn; the code may have leaked".into(),
                ));
            }
            None => {
                return Err(ServerError::InvalidArgument(
                    "the invitation ended before a device asked to pair".into(),
                ));
            }
        }
    }
}

async fn prompt(question: &str) -> Result<String, ServerError> {
    use std::io::Write as _;
    print!("{question}");
    std::io::stdout().flush()?;
    tokio::task::spawn_blocking(|| {
        let mut line = String::new();
        std::io::stdin().read_line(&mut line).map(|_| line)
    })
    .await
    .map_err(|error| ServerError::InvalidArgument(error.to_string()))?
    .map_err(ServerError::Io)
}

/// The code as a QR code drawn with half-block characters, for a phone camera.
/// This machine's name, for the console to show what is signing in.
fn host_name() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .or_else(|| std::env::var("HOSTNAME").ok())
        .or_else(|| std::env::var("COMPUTERNAME").ok())
        .map(|name| name.trim().to_owned())
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "this machine".into())
}

fn qr_text(code: &str) -> String {
    use qrcode::render::unicode::Dense1x2;
    match qrcode::QrCode::with_error_correction_level(code.as_bytes(), qrcode::EcLevel::M) {
        Ok(qr) => qr
            .render::<Dense1x2>()
            .dark_color(Dense1x2::Light)
            .light_color(Dense1x2::Dark)
            .quiet_zone(true)
            .build(),
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(args: &[&str]) -> Result<(P2pCommand, Vec<OsString>), ServerError> {
        let mut args = args.iter().map(OsString::from).collect();
        parse(&mut args).map(|command| (command, args))
    }

    #[test]
    fn p2p_commands_parse_and_leave_shared_options() {
        assert_eq!(
            parsed(&[
                "enable",
                "--relay",
                "https://relay.example",
                "--profile",
                "work"
            ])
            .unwrap(),
            (
                P2pCommand::Enable {
                    relay_url: Some("https://relay.example".into())
                },
                vec![OsString::from("--profile"), OsString::from("work")]
            )
        );
        let key = "07".repeat(32);
        assert_eq!(
            parsed(&["revoke", "--reset-tls", &key]).unwrap().0,
            P2pCommand::Revoke {
                endpoint_id: EndpointKey([7; 32]),
                reset_tls: true
            }
        );
        assert_eq!(parsed(&["pair"]).unwrap().0, P2pCommand::Pair);
        assert_eq!(
            parsed(&["login", "https://control.example"]).unwrap().0,
            P2pCommand::Login {
                control_url: "https://control.example".into(),
                password_email: None
            }
        );
        assert_eq!(
            parsed(&[
                "login",
                "https://control.example",
                "--password",
                "me@example.com"
            ])
            .unwrap()
            .0,
            P2pCommand::Login {
                control_url: "https://control.example".into(),
                password_email: Some("me@example.com".into())
            }
        );
        for bad in [
            &["enable", "--relay"][..],
            &["login"],
            &["login", "control.example"],
            &["login", "https://control.example", "--password"],
            &["enable", "--relay", "relay.example"],
            &["revoke", "abc"],
            &["revoke"],
            &["open"],
            &[],
        ] {
            assert!(parsed(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_terminal_qr_code_is_drawn() {
        let text = qr_text("YTTT-PAIR:1:ABCDEFGHIJKLMNOP");
        assert!(text.lines().count() > 10);
    }
}
