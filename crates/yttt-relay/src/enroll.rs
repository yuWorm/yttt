//! `yttt-relay enroll`: turns a one-time token from the console into this
//! machine's node configuration.

use std::{
    io::{Read as _, Write as _},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};

const USAGE: &str = "usage: yttt-relay enroll --control <url> --config-dir <dir> [--domain <name>] [--public-url <url>] [--port <port>]  (token on stdin or in YTTT_ENROLL)";
const DEFAULT_PORT: u16 = 3341;
/// Where certificates persist; systemd's StateDirectory for the service.
const ACME_CACHE: &str = "/var/lib/yttt-relay/acme";
const MAX_CLOCK_SKEW_SECS: u64 = 60;

#[derive(Serialize)]
struct Request<'a> {
    token: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    domain: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    public_url: Option<&'a str>,
    port: u16,
}

#[derive(Deserialize)]
struct Enrolled {
    node_id: String,
    node_token: String,
    url: String,
    ticket_keys: Vec<String>,
    report_interval_secs: u64,
    server_unix: u64,
}

struct Options {
    control: String,
    config_dir: PathBuf,
    domain: Option<String>,
    public_url: Option<String>,
    port: u16,
}

pub async fn run(args: impl Iterator<Item = String>) -> Result<(), String> {
    let options = parse(args)?;
    let token = token()?;
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|error| error.to_string())?;
    let response = client
        .post(format!("{}/v1/nodes/enroll", options.control))
        .json(&Request {
            token: &token,
            domain: options.domain.as_deref(),
            public_url: options.public_url.as_deref(),
            port: options.port,
        })
        .send()
        .await
        .map_err(|error| format!("cannot reach {}: {error}", options.control))?;
    if !response.status().is_success() {
        let status = response.status();
        let message = response.text().await.unwrap_or_default();
        return Err(format!("the control plane refused ({status}): {message}"));
    }
    let enrolled: Enrolled = response.json().await.map_err(|error| error.to_string())?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs());
    if now.abs_diff(enrolled.server_unix) > MAX_CLOCK_SKEW_SECS {
        eprintln!(
            "warning: this machine's clock is {} s off the control plane's; tickets may be refused until it is synchronized",
            now.abs_diff(enrolled.server_unix)
        );
    }
    write_private(&options.config_dir.join("node-token"), &enrolled.node_token)?;
    let config = config(&options, &enrolled);
    std::fs::write(options.config_dir.join("relay.toml"), config)
        .map_err(|error| error.to_string())?;
    println!("enrolled as node {} at {}", enrolled.node_id, enrolled.url);
    Ok(())
}

fn parse(mut args: impl Iterator<Item = String>) -> Result<Options, String> {
    let mut control = None;
    let mut config_dir = None;
    let mut domain = None;
    let mut public_url = None;
    let mut port = DEFAULT_PORT;
    while let Some(argument) = args.next() {
        let mut value = || args.next().ok_or_else(|| USAGE.to_string());
        match argument.as_str() {
            "--control" => control = Some(value()?.trim_end_matches('/').to_string()),
            "--config-dir" => config_dir = Some(PathBuf::from(value()?)),
            "--domain" => domain = Some(value()?),
            "--public-url" => public_url = Some(value()?),
            "--port" => port = value()?.parse().map_err(|_| USAGE.to_string())?,
            _ => return Err(USAGE.into()),
        }
    }
    Ok(Options {
        control: control.ok_or(USAGE)?,
        config_dir: config_dir.ok_or(USAGE)?,
        domain,
        public_url,
        port,
    })
}

fn token() -> Result<String, String> {
    if let Ok(token) = std::env::var("YTTT_ENROLL")
        && !token.trim().is_empty()
    {
        return Ok(token.trim().to_owned());
    }
    let mut token = String::new();
    std::io::stdin()
        .take(1024)
        .read_to_string(&mut token)
        .map_err(|error| error.to_string())?;
    let token = token.trim().to_owned();
    if token.is_empty() {
        return Err("no enrollment token on stdin or in YTTT_ENROLL".into());
    }
    Ok(token)
}

fn config(options: &Options, enrolled: &Enrolled) -> String {
    let keys = enrolled
        .ticket_keys
        .iter()
        .map(|key| format!("\"{key}\""))
        .collect::<Vec<_>>()
        .join(", ");
    let mut text = format!(
        "# Written by `yttt-relay enroll` for node {id}.\n# url = \"{url}\"\nnode_id = \"{id}\"\n",
        id = enrolled.node_id,
        url = enrolled.url,
    );
    match &options.domain {
        Some(domain) => text.push_str(&format!(
            "listen = \"0.0.0.0:443\"\ncaptive_portal_listen = \"0.0.0.0:80\"\nquic_listen = \"0.0.0.0:7842\"\nticket_keys = [{keys}]\n\n[acme]\ndomains = [\"{domain}\"]\ncache_dir = \"{ACME_CACHE}\"\n"
        )),
        None => text.push_str(&format!(
            "listen = \"0.0.0.0:{port}\"\nticket_keys = [{keys}]\n",
            port = options.port
        )),
    }
    text.push_str(&format!(
        "\n[control]\nurl = \"{control}\"\nnode_token_file = \"node-token\"\nreport_interval_secs = {interval}\n",
        control = options.control,
        interval = enrolled.report_interval_secs,
    ));
    text
}

fn write_private(path: &Path, value: &str) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let mut file = options
        .open(path)
        .map_err(|error| format!("{}: {error}", path.display()))?;
    file.write_all(value.as_bytes())
        .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enrolled() -> Enrolled {
        Enrolled {
            node_id: "tokyo-1".into(),
            node_token: "secret".into(),
            url: "https://relay.example.com".into(),
            ticket_keys: vec!["ab".repeat(32)],
            report_interval_secs: 15,
            server_unix: 0,
        }
    }

    #[test]
    fn an_enrolled_config_reads_back_and_keeps_the_token_out() {
        let temp = tempfile::tempdir().unwrap();
        for domain in [None, Some("relay.example.com".to_string())] {
            let options = Options {
                control: "https://control.example".into(),
                config_dir: temp.path().into(),
                domain: domain.clone(),
                public_url: None,
                port: 3341,
            };
            let text = config(&options, &enrolled());
            assert!(!text.contains("secret"));
            std::fs::write(temp.path().join("relay.toml"), &text).unwrap();
            write_private(&temp.path().join("node-token"), "secret").unwrap();
            let loaded = crate::NodeConfig::load(&temp.path().join("relay.toml")).unwrap();
            assert_eq!(loaded.node_id, "tokyo-1");
            assert_eq!(loaded.control.unwrap().node_token, "secret");
            assert_eq!(loaded.acme.is_some(), domain.is_some());
        }
    }
}
