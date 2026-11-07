//! Relay nodes managed from the console: create one, hand out a one-time
//! deploy command, let the machine enroll, and watch it come online.
//!
//! An enrollment token makes a machine a billing node of this service, and
//! nothing more: a node never holds anything that could reach a Host. The
//! worst a leaked token allows is a node that lies about usage on its own
//! accounts or refuses service, which an administrator ends by disabling it.

use std::{sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
};
use rand::RngCore as _;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};
use sqlx::Row as _;

use crate::{
    AppState, Failure,
    admin::{NodeView, audit, nodes},
    auth::{self, AdminSession, Attempt},
    clamp_i64, now_unix, placement, sha256,
};

type Shared = State<Arc<AppState>>;

const ENROLLMENT_TTL_SECS: u64 = 3600;
const DEFAULT_PORT: u16 = 3341;
const REPORT_INTERVAL_SECS: u64 = 15;
const PROBE_INTERVAL: Duration = Duration::from_secs(60);
const INSTALL_SCRIPT: &str = include_str!("../install/relay.sh");
/// The image a node installed with Docker runs: this release's.
const RELAY_IMAGE: &str = concat!("ghcr.io/yuworm/yttt-relay:", env!("CARGO_PKG_VERSION"));
/// The only files `/install/bin/` serves.
const BINARIES: [&str; 3] = [
    "yttt-relay-linux-x86_64",
    "yttt-relay-linux-aarch64",
    "SHA256SUMS",
];

#[derive(Deserialize)]
pub(crate) struct NewNode {
    id: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    domain: Option<String>,
}

#[derive(Deserialize, Default)]
pub(crate) struct Regenerate {
    #[serde(default)]
    domain: Option<String>,
}

#[derive(Serialize)]
pub(crate) struct DeployCommand {
    node: NodeView,
    command: String,
    expires_unix: u64,
}

#[derive(Deserialize)]
pub(crate) struct NodeChange {
    name: Option<String>,
    url: Option<String>,
    disabled: Option<bool>,
    region: Option<String>,
    /// Devices connected at once before the node takes no new Hosts; 0 for
    /// no limit.
    max_connections: Option<u32>,
}

#[derive(Deserialize)]
pub(crate) struct Enroll {
    token: String,
    #[serde(default)]
    domain: Option<String>,
    #[serde(default)]
    public_url: Option<String>,
    #[serde(default)]
    port: Option<u16>,
}

#[derive(Serialize)]
pub(crate) struct Enrolled {
    node_id: String,
    node_token: String,
    url: String,
    ticket_keys: Vec<String>,
    report_interval_secs: u64,
    /// So the node can warn about a clock far from the control plane's.
    server_unix: u64,
}

pub(crate) async fn create(
    State(state): Shared,
    admin: AdminSession,
    Json(request): Json<NewNode>,
) -> Result<(StatusCode, Json<DeployCommand>), Failure> {
    let id = request.id.trim().to_ascii_lowercase();
    if id.is_empty()
        || id.len() > 32
        || !id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(Failure::bad_request(
            "a node ID is 1 to 32 lowercase letters, digits and dashes",
        ));
    }
    let domain = valid_domain(request.domain)?;
    let base = deploy_base(&state)?;
    let name = request.name.trim().chars().take(64).collect::<String>();
    let mut tx = crate::db::begin(&state.pool, state.dialect).await?;
    let inserted = sqlx::query(
        "INSERT INTO nodes (id, url, token_hash, name, state, created_unix)
         VALUES ($1, '', NULL, $2, 'pending', $3)",
    )
    .bind(&id)
    .bind(if name.is_empty() { id.clone() } else { name })
    .bind(clamp_i64(now_unix()))
    .execute(&mut *tx)
    .await;
    match inserted {
        Ok(_) => {}
        Err(sqlx::Error::Database(error)) if error.is_unique_violation() => {
            return Err(Failure::conflict("a node with this ID exists"));
        }
        Err(error) => return Err(error.into()),
    }
    placement::open_node(&mut *tx, &id).await?;
    let (token, expires_unix) = new_enrollment(&mut tx, &id).await?;
    audit(
        &mut tx,
        admin.account_id,
        "create_node",
        None,
        Some(&id),
        "",
    )
    .await?;
    tx.commit().await?;
    let node = node_view(&state, &id).await?;
    Ok((
        StatusCode::CREATED,
        Json(DeployCommand {
            node,
            command: command(&base, &token, domain.as_deref()),
            expires_unix,
        }),
    ))
}

/// A fresh command for a node: the earlier unused ones stop working. The node
/// keeps its current token until a machine enrolls with the new one.
pub(crate) async fn regenerate(
    State(state): Shared,
    admin: AdminSession,
    Path(id): Path<String>,
    request: Option<Json<Regenerate>>,
) -> Result<Json<DeployCommand>, Failure> {
    let domain = valid_domain(
        request
            .map(|Json(request)| request)
            .unwrap_or_default()
            .domain,
    )?;
    let base = deploy_base(&state)?;
    let mut tx = crate::db::begin(&state.pool, state.dialect).await?;
    let exists: Option<String> = sqlx::query_scalar(&format!(
        "SELECT id FROM nodes WHERE id = $1{}",
        state.dialect.for_update()
    ))
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    if exists.is_none() {
        return Err(Failure::not_found());
    }
    sqlx::query("DELETE FROM node_enrollments WHERE node_id = $1 AND used_unix IS NULL")
        .bind(&id)
        .execute(&mut *tx)
        .await?;
    let (token, expires_unix) = new_enrollment(&mut tx, &id).await?;
    audit(
        &mut tx,
        admin.account_id,
        "regenerate_enrollment",
        None,
        Some(&id),
        "",
    )
    .await?;
    tx.commit().await?;
    Ok(Json(DeployCommand {
        node: node_view(&state, &id).await?,
        command: command(&base, &token, domain.as_deref()),
        expires_unix,
    }))
}

pub(crate) async fn change(
    State(state): Shared,
    admin: AdminSession,
    Path(id): Path<String>,
    Json(change): Json<NodeChange>,
) -> Result<StatusCode, Failure> {
    let mut tx = crate::db::begin(&state.pool, state.dialect).await?;
    let current: Option<String> = sqlx::query_scalar(&format!(
        "SELECT state FROM nodes WHERE id = $1{}",
        state.dialect.for_update()
    ))
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?;
    let current = current.ok_or_else(Failure::not_found)?;
    if let Some(name) = &change.name {
        let name: String = name.trim().chars().take(64).collect();
        sqlx::query("UPDATE nodes SET name = $2 WHERE id = $1")
            .bind(&id)
            .bind(&name)
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            admin.account_id,
            "rename_node",
            None,
            Some(&id),
            &name,
        )
        .await?;
    }
    if let Some(region) = &change.region {
        let region: String = region.trim().chars().take(64).collect();
        sqlx::query("UPDATE nodes SET region = $2 WHERE id = $1")
            .bind(&id)
            .bind(&region)
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            admin.account_id,
            "set_node_region",
            None,
            Some(&id),
            &region,
        )
        .await?;
    }
    if let Some(limit) = change.max_connections {
        let limit = (limit > 0).then(|| i32::try_from(limit).unwrap_or(i32::MAX));
        sqlx::query("UPDATE nodes SET max_connections = $2 WHERE id = $1")
            .bind(&id)
            .bind(limit)
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            admin.account_id,
            "set_node_limit",
            None,
            Some(&id),
            &limit.map_or_else(|| "none".to_owned(), |limit| limit.to_string()),
        )
        .await?;
    }
    if let Some(url) = &change.url {
        let url = url.trim().trim_end_matches('/');
        if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 256 {
            return Err(Failure::bad_request(
                "the node URL must be an http or https URL",
            ));
        }
        sqlx::query("UPDATE nodes SET url = $2, reachable = NULL WHERE id = $1")
            .bind(&id)
            .bind(url)
            .execute(&mut *tx)
            .await?;
        audit(
            &mut tx,
            admin.account_id,
            "set_node_url",
            None,
            Some(&id),
            url,
        )
        .await?;
    }
    if let Some(disabled) = change.disabled {
        if current == "pending" && !disabled {
            return Err(Failure::bad_request("the node has not enrolled yet"));
        }
        let state_name = if disabled { "disabled" } else { "active" };
        sqlx::query("UPDATE nodes SET state = $2 WHERE id = $1 AND state <> 'pending'")
            .bind(&id)
            .bind(state_name)
            .execute(&mut *tx)
            .await?;
        let action = if disabled {
            "disable_node"
        } else {
            "enable_node"
        };
        audit(&mut tx, admin.account_id, action, None, Some(&id), "").await?;
    }
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Moves every account on one node to another, e.g. before retiring it.
pub(crate) async fn move_accounts(
    State(state): Shared,
    admin: AdminSession,
    Path((from, to)): Path<(String, String)>,
) -> Result<Json<u64>, Failure> {
    let mut tx = crate::db::begin(&state.pool, state.dialect).await?;
    let target: Option<String> = sqlx::query_scalar("SELECT state FROM nodes WHERE id = $1")
        .bind(&to)
        .fetch_optional(&mut *tx)
        .await?;
    if target.as_deref() != Some("active") {
        return Err(Failure::bad_request(
            "move accounts to an enrolled, enabled node",
        ));
    }
    // Hosts, their Clients and the accounts that default here all move.
    let moved = placement::move_node(&mut tx, &from, &to).await?;
    audit(
        &mut tx,
        admin.account_id,
        "move_node_accounts",
        None,
        Some(&from),
        &format!("{moved} accounts to {to}"),
    )
    .await?;
    tx.commit().await?;
    Ok(Json(moved))
}

pub(crate) async fn remove(
    State(state): Shared,
    admin: AdminSession,
    Path(id): Path<String>,
) -> Result<StatusCode, Failure> {
    let mut tx = crate::db::begin(&state.pool, state.dialect).await?;
    let accounts: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM accounts WHERE node_id = $1)
              + (SELECT COUNT(*) FROM devices WHERE node_id = $1)",
    )
    .bind(&id)
    .fetch_one(&mut *tx)
    .await?;
    if accounts > 0 {
        return Err(Failure::conflict(
            "move the node's accounts elsewhere first",
        ));
    }
    let removed = sqlx::query("DELETE FROM nodes WHERE id = $1")
        .bind(&id)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if removed == 0 {
        return Err(Failure::not_found());
    }
    audit(
        &mut tx,
        admin.account_id,
        "delete_node",
        None,
        Some(&id),
        "",
    )
    .await?;
    tx.commit().await?;
    Ok(StatusCode::NO_CONTENT)
}

pub(crate) async fn list(
    State(state): Shared,
    _admin: AdminSession,
) -> Result<Json<Vec<NodeView>>, Failure> {
    Ok(Json(nodes(&state).await?))
}

/// A machine turns an enrollment token into node credentials. The token is
/// spent in one statement, so of two machines racing with it one wins.
pub(crate) async fn enroll(
    State(state): Shared,
    _attempt: Attempt,
    headers: HeaderMap,
    peer: axum::extract::ConnectInfo<std::net::SocketAddr>,
    Json(request): Json<Enroll>,
) -> Result<Json<Enrolled>, Failure> {
    let domain = valid_domain(request.domain)?;
    let node_id: Option<String> = sqlx::query_scalar(
        "UPDATE node_enrollments SET used_unix = $2
         WHERE token_hash = $1 AND used_unix IS NULL AND expires_unix > $2
         RETURNING node_id",
    )
    .bind(sha256(request.token.trim().as_bytes()))
    .bind(clamp_i64(now_unix()))
    .fetch_optional(&state.pool)
    .await?;
    let node_id = node_id.ok_or(Failure::forbidden(
        "the enrollment token is unknown, used or expired; generate a new deploy command",
    ))?;
    let url = match (request.public_url, domain) {
        (Some(url), _) => {
            let url = url.trim().trim_end_matches('/').to_string();
            if !(url.starts_with("https://") || url.starts_with("http://")) || url.len() > 256 {
                return Err(Failure::bad_request(
                    "the public URL must be an http or https URL",
                ));
            }
            url
        }
        (None, Some(domain)) => format!("https://{domain}"),
        (None, None) => {
            let ip = auth::client_ip(&headers, Some(peer.0), state.config.trusted_proxy);
            let port = request.port.unwrap_or(DEFAULT_PORT);
            match ip {
                std::net::IpAddr::V6(ip) => format!("http://[{ip}]:{port}"),
                ip => format!("http://{ip}:{port}"),
            }
        }
    };
    let mut token = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut token);
    let node_token = data_encoding::BASE64URL_NOPAD.encode(&token);
    sqlx::query(
        "UPDATE nodes SET token_hash = $2, url = $3, enrolled_unix = $4, reachable = NULL,
             state = CASE WHEN state = 'pending' THEN 'active' ELSE state END
         WHERE id = $1",
    )
    .bind(&node_id)
    .bind(sha256(node_token.as_bytes()))
    .bind(&url)
    .bind(clamp_i64(now_unix()))
    .execute(&state.pool)
    .await?;
    Ok(Json(Enrolled {
        node_id,
        node_token,
        url,
        ticket_keys: vec![yttt_relay_ticket::hex(&state.signer.public_key())],
        report_interval_secs: REPORT_INTERVAL_SECS,
        server_unix: now_unix(),
    }))
}

/// The installer, with this control plane's address filled in.
pub(crate) async fn install_script(State(state): Shared) -> Result<Response, Failure> {
    let base = deploy_base(&state)?;
    Ok((
        [(header::CONTENT_TYPE, "text/x-shellscript; charset=utf-8")],
        script(&base),
    )
        .into_response())
}

pub(crate) async fn install_binary(
    State(state): Shared,
    Path(file): Path<String>,
) -> Result<Response, Failure> {
    let directory = state
        .config
        .relay_binaries_dir
        .clone()
        .ok_or_else(Failure::not_found)?;
    if !BINARIES.contains(&file.as_str()) {
        return Err(Failure::not_found());
    }
    let bytes = tokio::fs::read(directory.join(&file))
        .await
        .map_err(|_| Failure::not_found())?;
    let content_type = if file == "SHA256SUMS" {
        "text/plain; charset=utf-8"
    } else {
        "application/octet-stream"
    };
    Ok(([(header::CONTENT_TYPE, content_type)], bytes).into_response())
}

/// Probes every enrolled node's `/healthz` so the console can flag a node
/// that reports but cannot be reached at its address.
pub(crate) async fn probe_loop(state: Arc<AppState>) {
    let Ok(client) = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .build()
    else {
        return;
    };
    let mut tick = tokio::time::interval(PROBE_INTERVAL);
    loop {
        tick.tick().await;
        let Ok(rows) =
            sqlx::query("SELECT id, url FROM nodes WHERE state = 'active' AND url <> ''")
                .fetch_all(&state.pool)
                .await
        else {
            continue;
        };
        for row in rows {
            let id: String = row.get("id");
            let url: String = row.get("url");
            let reachable = client
                .get(format!("{}/healthz", url.trim_end_matches('/')))
                .send()
                .await
                .is_ok_and(|response| response.status().is_success());
            let _ = sqlx::query("UPDATE nodes SET reachable = $2 WHERE id = $1")
                .bind(&id)
                .bind(reachable)
                .execute(&state.pool)
                .await;
        }
    }
}

async fn new_enrollment(
    tx: &mut crate::db::Tx<'_>,
    node_id: &str,
) -> Result<(String, u64), Failure> {
    let mut token = [0_u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut token);
    let token = data_encoding::BASE64URL_NOPAD.encode(&token);
    let now = now_unix();
    let expires_unix = now + ENROLLMENT_TTL_SECS;
    sqlx::query(
        "INSERT INTO node_enrollments (token_hash, node_id, created_unix, expires_unix)
         VALUES ($1, $2, $3, $4)",
    )
    .bind(sha256(token.as_bytes()))
    .bind(node_id)
    .bind(clamp_i64(now))
    .bind(clamp_i64(expires_unix))
    .execute(&mut **tx)
    .await?;
    Ok((token, expires_unix))
}

async fn node_view(state: &AppState, id: &str) -> Result<NodeView, Failure> {
    nodes(state)
        .await?
        .into_iter()
        .find(|node| node.id == id)
        .ok_or_else(Failure::not_found)
}

/// Where deploy commands download from. A command that fetches a root
/// installer over plain HTTP could be rewritten in transit, so outside
/// development the control plane must be reached over HTTPS.
fn deploy_base(state: &AppState) -> Result<String, Failure> {
    let url = state
        .config
        .public_url
        .as_deref()
        .ok_or(Failure::unavailable(
            "set public_url to generate deploy commands",
        ))?;
    if !url.starts_with("https://") && !state.config.dev_mode {
        return Err(Failure::unavailable(
            "deploy commands need public_url to be https outside dev_mode",
        ));
    }
    Ok(url.trim_end_matches('/').to_string())
}

fn valid_domain(domain: Option<String>) -> Result<Option<String>, Failure> {
    let Some(domain) = domain.map(|domain| domain.trim().to_ascii_lowercase()) else {
        return Ok(None);
    };
    if domain.is_empty() {
        return Ok(None);
    }
    let valid = domain.len() <= 253
        && domain.contains('.')
        && domain.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && !label.starts_with('-')
                && !label.ends_with('-')
                && label
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        });
    valid
        .then_some(Some(domain))
        .ok_or(Failure::bad_request("the domain is not a valid host name"))
}

pub(crate) fn script(base: &str) -> String {
    INSTALL_SCRIPT
        .replace("__CONTROL_URL__", base)
        .replace("__IMAGE__", RELAY_IMAGE)
        .replace("__VERSION__", env!("CARGO_PKG_VERSION"))
}

/// The command an administrator pastes on the new machine. It checks the
/// installer against the hash it was generated with before running it as
/// root, and hands the token over in the environment, not as an argument.
fn command(base: &str, token: &str, domain: Option<&str>) -> String {
    let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(script(base).as_bytes()));
    let domain = domain.map_or(String::new(), |domain| format!(" --domain {domain}"));
    format!(
        "curl -fsSL {base}/install/relay.sh -o /tmp/yttt-relay-install.sh && \
         echo '{digest}  /tmp/yttt-relay-install.sh' | sha256sum -c - && \
         sudo YTTT_ENROLL='{token}' sh /tmp/yttt-relay-install.sh{domain}"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deploy_command_pins_the_installer_and_names_the_domain() {
        let command = command(
            "https://control.example",
            "TOKEN",
            Some("relay.example.com"),
        );
        let digest = data_encoding::HEXLOWER.encode(&Sha256::digest(
            script("https://control.example").as_bytes(),
        ));
        assert!(command.contains(&digest));
        assert!(command.contains("YTTT_ENROLL='TOKEN'"));
        assert!(command.ends_with("--domain relay.example.com"));
        assert!(script("https://control.example").contains("CONTROL='https://control.example'"));
    }

    #[test]
    fn domains_are_host_names() {
        assert_eq!(
            valid_domain(Some(" Relay.Example.com ".into())).unwrap(),
            Some("relay.example.com".into())
        );
        assert_eq!(valid_domain(Some(String::new())).unwrap(), None);
        for bad in [
            "localhost",
            "-a.example",
            "a..example",
            "a b.example",
            "a.example;rm",
        ] {
            assert!(valid_domain(Some(bad.into())).is_err(), "{bad}");
        }
    }
}
